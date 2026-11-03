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

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use maharit_query::Parser;

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            markdown_files(&path, out);
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

/// Known-failure key: `docs/<file>: <first non-comment line>`.
fn failure_key(rel: &Path, stmt: &str) -> String {
    let first = stmt
        .lines()
        .map(str::trim)
        .find(|l| !l.starts_with("//"))
        .unwrap_or_default();
    format!("docs/{}: {first}", rel.display())
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
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
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
