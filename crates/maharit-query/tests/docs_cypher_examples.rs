//! Every ```cypher code block under `docs/` must parse.
//!
//! Format rules for those blocks (so they can be checked mechanically):
//! - statements are separated by a blank line (one statement per paragraph);
//! - explanations and sample output go in `//` comments, or in a ```text block.
//!
//! Only parsing is checked: procedures and functions named in the docs may
//! still be unimplemented at execution time.
//!
//! Statements documenting syntax the engine does not support yet are listed in
//! `docs_cypher_known_failures.txt` as `docs/<file>: <first code line>`. They
//! must still fail; once one parses (or disappears) the test asks you to drop
//! its entry, and any failure not on the list fails the test.
//!
//! Translations (`foo.en.md` next to the Japanese source `foo.md`, see
//! mkdocs-static-i18n) share the source's known-failure keys, and their cypher
//! blocks must match the source's statement for statement: only `//` comments
//! may be translated.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use maharit_query::Parser;

/// Language suffixes of translated pages (`foo.<lang>.md`); Japanese `foo.md` is the source.
const TRANSLATION_LANGS: &[&str] = &["en"];

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

/// Every `.md` under `dir`, skipping `_`-prefixed directories (e.g. `_i18n/`,
/// which mkdocs excludes from the site).
fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('_'));
            if !hidden {
                markdown_files(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

/// Statements of every ```cypher block in `markdown`, with their 1-based line numbers.
fn cypher_statements(markdown: &str) -> Vec<(usize, String)> {
    let mut statements = Vec::new();
    let mut in_cypher = false;
    let mut in_other_fence = false;
    let mut current: Option<(usize, String)> = None;

    for (i, line) in markdown.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_cypher {
                statements.extend(current.take());
                in_cypher = false;
            } else if in_other_fence {
                in_other_fence = false;
            } else if trimmed.trim_start_matches('`').trim() == "cypher" {
                in_cypher = true;
            } else {
                in_other_fence = true;
            }
            continue;
        }
        if !in_cypher {
            continue;
        }
        if line.trim().is_empty() {
            statements.extend(current.take());
            continue;
        }
        match &mut current {
            Some((_, text)) => {
                text.push('\n');
                text.push_str(line);
            }
            None => current = Some((i + 1, line.to_string())),
        }
    }
    statements
}

/// True when `stmt` holds nothing but `//` comment lines.
fn is_comment_only(stmt: &str) -> bool {
    stmt.lines().all(|l| l.trim().starts_with("//"))
}

/// For a translated page `foo.<lang>.md`, the Japanese source `foo.md`.
fn translation_source(rel: &Path) -> Option<PathBuf> {
    let name = rel.file_name()?.to_str()?;
    TRANSLATION_LANGS.iter().find_map(|lang| {
        let stem = name.strip_suffix(&format!(".{lang}.md"))?;
        Some(rel.with_file_name(format!("{stem}.md")))
    })
}

/// Known-failure key: `docs/<file>: <first non-comment line>`. A translation
/// uses its source's file name, so one entry covers every language.
fn failure_key(rel: &Path, stmt: &str) -> String {
    let first = stmt
        .lines()
        .map(str::trim)
        .find(|l| !l.starts_with("//"))
        .unwrap_or_default();
    let file = translation_source(rel).unwrap_or_else(|| rel.to_path_buf());
    format!("docs/{}: {first}", file.display())
}

/// `code` without `//` and `/* */` comments (those outside string literals).
/// Block comments keep their newlines so line structure is preserved.
fn strip_comments(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut chars = code.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"' | '\'' | '`', _) => {
                quote = Some(c);
                out.push(c);
            }
            ('/', Some('/')) => {
                // line comment: drop up to (not including) the newline
                while chars.peek().is_some_and(|&n| n != '\n') {
                    chars.next();
                }
            }
            ('/', Some('*')) => {
                // block comment: drop through the closing */, keep its newlines
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                    }
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn cypher_code(markdown: &str) -> Vec<(usize, String)> {
    cypher_statements(markdown)
        .into_iter()
        .filter_map(|(line, stmt)| {
            let stripped = strip_comments(&stmt);
            let code: Vec<&str> = stripped
                .lines()
                .map(str::trim_end)
                .filter(|l| !l.trim().is_empty())
                .collect();
            (!code.is_empty()).then(|| (line, code.join("\n")))
        })
        .collect()
}

fn known_failures() -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/docs_cypher_known_failures.txt");
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

#[test]
fn docs_cypher_blocks_parse() {
    let docs = docs_dir();
    let mut files = Vec::new();
    markdown_files(&docs, &mut files);
    assert!(
        !files.is_empty(),
        "no markdown found under {}",
        docs.display()
    );

    let known = known_failures();
    let mut still_failing = BTreeSet::new();
    let mut checked = 0;
    let mut failures = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(&docs).unwrap_or(file);
        let markdown = fs::read_to_string(file).unwrap();
        for (line, stmt) in cypher_statements(&markdown) {
            if is_comment_only(&stmt) {
                continue;
            }
            checked += 1;
            if let Err(e) = Parser::new(&stmt).and_then(|mut p| p.parse()) {
                let key = failure_key(rel, &stmt);
                if known.contains(&key) {
                    still_failing.insert(key);
                } else {
                    failures.push(format!("docs/{}:{line}: {e}\n{stmt}\n", rel.display()));
                }
            }
        }
    }
    assert!(checked > 0, "no cypher statements found under docs/");

    let stale: Vec<&String> = known.difference(&still_failing).collect();
    assert!(
        stale.is_empty(),
        "these docs statements now parse (or are gone) — remove them from \
         docs_cypher_known_failures.txt:\n{}",
        stale
            .iter()
            .map(|k| format!("  {k}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} docs cypher statement(s) do not parse:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn translated_cypher_blocks_match_source() {
    let docs = docs_dir();
    let mut files = Vec::new();
    markdown_files(&docs, &mut files);

    let mut problems = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(&docs).unwrap_or(file);
        let Some(source_rel) = translation_source(rel) else {
            continue;
        };
        let source = docs.join(&source_rel);
        let Ok(source_md) = fs::read_to_string(&source) else {
            problems.push(format!(
                "docs/{}: no Japanese source docs/{}",
                rel.display(),
                source_rel.display()
            ));
            continue;
        };
        let translated = cypher_code(&fs::read_to_string(file).unwrap());
        let original = cypher_code(&source_md);
        let mismatch = translated
            .iter()
            .map(Some)
            .chain(std::iter::repeat(None))
            .zip(original.iter().map(Some).chain(std::iter::repeat(None)))
            .take(translated.len().max(original.len()))
            .find(|(t, o)| t.map(|x| &x.1) != o.map(|x| &x.1));
        if let Some((t, o)) = mismatch {
            let show = |side: Option<&(usize, String)>, path: &Path| match side {
                Some((line, code)) => format!("docs/{}:{line}\n{code}", path.display()),
                None => format!("docs/{}: (no more statements)", path.display()),
            };
            problems.push(format!(
                "cypher differs from the source (only // and /* */ comments may be translated):\n  translation: {}\n  source:      {}",
                show(t, rel).replace('\n', "\n    "),
                show(o, &source_rel).replace('\n', "\n    ")
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n\n"));
}

#[test]
fn strip_comments_keeps_strings() {
    assert_eq!(strip_comments("MATCH (n) // 説明"), "MATCH (n) ");
    assert_eq!(
        strip_comments(r#"RETURN "http://example.com" // url"#),
        r#"RETURN "http://example.com" "#
    );
    assert_eq!(strip_comments("RETURN 'a\\'//b'"), "RETURN 'a\\'//b'");
    assert_eq!(strip_comments("RETURN 4 / 2"), "RETURN 4 / 2");
    assert_eq!(strip_comments("RETURN 1 /* x */ + 2"), "RETURN 1  + 2");
    assert_eq!(strip_comments("RETURN '/* x */'"), "RETURN '/* x */'");
    assert_eq!(
        cypher_code("```cypher\n// 見出し\nMATCH (n) // a\nRETURN n\n```\n"),
        cypher_code("```cypher\n// heading\nMATCH (n) // b\nRETURN n\n```\n")
    );
    assert_eq!(
        cypher_code("```cypher\nMATCH (n)\n/* 複数行の\n   コメント */\nRETURN n\n```\n"),
        cypher_code("```cypher\nMATCH (n)\n/* a block\n   comment */\nRETURN n\n```\n")
    );
}
