//! Local REPL for testing RPC handlers without Tabularis.
//!
//! Reads JSON-RPC requests from stdin, dispatches them, and prints responses.
//! Usage:
//!   cargo run --bin test_plugin
//!   Then type JSON-RPC requests, one per line.

use std::io::{self, BufRead, Write};

#[tokio::main]
async fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    println!("Tabularis DynamoDB Plugin — Test REPL");
    println!("Enter JSON-RPC requests (one per line). Ctrl+D to exit.");
    println!();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error reading input: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        // Handle special commands
        match trimmed {
            ":help" | ":h" => {
                println!("Commands:");
                println!("  :help / :h   Show this help");
                println!("  :ping        Send a ping request");
                println!("  :init        Send an initialize request");
                println!("  :tables      Send get_tables request");
                println!("  :exit / :q   Exit");
                println!("  <json>       Send arbitrary JSON-RPC request");
                println!();
                continue;
            }
            ":exit" | ":q" => break,
            ":ping" => {
                let req = r#"{"jsonrpc":"2.0","method":"ping","id":1,"params":{"params":{}}}"#;
                println!(">>> {req}");
                let resp = tabularis_dynamodb_plugin::rpc::handle_line(req).await;
                println!("<<< {}", serde_json::to_string_pretty(&resp).unwrap());
                println!();
                continue;
            }
            ":init" => {
                let req = r#"{"jsonrpc":"2.0","method":"initialize","id":1}"#;
                println!(">>> {req}");
                let resp = tabularis_dynamodb_plugin::rpc::handle_line(req).await;
                println!("<<< {}", serde_json::to_string_pretty(&resp).unwrap());
                println!();
                continue;
            }
            ":tables" => {
                let req =
                    r#"{"jsonrpc":"2.0","method":"get_tables","id":1,"params":{"params":{}}}"#;
                println!(">>> {req}");
                let resp = tabularis_dynamodb_plugin::rpc::handle_line(req).await;
                println!("<<< {}", serde_json::to_string_pretty(&resp).unwrap());
                println!();
                continue;
            }
            _ => {}
        }

        let response = tabularis_dynamodb_plugin::rpc::handle_line(trimmed).await;
        let body = serde_json::to_string_pretty(&response).unwrap();
        println!("{body}");
        println!();

        let _ = stdout.flush();
    }
}
