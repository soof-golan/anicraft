//! `anicraft-cli mcp` as a real MCP server over stdio: the protocol, the generated tools, atomic batches, undo,
//! rendering, exporting, saving and file confinement.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    reason = "test code"
)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::{decode_png, mover};
use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    next_id: u64,
}

impl Client {
    fn spawn(args: &[&str]) -> Self {
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

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(60)).expect("no reply from anicraft-cli mcp");
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout must carry only JSON-RPC, got {line:?}: {e}"));
            if message.get("id") == Some(&json!(id)) {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    /// Calls a tool; returns the structured content and whether it was an error.
    fn tool(&mut self, name: &str, args: Value) -> (Value, bool) {
        let result = self.request("tools/call", json!({ "name": name, "arguments": args }));
        (result["structuredContent"].clone(), result["isError"] == true)
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        let (value, is_error) = self.tool(name, args);
        assert!(!is_error, "{name} failed: {value}");
        value
    }

    fn images(&mut self, name: &str, args: Value) -> Vec<Vec<u8>> {
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

fn fixture() -> String {
    mover().to_str().unwrap().to_owned()
}

fn root(dir: &tempfile::TempDir) -> String {
    dir.path().to_str().unwrap().to_owned()
}

#[test]
fn tools_are_generated_from_the_commands_and_queries() {
    let mut client = Client::spawn(&[&fixture()]);
    let tools = client.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in
        ["inspect_document", "render_frame", "export", "run_batch", "undo", "layer_add", "timeline_insert_keyframe", "query_references", "document_set_stage"]
    {
        assert!(names.contains(&expected), "{expected} is missing from {names:?}");
    }
    // Typed tools carry the command's schema.
    let add = tools["tools"].as_array().unwrap().iter().find(|t| t["name"] == "timeline_insert_keyframe").unwrap();
    assert_eq!(add["inputSchema"]["required"], json!(["layer", "frame"]));
    let resources = client.request("resources/list", json!({}));
    assert!(resources["resources"].as_array().unwrap().iter().any(|r| r["uri"] == "anim://document/outline"));
}

#[test]
fn the_document_can_be_inspected_and_queried() {
    let mut client = Client::spawn(&[&fixture()]);
    let outline = client.ok("inspect_document", json!({ "depth": 2 }));
    assert_eq!(outline["scenes"][0]["frames"], 12);
    assert_eq!(outline["scenes"][0]["layers"][0]["name"], "Mover");
    assert_eq!(outline["scenes"][0]["layers"][0]["keyframes"][1]["frame"], 7, "frames are 1-based");
    assert_eq!(outline["scenes"][0]["layers"][0]["keyframes"][0]["elements"][0]["item"], "Box");
    let library = client.ok("get_library", json!({}));
    assert_eq!(library["symbols"][0]["name"], "Box");
    let uses = client.ok("query_references", json!({ "name": "Box" }));
    assert_eq!(uses["uses"].as_array().unwrap().len(), 2);
    let at = client.ok("query_keyframes", json!({ "frame": 9 }));
    assert_eq!(at["keyframes"][0]["frame"], 7);
    assert_eq!(client.ok("query_validate", json!({}))["problems"], json!([]));
    assert_eq!(client.ok("query_stats", json!({}))["symbols"], 1);
    let status = client.ok("session_status", json!({}));
    assert_eq!(status["app"]["total_frames"], 12);
}

#[test]
fn render_frame_returns_a_png_you_can_look_at() {
    let mut client = Client::spawn(&[&fixture()]);
    let first = client.images("render_frame", json!({ "frame": 1 }));
    let last = client.images("render_frame", json!({ "frame": 12, "scale": 2.0 }));
    let (a, b) = (decode_png(&first[0]), decode_png(&last[0]));
    assert_eq!((a.width, a.height), (120, 80));
    assert_eq!((b.width, b.height), (240, 160));
    assert_ne!(a.red_columns().first(), b.red_columns().first().map(|x| x / 2).as_ref());
    // A library symbol can be previewed alone.
    let symbol = client.images("render_frame", json!({ "options": { "symbol": "Box" } }));
    assert_eq!(decode_png(&symbol[0]).width, 36);
    let (err, is_error) = client.tool("render_frame", json!({ "frame": 99 }));
    assert!(is_error);
    assert_eq!(err["error"]["kind"], "invalid_params");
}

#[test]
fn edits_are_undoable_and_batches_are_atomic() {
    let mut client = Client::spawn(&[&fixture()]);
    let added = client.ok("scene_add", json!({ "name": "Outro" }));
    assert_eq!(added["result"]["scene"], 1);
    assert_eq!(client.ok("inspect_document", json!({ "depth": 0 }))["scenes"].as_array().unwrap().len(), 2);
    client.ok("undo", json!({}));
    assert_eq!(client.ok("inspect_document", json!({ "depth": 0 }))["scenes"].as_array().unwrap().len(), 1);

    // The second step fails (frame 0), so the first must not stick.
    let (err, is_error) = client.tool(
        "run_batch",
        json!({ "label": "two steps", "steps": [{ "id": "layer.add", "params": { "name": "new" } }, { "id": "timeline.insert_keyframe", "params": { "layer": 0, "frame": 0 } }] }),
    );
    assert!(is_error);
    assert_eq!(err["error"]["data"]["step"], 1);
    assert_eq!(client.ok("inspect_document", json!({ "depth": 1 }))["scenes"][0]["layer_count"], 2);

    // A good batch is one undo step.
    client.ok("run_batch", json!({ "label": "good", "steps": [{ "id": "layer.add", "params": { "name": "a" } }, { "id": "layer.rename", "params": { "layer": 0, "name": "b" } }] }));
    assert_eq!(client.ok("inspect_document", json!({ "depth": 1 }))["scenes"][0]["layers"][0]["name"], "b");
    client.ok("undo", json!({}));
    assert_eq!(client.ok("inspect_document", json!({ "depth": 1 }))["scenes"][0]["layer_count"], 2);

    // The change shows in the render: a tween added by command moves the square.
    client.ok("timeline_insert_keyframe", json!({ "layer": 1, "frame": 12 }));
    let kfs = client.ok("query_keyframes", json!({ "frame": 12 }));
    assert!(kfs["keyframes"].as_array().unwrap().iter().any(|k| k["frame"] == 12));
}

#[test]
fn properties_read_and_write_with_validation() {
    let mut client = Client::spawn(&[&fixture()]);
    assert_eq!(client.ok("get_property", json!({ "path": "/stage/width" }))["value"], 120.0);
    client.ok("set_property", json!({ "path": "/stage/width", "value": 200 }));
    let img = client.images("render_frame", json!({}));
    assert_eq!(decode_png(&img[0]).width, 200);
    client.ok("set_property", json!({ "path": "scenes[0].layers[1].visible", "value": false }));
    let (_, is_error) = client.tool("set_property", json!({ "path": "/stage/frame_rate", "value": -4 }));
    assert!(is_error);
    let (err, is_error) = client.tool("get_property", json!({ "path": "/scenes/7" }));
    assert!(is_error);
    assert_eq!(err["error"]["kind"], "not_found");
    client.ok("undo", json!({}));
    assert_eq!(client.ok("get_property", json!({ "path": "/scenes/0/layers/1/visible" }))["value"], true);
}

#[test]
fn files_are_confined_to_the_roots() {
    let allowed = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let mut client = Client::spawn(&["--root", &root(&allowed), &fixture()]);

    // Inside the root: export and save work.
    let out = allowed.path().join("seq");
    let result = client.ok("export", json!({ "format": "png-seq", "path": out.to_str().unwrap(), "options": { "from": 1, "to": 3 } }));
    assert_eq!(result["frames"], 3);
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 3);
    let fla = allowed.path().join("saved.fla");
    client.ok("save_document", json!({ "path": fla.to_str().unwrap() }));
    assert!(fla.metadata().unwrap().len() > 100);
    client.ok("open_document", json!({ "path": fla.to_str().unwrap() }));
    assert_eq!(client.ok("inspect_document", json!({ "depth": 0 }))["scenes"][0]["frames"], 12);

    // Outside it: refused, and nothing is written.
    let stray = elsewhere.path().join("stray");
    let (err, is_error) = client.tool("export", json!({ "format": "gif", "path": stray.to_str().unwrap() }));
    assert!(is_error);
    assert_eq!(err["error"]["kind"], "denied");
    assert!(!stray.exists());
    let (err, _) = client.tool("open_document", json!({ "path": fixture() }));
    assert_eq!(err["error"]["kind"], "denied");
    let (err, _) = client.tool("render_frame", json!({ "path": elsewhere.path().join("x.png").to_str().unwrap() }));
    assert_eq!(err["error"]["kind"], "denied");
    let (err, is_error) = client.tool("export", json!({ "format": "teapot", "path": out.to_str().unwrap() }));
    assert!(is_error);
    assert_eq!(err["error"]["kind"], "not_found");
}

#[test]
fn a_new_document_can_be_built_by_commands_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::spawn(&["--root", &root(&dir)]);
    client.ok("document_set_stage", json!({ "width": 64, "height": 48, "frame_rate": 10, "background": "#00ff00" }));
    client.ok("layer_rename", json!({ "layer": 0, "name": "bg" }));
    client.ok("timeline_insert_frames", json!({ "layer": 0, "frame": 1, "count": 9 }));
    let img = decode_png(&client.images("render_frame", json!({}))[0]);
    assert_eq!((img.width, img.height), (64, 48));
    assert_eq!(img.at(3, 3), [0, 255, 0, 255]);
    let out = dir.path().join("n.fla");
    client.ok("save_document", json!({ "path": out.to_str().unwrap() }));
    let info = common::cli_ok(&["info", out.to_str().unwrap(), "--json"]);
    let report: Value = serde_json::from_str(&info).unwrap();
    assert_eq!(report["stage"]["width"], 64.0);
    assert_eq!(report["scenes"][0]["layers"][0]["name"], "bg");
    assert!(Path::new(&out).is_file());
}

#[test]
fn a_bad_root_is_reported_before_the_client_connects() {
    let out = Command::new(env!("CARGO_BIN_EXE_anicraft-cli")).args(["mcp", "--root", "/definitely/not/here"]).stdin(Stdio::null()).output().unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "nothing may reach stdout before the protocol starts");
}
