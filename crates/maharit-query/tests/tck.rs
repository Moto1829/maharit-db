//! openCypher TCK の式のシナリオ（`tests/tck/features/`）を実行して、言語の意味を外部の基準で確かめる（task181）。
//!
//! Gherkin のうち TCK が使う手順だけを読む:
//! `Given an empty graph` / `Given any graph` / `having executed:` / `parameters are:` /
//! `When executing query:` / `Then the result should be, in any order:` / `Then the result should be empty` /
//! `Then a <Error> should be raised …`。副作用の検査（`no side effects` など）は見ない。
//!
//! 通らないシナリオは `tests/tck_known_failures.txt` に載せる。載っていないシナリオが落ちる、または
//! 載っているシナリオが通るとテストが失敗する（後者は一覧から消すため）。
//! 一覧を作り直すときは `MAHARIT_TCK_WRITE=1 cargo test -p maharit-query --test tck`。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use maharit_core::{Graph, GraphBackend, PropertyValue};
use maharit_query::{Executor, Parser, Value};

const KNOWN_FAILURES: &str = "tests/tck_known_failures.txt";

// ── TCK の値 ──────────────────────────────────────────────────────────────────

/// 比較用に正規化した値。
#[derive(Debug, Clone, PartialEq)]
enum V {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<V>),
    Map(BTreeMap<String, V>),
    Node(BTreeSet<String>, BTreeMap<String, V>),
    Rel(String, BTreeMap<String, V>),
    /// 比べ方を決めていない値（パスなど）。常に不一致になる
    Other(String),
}

impl V {
    fn same(&self, other: &V) -> bool {
        match (self, other) {
            (V::Float(a), V::Float(b)) => {
                (a.is_nan() && b.is_nan())
                    || a == b
                    || (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
            }
            (V::List(a), V::List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same(y))
            }
            (V::Map(a), V::Map(b)) => {
                a.len() == b.len() && a.iter().all(|(k, x)| b.get(k).is_some_and(|y| x.same(y)))
            }
            (V::Node(la, pa), V::Node(lb, pb)) => {
                la == lb && V::Map(pa.clone()).same(&V::Map(pb.clone()))
            }
            (V::Rel(ta, pa), V::Rel(tb, pb)) => {
                ta == tb && V::Map(pa.clone()).same(&V::Map(pb.clone()))
            }
            (V::Other(_), _) | (_, V::Other(_)) => false,
            _ => self == other,
        }
    }
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn rest_starts(&mut self, w: &str) -> bool {
        self.ws();
        self.s[self.i..].starts_with(w.as_bytes())
    }
    fn ident(&mut self) -> Option<String> {
        self.ws();
        if self.eat(b'`') {
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'`' {
                self.i += 1;
            }
            let id = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
            self.i += 1;
            return Some(id);
        }
        let start = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_')
        {
            self.i += 1;
        }
        (self.i > start).then(|| String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }
    fn props(&mut self) -> Option<BTreeMap<String, V>> {
        let mut m = BTreeMap::new();
        if !self.eat(b'{') {
            return Some(m);
        }
        if self.eat(b'}') {
            return Some(m);
        }
        loop {
            let k = self.ident()?;
            if !self.eat(b':') {
                return None;
            }
            m.insert(k, self.value()?);
            if self.eat(b',') {
                continue;
            }
            return self.eat(b'}').then_some(m);
        }
    }
    fn value(&mut self) -> Option<V> {
        match self.peek()? {
            b'\'' => {
                self.i += 1;
                let mut out = Vec::new();
                while self.i < self.s.len() {
                    let c = self.s[self.i];
                    self.i += 1;
                    match c {
                        b'\\' if self.i < self.s.len() => {
                            out.push(self.s[self.i]);
                            self.i += 1;
                        }
                        b'\'' => return Some(V::Str(String::from_utf8_lossy(&out).into_owned())),
                        _ => out.push(c),
                    }
                }
                None
            }
            b'[' => {
                self.i += 1;
                // リレーションシップ `[:T {…}]`
                if self.peek() == Some(b':') {
                    self.i += 1;
                    let t = self.ident()?;
                    let p = self.props()?;
                    return self.eat(b']').then_some(V::Rel(t, p));
                }
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Some(V::List(items));
                }
                loop {
                    items.push(self.value()?);
                    if self.eat(b',') {
                        continue;
                    }
                    return self.eat(b']').then_some(V::List(items));
                }
            }
            b'{' => self.props().map(V::Map),
            b'(' => {
                self.i += 1;
                let mut labels = BTreeSet::new();
                while self.eat(b':') {
                    labels.insert(self.ident()?);
                }
                let p = self.props()?;
                self.eat(b')').then_some(V::Node(labels, p))
            }
            b'<' => {
                let start = self.i;
                self.i = self.s.len();
                Some(V::Other(
                    String::from_utf8_lossy(&self.s[start..]).into_owned(),
                ))
            }
            _ => {
                for (w, v) in [
                    ("null", V::Null),
                    ("true", V::Bool(true)),
                    ("false", V::Bool(false)),
                    ("NaN", V::Float(f64::NAN)),
                    ("Infinity", V::Float(f64::INFINITY)),
                    ("-Infinity", V::Float(f64::NEG_INFINITY)),
                ] {
                    if self.rest_starts(w) {
                        self.i += w.len();
                        return Some(v);
                    }
                }
                let start = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_alphanumeric() || b"-+.".contains(&self.s[self.i]))
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).ok()?;
                if t.contains(['.', 'e', 'E']) && !t.starts_with("0x") {
                    t.parse().ok().map(V::Float)
                } else {
                    t.parse().ok().map(V::Int)
                }
            }
        }
    }
}

fn parse_tck_value(cell: &str) -> V {
    let mut r = Reader {
        s: cell.as_bytes(),
        i: 0,
    };
    match r.value() {
        Some(v) if r.peek().is_none() => v,
        _ => V::Other(cell.to_string()),
    }
}

fn from_property(p: &PropertyValue) -> V {
    from_value(&Value::from(p), None)
}

fn props(map: &HashMap<String, PropertyValue>) -> BTreeMap<String, V> {
    map.iter()
        .map(|(k, v)| (k.clone(), from_property(v)))
        .collect()
}

fn from_value(v: &Value, graph: Option<&Graph>) -> V {
    match v {
        Value::Null => V::Null,
        Value::Bool(b) => V::Bool(*b),
        Value::Int(n) => V::Int(*n),
        Value::Float(f) => V::Float(*f),
        Value::String(s) => V::Str(s.clone()),
        Value::List(items) => V::List(items.iter().map(|x| from_value(x, graph)).collect()),
        Value::Map(m) => V::Map(
            m.iter()
                .map(|(k, x)| (k.clone(), from_value(x, graph)))
                .collect(),
        ),
        Value::NodeData {
            labels, properties, ..
        } => V::Node(labels.iter().cloned().collect(), props(properties)),
        Value::Node(id) => match graph.and_then(|g| GraphBackend::get_node(g, *id)) {
            Some(n) => V::Node(n.labels.iter().cloned().collect(), props(&n.properties)),
            None => V::Other(format!("node {id}")),
        },
        Value::Edge(id) => match graph.and_then(|g| GraphBackend::get_edge(g, *id)) {
            Some(e) => V::Rel(e.label.clone(), props(&e.properties)),
            None => V::Other(format!("relationship {id}")),
        },
        // TCK は時間型を文字列で書く
        Value::Date(_) | Value::DateTime(_) | Value::Duration { .. } => V::Str(v.to_string()),
        Value::Path { .. } => V::Other(v.to_string()),
    }
}

fn to_param(v: &V) -> Value {
    match v {
        V::Null => Value::Null,
        V::Bool(b) => Value::Bool(*b),
        V::Int(n) => Value::Int(*n),
        V::Float(f) => Value::Float(*f),
        V::Str(s) => Value::String(s.clone()),
        V::List(items) => Value::List(items.iter().map(to_param).collect()),
        V::Map(m) => Value::Map(m.iter().map(|(k, x)| (k.clone(), to_param(x))).collect()),
        V::Node(..) | V::Rel(..) | V::Other(_) => Value::Null,
    }
}

// ── シナリオ ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Step {
    Setup(String),
    Params(Vec<(String, String)>),
    Query(String),
    ExpectRows {
        rows: Vec<Vec<String>>,
        ordered: bool,
    },
    ExpectEmpty,
    ExpectError(String),
}

struct Scenario {
    id: String,
    steps: Vec<Step>,
}

fn table_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let inner = t
        .strip_prefix('|')
        .and_then(|x| x.strip_suffix('|'))
        .unwrap_or(t);
    // セルの中の `\|` は区切りではない
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn parse_feature(rel: &str, text: &str) -> Vec<Scenario> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut background: Vec<Step> = Vec::new();
    let mut current: Option<(String, Vec<Step>, bool)> = None; // (名前, 手順, Outline か)
    let mut examples: Vec<Vec<String>> = Vec::new();

    let flush = |cur: &mut Option<(String, Vec<Step>, bool)>,
                 examples: &mut Vec<Vec<String>>,
                 out: &mut Vec<Scenario>| {
        if let Some((name, steps, outline)) = cur.take() {
            if outline && examples.len() > 1 {
                let header = examples[0].clone();
                for (n, row) in examples[1..].iter().enumerate() {
                    let sub = |s: &str| {
                        let mut s = s.to_string();
                        for (h, v) in header.iter().zip(row) {
                            s = s.replace(&format!("<{h}>"), v);
                        }
                        s
                    };
                    let steps = steps
                        .iter()
                        .map(|st| match st {
                            Step::Setup(q) => Step::Setup(sub(q)),
                            Step::Query(q) => Step::Query(sub(q)),
                            Step::Params(p) => {
                                Step::Params(p.iter().map(|(k, v)| (sub(k), sub(v))).collect())
                            }
                            Step::ExpectRows { rows, ordered } => Step::ExpectRows {
                                rows: rows
                                    .iter()
                                    .map(|r| r.iter().map(|c| sub(c)).collect())
                                    .collect(),
                                ordered: *ordered,
                            },
                            other => other.clone(),
                        })
                        .collect();
                    out.push(Scenario {
                        id: format!("{rel}: {} #{}", sub(&name), n + 1),
                        steps,
                    });
                }
            } else {
                out.push(Scenario {
                    id: format!("{rel}: {name}"),
                    steps,
                });
            }
        }
        examples.clear();
    };

    let docstring = |i: &mut usize| -> String {
        // 次の `"""` から閉じる `"""` まで
        while *i < lines.len() && lines[*i].trim() != "\"\"\"" {
            *i += 1;
        }
        let indent = lines.get(*i).map_or(0, |l| l.len() - l.trim_start().len());
        *i += 1;
        let mut body = Vec::new();
        while *i < lines.len() && lines[*i].trim() != "\"\"\"" {
            let l = lines[*i];
            body.push(if l.len() >= indent {
                &l[indent..]
            } else {
                l.trim_start()
            });
            *i += 1;
        }
        *i += 1;
        body.join("\n")
    };
    let table = |i: &mut usize| -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        while *i < lines.len() && lines[*i].trim_start().starts_with('|') {
            rows.push(table_row(lines[*i]));
            *i += 1;
        }
        rows
    };

    let mut in_background = false;
    while i < lines.len() {
        let line = lines[i].trim();
        i += 1;
        let push = |cur: &mut Option<(String, Vec<Step>, bool)>,
                    bg: &mut Vec<Step>,
                    in_bg: bool,
                    st: Step| {
            if in_bg {
                bg.push(st);
            } else if let Some((_, steps, _)) = cur.as_mut() {
                steps.push(st);
            }
        };
        if let Some(name) = line
            .strip_prefix("Scenario Outline:")
            .or_else(|| line.strip_prefix("Scenario:"))
        {
            flush(&mut current, &mut examples, &mut out);
            in_background = false;
            current = Some((
                name.trim().to_string(),
                background.clone(),
                line.starts_with("Scenario Outline"),
            ));
        } else if line.starts_with("Background:") {
            in_background = true;
        } else if line.starts_with("Examples:") {
            examples = table(&mut i);
        } else if line.ends_with("having executed:") {
            let q = docstring(&mut i);
            push(&mut current, &mut background, in_background, Step::Setup(q));
        } else if line.ends_with("parameters are:") {
            let rows = table(&mut i);
            let p = rows
                .into_iter()
                .filter(|r| r.len() >= 2)
                .map(|r| (r[0].clone(), r[1].clone()))
                .collect();
            push(
                &mut current,
                &mut background,
                in_background,
                Step::Params(p),
            );
        } else if line.starts_with("When executing") {
            let q = docstring(&mut i);
            push(&mut current, &mut background, in_background, Step::Query(q));
        } else if line.starts_with("Then the result should be empty") {
            push(
                &mut current,
                &mut background,
                in_background,
                Step::ExpectEmpty,
            );
        } else if line.starts_with("Then the result should be") {
            let ordered = line.contains("in order");
            let rows = table(&mut i);
            push(
                &mut current,
                &mut background,
                in_background,
                Step::ExpectRows { rows, ordered },
            );
        } else if line.starts_with("Then a ") && line.contains("should be raised") {
            push(
                &mut current,
                &mut background,
                in_background,
                Step::ExpectError(line.to_string()),
            );
        } else if line.ends_with("side effects should be:") {
            table(&mut i);
        }
    }
    flush(&mut current, &mut examples, &mut out);
    out
}

fn run_statement(
    graph: &mut Graph,
    query: &str,
    params: &HashMap<String, Value>,
) -> Result<maharit_query::executor::ResultSet, String> {
    let stmt = Parser::new(query)
        .and_then(|mut p| p.parse())
        .map_err(|e| format!("parse error: {e}"))?;
    Executor::new(graph)
        .execute_with_params(stmt, params.clone())
        .map_err(|e| format!("execution error: {e}"))
}

/// シナリオを実行し、通らなければ理由を返す。
fn run_scenario(sc: &Scenario) -> Result<(), String> {
    let mut graph = Graph::new();
    let mut params = HashMap::new();
    let mut last: Option<Result<maharit_query::executor::ResultSet, String>> = None;
    for step in &sc.steps {
        match step {
            Step::Setup(q) => {
                run_statement(&mut graph, q, &HashMap::new())
                    .map_err(|e| format!("setup failed: {e}"))?;
            }
            Step::Params(p) => {
                for (k, v) in p {
                    params.insert(k.clone(), to_param(&parse_tck_value(v)));
                }
            }
            Step::Query(q) => last = Some(run_statement(&mut graph, q, &params)),
            Step::ExpectError(what) => match last.take() {
                Some(Err(_)) => {}
                Some(Ok(rs)) => {
                    return Err(format!(
                        "expected an error ({what}), got {} row(s)",
                        rs.rows.len()
                    ));
                }
                None => return Err("no query".into()),
            },
            Step::ExpectEmpty => match last.take() {
                Some(Ok(rs)) if rs.rows.is_empty() => {}
                Some(Ok(rs)) => return Err(format!("expected no rows, got {}", rs.rows.len())),
                Some(Err(e)) => return Err(e),
                None => return Err("no query".into()),
            },
            Step::ExpectRows { rows, ordered } => {
                let rs = match last.take() {
                    Some(Ok(rs)) => rs,
                    Some(Err(e)) => return Err(e),
                    None => return Err("no query".into()),
                };
                let expected: Vec<Vec<V>> = rows[1..]
                    .iter()
                    .map(|r| r.iter().map(|c| parse_tck_value(c)).collect())
                    .collect();
                let actual: Vec<Vec<V>> = rs
                    .rows
                    .iter()
                    .map(|r| {
                        r.columns
                            .iter()
                            .map(|v| from_value(v, Some(&graph)))
                            .collect()
                    })
                    .collect();
                let row_eq = |a: &Vec<V>, b: &Vec<V>| {
                    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same(y))
                };
                let ok = if *ordered {
                    expected.len() == actual.len()
                        && expected.iter().zip(&actual).all(|(e, a)| row_eq(e, a))
                } else {
                    let mut left: Vec<&Vec<V>> = actual.iter().collect();
                    expected.len() == actual.len()
                        && expected
                            .iter()
                            .all(|e| match left.iter().position(|a| row_eq(e, a)) {
                                Some(p) => {
                                    left.swap_remove(p);
                                    true
                                }
                                None => false,
                            })
                };
                if !ok {
                    return Err(format!("expected {expected:?}, got {actual:?}"));
                }
            }
        }
    }
    Ok(())
}

fn feature_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("tck dir")
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            feature_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "feature") {
            out.push(p);
        }
    }
}

#[test]
fn opencypher_tck_expressions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tck/features");
    let mut files = Vec::new();
    feature_files(&root, &mut files);
    let mut scenarios = Vec::new();
    for f in &files {
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        scenarios.extend(parse_feature(&rel, &std::fs::read_to_string(f).unwrap()));
    }
    assert!(
        scenarios.len() > 400,
        "too few scenarios parsed: {}",
        scenarios.len()
    );

    let mut failing = BTreeMap::new();
    for sc in &scenarios {
        if std::env::var_os("MAHARIT_TCK_TRACE").is_some() {
            eprintln!("RUN {}", sc.id);
        }
        // パニックも失敗として数える（一覧に載せて先に進む）
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_scenario(sc)))
            .unwrap_or_else(|_| Err("panicked".into()));
        if let Err(e) = r {
            failing.insert(sc.id.clone(), e);
        }
    }
    let passed = scenarios.len() - failing.len();
    println!(
        "openCypher TCK (expressions): {passed} / {} scenarios pass",
        scenarios.len()
    );

    if let Some(path) = std::env::var_os("MAHARIT_TCK_REPORT") {
        let report: String = failing
            .iter()
            .map(|(id, e)| format!("{id}\n    {e}\n"))
            .collect();
        std::fs::write(path, report).unwrap();
    }
    let known_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(KNOWN_FAILURES);
    if std::env::var_os("MAHARIT_TCK_WRITE").is_some() {
        let mut text = String::from(
            "# openCypher TCK scenarios that do not pass yet (see tests/tck.rs).\n\
             # Regenerate: MAHARIT_TCK_WRITE=1 cargo test -p maharit-query --test tck\n",
        );
        for id in failing.keys() {
            text.push_str(id);
            text.push('\n');
        }
        std::fs::write(&known_path, text).unwrap();
        return;
    }
    let known: BTreeSet<String> = std::fs::read_to_string(&known_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    let new_failures: Vec<String> = failing
        .iter()
        .filter(|(id, _)| !known.contains(*id))
        .map(|(id, e)| format!("{id}\n    {e}"))
        .collect();
    let now_passing: Vec<&String> = known
        .iter()
        .filter(|id| !failing.contains_key(*id))
        .collect();
    assert!(
        new_failures.is_empty() && now_passing.is_empty(),
        "TCK: {} new failure(s):\n{}\n\n{} scenario(s) now pass; remove them from {KNOWN_FAILURES}:\n{}",
        new_failures.len(),
        new_failures.join("\n"),
        now_passing.len(),
        now_passing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
