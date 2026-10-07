//! Integration test of the stdio MCP transport (`searxng-rs mcp`).
//!
//! In stdio mode stdout IS the JSON-RPC channel: the MCP spec forbids writing
//! anything else there. Log lines on stdout break strict MCP clients, so every
//! line the server prints to stdout must parse as a JSON-RPC message.
//!
//! The test is offline: it calls `engine_status` and `search` with an engine
//! name that does not exist, so no outgoing HTTP request is made.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const NONEXISTENT_ENGINE: &str = "no_such_engine";
/// `debug = true` switches logging to TRACE: maximum log volume for the check.
const DEBUG_CONFIG: &str = "[general]\ndebug = true\n";

struct StdioServer {
    child: Child,
    stdin: ChildStdin,
    stdout_lines: Receiver<String>,
}

impl StdioServer {
    fn start(work_dir: &std::path::Path) -> Self {
        let config_path = work_dir.join("searxng-rs.toml");
        std::fs::write(&config_path, DEBUG_CONFIG).expect("write test config");

        let mut child = Command::new(env!("CARGO_BIN_EXE_searxng-rs"))
            .arg("--config")
            .arg(&config_path)
            .arg("mcp")
            // The server creates `searxng-rs.log` in its working directory.
            .current_dir(work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn searxng-rs mcp");

        let stdin = child.stdin.take().expect("child stdin");
        let stdout = child.stdout.take().expect("child stdout");
        let (sender, stdout_lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        Self { child, stdin, stdout_lines }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").expect("write to server stdin");
        self.stdin.flush().expect("flush server stdin");
    }

    /// Read stdout until the response with `id` arrives. Panics on any stdout
    /// line that is not a JSON-RPC message.
    fn receive_response(&self, id: u64) -> Value {
        loop {
            let line = self
                .stdout_lines
                .recv_timeout(RESPONSE_TIMEOUT)
                .unwrap_or_else(|_| panic!("no response with id={id} within {RESPONSE_TIMEOUT:?}"));
            let message: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("non-JSON line on MCP stdout ({e}): {line:?}"));
            assert_eq!(message["jsonrpc"], "2.0", "not a JSON-RPC message: {line}");
            if message["id"] == id {
                return message;
            }
        }
    }
}

impl Drop for StdioServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool response without text content: {response}"))
}

#[test]
fn stdio_stdout_carries_only_json_rpc() {
    let work_dir = tempfile::tempdir().expect("temp dir");
    let mut server = StdioServer::start(work_dir.path());

    server.send(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "stdio-test", "version": "1"}
        }
    }));
    let init = server.receive_response(1);
    assert!(init["result"]["serverInfo"].is_object(), "bad initialize: {init}");

    server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    server.send(&json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "engine_status", "arguments": {"engine": "bing"}}
    }));
    let status = server.receive_response(2);
    assert_eq!(tool_text(&status), "bing: enabled (general)");

    server.send(&json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "search", "arguments": {"query": "rust", "engines": NONEXISTENT_ENGINE}}
    }));
    let search = server.receive_response(3);
    assert_eq!(
        tool_text(&search),
        format!("no results; unknown engine: {NONEXISTENT_ENGINE}")
    );
}
