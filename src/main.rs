//! Tabularis driver plugin for MongoDB (6.0+).
//!
//! Speaks JSON-RPC 2.0 over stdin/stdout, one request per line, talking to
//! MongoDB through the official `mongodb` Rust driver. Native MongoDB shell
//! calls (`db.coll.find(...)`, `aggregate`, CRUD, ...) are accepted in the
//! query editor; collection browsing is auto-translated from the host's
//! `SELECT * FROM coll`.
//!
//! Release builds use the Windows "windows" subsystem so no console window is
//! allocated when Tabularis (a GUI app) spawns the plugin; stdio still works
//! through the pipes Tabularis sets up.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{self, BufRead, Write};

mod client;
mod config;
mod error;
mod handlers;
mod models;
mod rpc;
mod runtime;
mod utils;

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = rpc::handle_line(trimmed);
        let mut body = match serde_json::to_string(&response) {
            Ok(s) => s,
            Err(err) => format!(
                "{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":-32603,\"message\":\"serialization failed: {err}\"}},\"id\":null}}"
            ),
        };
        body.push('\n');
        if out.write_all(body.as_bytes()).is_err() {
            break;
        }
        let _ = out.flush();
    }
}
