use std::borrow::Cow;

use colored::Colorize;
use maharit_core::Graph;
use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::history::DefaultHistory;
use rustyline::validate::{
    MatchingBracketValidator, ValidationContext, ValidationResult, Validator,
};
use rustyline::{Context, Editor, Helper};

const HISTORY_FILE: &str = ".maharit_history";

/// REPLヘルパー（補完、ヒント、検証）
pub struct ReplHelper {
    hinter: HistoryHinter,
    validator: MatchingBracketValidator,
}

impl ReplHelper {
    pub fn new() -> Self {
        Self {
            hinter: HistoryHinter::new(),
            validator: MatchingBracketValidator::new(),
        }
    }
}

impl Helper for ReplHelper {}

impl Completer for ReplHelper {
    type Candidate = String;
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<Self::Hint> {
        self.hinter.hint(line, pos, ctx)
    }
}

impl Highlighter for ReplHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(format!("\x1b[90m{}\x1b[0m", hint))
    }
}

impl Validator for ReplHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        self.validator.validate(ctx)
    }
}

/// REPL（Read-Eval-Print Loop）
pub struct Repl {
    editor: Editor<ReplHelper, DefaultHistory>,
    graph: Graph,
}

impl Repl {
    pub fn new() -> rustyline::Result<Self> {
        let helper = ReplHelper::new();
        let mut editor = Editor::new()?;
        editor.set_helper(Some(helper));

        // ヒストリーファイルの読み込み（存在しなくてもOK）
        let _ = editor.load_history(HISTORY_FILE);

        Ok(Self {
            editor,
            graph: Graph::new(),
        })
    }

    /// REPLを実行
    pub fn run(&mut self) -> rustyline::Result<()> {
        self.print_welcome();

        loop {
            match self.editor.readline("maharit> ") {
                Ok(line) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }

                    self.editor.add_history_entry(line)?;

                    if line.starts_with(':') {
                        if !self.handle_command(line) {
                            break;
                        }
                    } else {
                        self.handle_query(line);
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("^C");
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    println!("Goodbye!");
                    break;
                }
                Err(err) => {
                    eprintln!("{}: {:?}", "Error".red().bold(), err);
                    break;
                }
            }
        }

        // ヒストリーの保存
        let _ = self.editor.save_history(HISTORY_FILE);

        Ok(())
    }

    fn print_welcome(&self) {
        println!(
            "{} - Graph Database v{}",
            "MaharitDB".green().bold(),
            env!("CARGO_PKG_VERSION")
        );
        println!("Type {} for available commands", ":help".cyan());
        println!();
    }

    /// 組み込みコマンドを処理（継続するならtrue、終了するならfalse）
    fn handle_command(&mut self, line: &str) -> bool {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let command = parts.first().map(|s| s.to_lowercase());

        match command.as_deref() {
            Some(":quit") | Some(":exit") | Some(":q") => {
                println!("Goodbye!");
                false
            }
            Some(":help") | Some(":h") => {
                self.print_help();
                true
            }
            Some(":stats") => {
                self.print_stats();
                true
            }
            Some(":clear") => {
                self.clear_screen();
                true
            }
            Some(":dump") => {
                self.dump_graph();
                true
            }
            Some(":reset") => {
                self.graph = Graph::new();
                println!("Graph cleared.");
                true
            }
            Some(":tokens") => {
                // デバッグ用：クエリをトークン化して表示
                if parts.len() > 1 {
                    let query = parts[1..].join(" ");
                    self.show_tokens(&query);
                } else {
                    println!("Usage: :tokens <query>");
                }
                true
            }
            Some(cmd) => {
                println!("{}: {}", "Unknown command".yellow(), cmd);
                println!("Type {} for available commands", ":help".cyan());
                true
            }
            None => true,
        }
    }

    fn print_help(&self) {
        println!("Available commands:");
        println!("  :help, :h     - Show this help message");
        println!("  :stats        - Show graph statistics");
        println!("  :dump         - Display all nodes and edges");
        println!("  :clear        - Clear the screen");
        println!("  :reset        - Clear all data from the graph");
        println!("  :tokens <q>   - Show tokens for a query (debug)");
        println!("  :quit, :q     - Exit the REPL");
        println!();
        println!("Query examples:");
        println!("  CREATE (n:Person {{name: \"Alice\"}})");
        println!("  MATCH (n:Person) RETURN n");
    }

    fn print_stats(&self) {
        println!("Graph Statistics:");
        println!("  Nodes: {}", self.graph.node_count());
        println!("  Edges: {}", self.graph.edge_count());
    }

    fn clear_screen(&self) {
        // ANSI escape code for clearing screen
        print!("\x1b[2J\x1b[1;1H");
    }

    fn dump_graph(&self) {
        if self.graph.node_count() == 0 {
            println!("(empty graph)");
            return;
        }

        println!("Nodes ({}):", self.graph.node_count());
        for node in self.graph.nodes() {
            print!("  ({}", node.id);
            if !node.label.is_empty() {
                print!(":{}", node.label);
            }
            if !node.properties.is_empty() {
                print!(" {{");
                let props: Vec<String> = node
                    .properties
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect();
                print!("{}", props.join(", "));
                print!("}}");
            }
            println!(")");
        }

        if self.graph.edge_count() > 0 {
            println!("\nEdges ({}):", self.graph.edge_count());
            for edge in self.graph.edges() {
                print!("  ({})", edge.from);
                print!("-[:{}]->", edge.label);
                println!("({})", edge.to);
            }
        }
    }

    fn show_tokens(&self, query: &str) {
        use maharit_query::Lexer;

        match Lexer::new(query).tokenize() {
            Ok(tokens) => {
                for token in tokens {
                    println!(
                        "  {:?} @ {}:{}",
                        token.kind, token.span.line, token.span.column
                    );
                }
            }
            Err(e) => {
                println!("{}: {}", "Lexer error".red().bold(), e);
            }
        }
    }

    fn handle_query(&mut self, line: &str) {
        use maharit_query::{Executor, Parser};

        // パース
        let stmt = match Parser::new(line) {
            Ok(mut parser) => match parser.parse() {
                Ok(stmt) => stmt,
                Err(e) => {
                    println!("{}: {}", "Parse error".red().bold(), e);
                    return;
                }
            },
            Err(e) => {
                println!("{}: {}", "Lexer error".red().bold(), e);
                return;
            }
        };

        // 実行
        let mut executor = Executor::new(&mut self.graph);
        match executor.execute(stmt) {
            Ok(result) => {
                self.print_result(&result);
            }
            Err(e) => {
                println!("{}: {}", "Execution error".red().bold(), e);
            }
        }
    }

    fn print_result(&self, result: &maharit_query::ResultSet) {
        if result.columns.is_empty() || result.rows.is_empty() {
            println!("(no results)");
            return;
        }

        // ヘッダー表示
        println!("{}", result.columns.join(" | "));
        println!(
            "{}",
            "-".repeat(result.columns.iter().map(|c| c.len() + 3).sum::<usize>())
        );

        // 行表示
        for row in &result.rows {
            let values: Vec<String> = row.columns.iter().map(|v| format!("{}", v)).collect();
            println!("{}", values.join(" | "));
        }

        println!("\n{} row(s) returned", result.rows.len());
    }
}
