//! A minimal MCP client over the stdio of a real `anicraft-cli mcp` process.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::arithmetic_side_effects, reason = "test code")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

/// A running server and the JSON-RPC conversation with it.
pub struct Client {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    next_id: u64,
}

impl Client {
    /// Starts `anicraft-cli mcp ARGS` and performs the MCP handshake.
    pub fn spawn(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_anicraft-cli"))
            .arg("mcp")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut client = Self { child, stdin, lines, next_id: 1 };
        let init =
            client.request("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }));
        assert_eq!(init["serverInfo"]["name"], "anicraft-cli");
        assert!(init["instructions"].as_str().unwrap().contains("1-based"));
        client.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        client
    }

    /// Sends a message.
    pub fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Sends a request and waits for its result (anything but a JSON-RPC error).
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(120)).expect("no reply from anicraft-cli mcp");
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout must carry only JSON-RPC, got {line:?}: {e}"));
            if message.get("id") == Some(&json!(id)) {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    /// Calls a tool; returns the structured content and whether it was an error.
    pub fn tool(&mut self, name: &str, args: Value) -> (Value, bool) {
        let result = self.request("tools/call", json!({ "name": name, "arguments": args }));
        (result["structuredContent"].clone(), result["isError"] == true)
    }

    /// Calls a tool that must succeed; returns its structured content.
    pub fn ok(&mut self, name: &str, args: Value) -> Value {
        let (value, is_error) = self.tool(name, args);
        assert!(!is_error, "{name} failed: {value}");
        value
    }

    /// Calls a tool that must succeed; returns the PNG bytes of its image content.
    pub fn images(&mut self, name: &str, args: Value) -> Vec<Vec<u8>> {
        use base64::Engine as _;
        let result = self.request("tools/call", json!({ "name": name, "arguments": args }));
        assert_ne!(result["isError"], true, "{result}");
        result["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "image")
            .map(|c| base64::engine::general_purpose::STANDARD.decode(c["data"].as_str().unwrap()).unwrap())
            .collect()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
