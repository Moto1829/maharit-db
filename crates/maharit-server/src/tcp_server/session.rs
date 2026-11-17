//! セッションの確認、ロールによる権限判定、ACL の適用と、
//! ユーザー管理・ACL の文（CREATE USER
//!  GRANT など）の処理。

use super::*;

/// 認証ロールをワイヤ表現の文字列にする。
pub(super) fn role_label(role: &crate::auth::Role) -> String {
    match role {
        crate::auth::Role::Admin => "admin".to_string(),
        crate::auth::Role::ReadWrite => "read_write".to_string(),
        crate::auth::Role::ReadOnly => "read_only".to_string(),
    }
}

/// `require_auth = true` のとき、リクエストの `sessionToken` を検証し、
/// 認証済みセッションのロールを返す。
///
/// - 認証が無効なら検証せず `Ok(Role::Admin)`（後段の権限チェックも実質無効化）
/// - トークン無し → `Err(AuthError レスポンス)`
/// - トークンが無効/期限切れ → `Err(AuthError レスポンス)`
/// - 有効 → `Ok(role)`
pub(super) fn check_session(
    require_auth: bool,
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    token: &Option<String>,
) -> Result<crate::auth::Role, Response> {
    check_session_user(require_auth, auth, token).map(|(role, _)| role)
}

/// [`check_session`] と同じだが、監査ログ用にユーザー名も返す
/// （認証が無効なら `"anonymous"`）。
pub(super) fn check_session_user(
    require_auth: bool,
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    token: &Option<String>,
) -> Result<(crate::auth::Role, String), Response> {
    if !require_auth {
        return Ok((crate::auth::Role::Admin, "anonymous".to_string()));
    }
    let token_str = match token {
        Some(t) if !t.is_empty() => t.as_str(),
        _ => {
            return Err(Response::AuthError {
                message: "authentication required: missing sessionToken".to_string(),
            });
        }
    };
    let mut mgr = auth.lock().unwrap();
    match mgr.validate_session(token_str) {
        Ok(session) => Ok((session.role, session.username.clone())),
        Err(e) => Err(Response::AuthError {
            message: format!("invalid session: {}", e),
        }),
    }
}

/// 認証済みロールが対象クエリを実行できるかを RBAC で検証する。
///
/// 書き込みを伴うクエリ（CREATE/SET/DELETE/MERGE/REMOVE/制約作成など）を
/// `ReadOnly` ロールが実行しようとした場合に拒否レスポンスを返す。
/// パースに失敗するクエリは権限判定をスキップし、実行側でパースエラーを返させる。
pub(super) fn authorize_query(role: crate::auth::Role, query: &str) -> Option<Response> {
    let stmt = match Parser::new(query) {
        Ok(mut parser) => match parser.parse() {
            Ok(s) => s,
            Err(_) => return None,
        },
        Err(_) => return None,
    };

    let operation = if is_read_only(&stmt) {
        crate::auth::Operation::Read
    } else {
        crate::auth::Operation::Write
    };

    match crate::auth::AuthManager::check_role_permission(role, operation) {
        Ok(()) => None,
        Err(e) => Some(Response::AuthError {
            message: format!("permission denied: {}", e),
        }),
    }
}

/// ロール文字列を [`crate::auth::Role`] に対応付ける。
/// `reader`/`read_only`、`writer`/`read_write`、`admin` を受理（大文字小文字無視）。
pub(super) fn parse_role_str(s: &str) -> Option<crate::auth::Role> {
    match s.to_ascii_lowercase().as_str() {
        "reader" | "read_only" | "readonly" => Some(crate::auth::Role::ReadOnly),
        "writer" | "read_write" | "readwrite" => Some(crate::auth::Role::ReadWrite),
        "admin" => Some(crate::auth::Role::Admin),
        _ => None,
    }
}

/// 監査ログが有効なら `event` を記録する。
pub(super) fn audit_log(audit: Option<&crate::audit::AuditLogger>, event: crate::audit::AuthEvent) {
    if let Some(a) = audit {
        a.log_auth_event(event);
    }
}

/// 権限エラーを監査ログに記録する。
pub(super) fn audit_denied(
    audit: Option<&crate::audit::AuditLogger>,
    username: &str,
    action: &str,
) {
    audit_log(
        audit,
        crate::audit::AuthEvent::PermissionDenied {
            username: username.to_string(),
            action: action.to_string(),
        },
    );
}

/// セッションに適用する ACL（認証が無効なとき、admin、規則が無いときは `None`）。
pub(super) fn session_policy(
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    require_auth: bool,
    role: crate::auth::Role,
    user: &str,
) -> Option<AccessPolicy> {
    if !require_auth {
        return None;
    }
    auth.lock()
        .unwrap_or_else(|e| e.into_inner())
        .acl()
        .policy_for(user, role)
}

/// ACL で書き込みを拒否したエラーか。
pub(super) fn is_access_denied(resp: &Response) -> bool {
    matches!(resp, Response::Error { message } if message.contains("access denied:"))
}

/// ACL 文の主体・対象・操作をサーバーの型に変換する。
pub(super) fn acl_parts(
    st: &maharit_query::ast::AclStatement,
) -> Result<
    (
        crate::auth::AclSubject,
        crate::auth::AclResource,
        crate::auth::AclAction,
    ),
    Response,
> {
    use crate::auth::{AclAction, AclResource, AclSubject};
    use maharit_query::ast::{AclAction as A, AclGrantee, AclTarget};
    let subject = match &st.grantee {
        AclGrantee::User(u) => AclSubject::User(u.clone()),
        AclGrantee::Role(r) => match parse_role_str(r) {
            Some(role) => AclSubject::Role(role),
            None => {
                return Err(Response::Error {
                    message: format!("unknown role: '{}'", r),
                });
            }
        },
    };
    let resource = match &st.target {
        AclTarget::AllLabels => AclResource::AllLabels,
        AclTarget::Label(l) => AclResource::Label(l.clone()),
        AclTarget::Property(l, p) => AclResource::Property(l.clone(), p.clone()),
    };
    let action = match st.action {
        A::Read => AclAction::Read,
        A::Write => AclAction::Write,
    };
    Ok((subject, resource, action))
}

/// 規則を `GRANT READ ON :Label(prop) TO USER alice` の形で表す（監査ログと SHOW ACL 用）。
pub(super) fn describe_acl(
    subject: &crate::auth::AclSubject,
    resource: &crate::auth::AclResource,
    action: crate::auth::AclAction,
) -> (String, String, String) {
    use crate::auth::{AclAction, AclResource, AclSubject};
    let subject = match subject {
        AclSubject::User(u) => format!("USER {}", u),
        AclSubject::Role(r) => format!("ROLE {}", role_label(r)),
    };
    let resource = match resource {
        AclResource::AllLabels => ":*".to_string(),
        AclResource::Label(l) => format!(":{}", l),
        AclResource::Property(l, p) => format!(":{}({})", l, p),
    };
    let action = match action {
        AclAction::Read => "READ",
        AclAction::Write => "WRITE",
    };
    (subject, resource, action.to_string())
}

/// 単一メッセージの `result` レスポンスを作る。
pub(super) fn result_message(msg: String) -> Response {
    let mut row = HashMap::new();
    row.insert("result".to_string(), serde_json::Value::String(msg));
    Response::Result { rows: vec![row] }
}

/// ユーザー管理文（CREATE/DROP/ALTER USER, SHOW USERS）をサーバーの
/// [`AuthManager`] に配線する。対象文なら `Some(Response)` を返し、そうでなければ
/// `None`（呼び出し側は通常のクエリ実行にフォールバック）。
///
/// クエリ実行エンジンはこれらを stub（実際には何もしない）として扱うため、
/// ここでサーバーの認証状態に対して実処理する。認証有効時は admin（ManageUsers）
/// 権限を要求する。
pub(super) fn try_handle_user_management(
    query: &str,
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    ast_cache: &Arc<Mutex<AstCache>>,
    role: crate::auth::Role,
    require_auth: bool,
    audit: Option<&crate::audit::AuditLogger>,
    actor: &str,
) -> Option<Response> {
    use crate::audit::AuthEvent;
    use maharit_query::ast::Statement;

    let stmt = ast_cache.lock().unwrap().get_or_parse(query).ok()?;
    let is_user_mgmt = matches!(
        stmt,
        Statement::CreateUser(_)
            | Statement::DropUser(_)
            | Statement::AlterUser(_)
            | Statement::ShowUsers
            | Statement::Acl(_)
            | Statement::ShowAcl
    );
    if !is_user_mgmt {
        return None;
    }

    // RBAC: 認証有効時はユーザー管理を admin (ManageUsers) に限定する。
    if require_auth
        && crate::auth::AuthManager::check_role_permission(
            role,
            crate::auth::Operation::ManageUsers,
        )
        .is_err()
    {
        audit_denied(audit, actor, "manage_users");
        return Some(Response::AuthError {
            message: "permission denied: user management requires admin role".to_string(),
        });
    }

    let mut mgr = auth.lock().unwrap();
    let resp = match stmt {
        Statement::CreateUser(cu) => match parse_role_str(&cu.role) {
            Some(r) => match mgr.create_user(&cu.username, &cu.password, r) {
                Ok(()) => {
                    audit_log(
                        audit,
                        AuthEvent::UserCreated {
                            username: cu.username.clone(),
                            by: actor.to_string(),
                        },
                    );
                    result_message(format!(
                        "User '{}' created with role '{}'",
                        cu.username, cu.role
                    ))
                }
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            },
            None => Response::Error {
                message: format!("unknown role: '{}'", cu.role),
            },
        },
        Statement::DropUser(du) => match mgr.drop_user(&du.username) {
            Ok(()) => {
                audit_log(
                    audit,
                    AuthEvent::UserDeleted {
                        username: du.username.clone(),
                        by: actor.to_string(),
                    },
                );
                result_message(format!("User '{}' dropped", du.username))
            }
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },
        Statement::AlterUser(au) => {
            let mut result: Result<(), crate::auth::AuthError> = Ok(());
            if let Some(pw) = &au.password {
                result = mgr.alter_user_password(&au.username, pw);
            }
            if result.is_ok()
                && let Some(rl) = &au.role
            {
                match parse_role_str(rl) {
                    Some(r) => result = mgr.alter_user_role(&au.username, r),
                    None => {
                        return Some(Response::Error {
                            message: format!("unknown role: '{}'", rl),
                        });
                    }
                }
            }
            match result {
                Ok(()) => {
                    audit_log(
                        audit,
                        AuthEvent::UserModified {
                            username: au.username.clone(),
                            by: actor.to_string(),
                        },
                    );
                    result_message(format!("User '{}' altered", au.username))
                }
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Statement::Acl(st) => {
            use maharit_query::ast::AclCommand;
            let (subject, resource, action) = match acl_parts(&st) {
                Ok(parts) => parts,
                Err(resp) => return Some(resp),
            };
            let (who, what, act) = describe_acl(&subject, &resource, action);
            let result = match st.command {
                AclCommand::Grant | AclCommand::Deny => {
                    let deny = st.command == AclCommand::Deny;
                    let rule = crate::auth::AclRule {
                        subject,
                        resource,
                        action,
                        permission: if deny {
                            crate::auth::AclPermission::Deny
                        } else {
                            crate::auth::AclPermission::Allow
                        },
                    };
                    let verb = if deny { "DENY" } else { "GRANT" };
                    mgr.add_acl_rule(rule)
                        .map(|()| format!("{} {} ON {} TO {}", verb, act, what, who))
                }
                AclCommand::Revoke => {
                    let n = mgr.revoke_acl(&subject, &resource, action);
                    Ok(format!(
                        "REVOKE {} ON {} FROM {}: {} rule(s) removed",
                        act, what, who, n
                    ))
                }
            };
            match result {
                Ok(msg) => {
                    audit_log(
                        audit,
                        AuthEvent::AclChanged {
                            rule: msg.clone(),
                            by: actor.to_string(),
                        },
                    );
                    result_message(msg)
                }
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Statement::ShowAcl => {
            let rows = mgr
                .acl()
                .rules()
                .iter()
                .map(|r| {
                    let (who, what, act) = describe_acl(&r.subject, &r.resource, r.action);
                    let permission = match r.permission {
                        crate::auth::AclPermission::Allow => "GRANT",
                        crate::auth::AclPermission::Deny => "DENY",
                    };
                    HashMap::from([
                        ("subject".to_string(), serde_json::Value::String(who)),
                        ("resource".to_string(), serde_json::Value::String(what)),
                        ("action".to_string(), serde_json::Value::String(act)),
                        (
                            "permission".to_string(),
                            serde_json::Value::String(permission.to_string()),
                        ),
                    ])
                })
                .collect();
            Response::Result { rows }
        }
        Statement::ShowUsers => {
            let rows: Vec<HashMap<String, serde_json::Value>> = mgr
                .list_users()
                .iter()
                .map(|u| {
                    let mut m = HashMap::new();
                    m.insert(
                        "username".to_string(),
                        serde_json::Value::String(u.username.clone()),
                    );
                    m.insert(
                        "role".to_string(),
                        serde_json::Value::String(role_label(&u.role)),
                    );
                    m.insert("active".to_string(), serde_json::Value::Bool(u.active));
                    m
                })
                .collect();
            Response::Result { rows }
        }
        _ => unreachable!("filtered to user-management statements above"),
    };
    Some(resp)
}
