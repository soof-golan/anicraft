//! `anicraft-cli mcp --bridge` against the real editor window: an agent draws, the window shows it, and a screenshot
//! comes back through MCP.
//!
//! Needs a display and the `anicraft` binary next to `anicraft-cli` (`cargo build -p anicraft`), so it runs only when
//! `ANICRAFT_GUI_TESTS` is set, for example `DISPLAY=:1 ANICRAFT_GUI_TESTS=1`. `ANICRAFT_SHOT_DIR` names a folder to
//! keep the screenshots in.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "test code")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::client::Client;
use common::decode_png;
use serde_json::json;

/// The editor process; killed when the test ends.
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn app_binary() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_anicraft-cli")).with_file_name(format!("anicraft{}", std::env::consts::EXE_SUFFIX))
}

/// Starts the editor with a control server and returns it with the port it listens on and its token file.
fn start_app(dir: &Path) -> (App, String, PathBuf) {
    let token = dir.join("control.token");
    let child = Command::new(app_binary())
        .args(["--control-port", "0", "--token-file", token.to_str().unwrap(), "--size", "1280x800", "--root", dir.to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut app = App(child);
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(&token)
            && let Some(addr) = text.lines().nth(1)
            && let Some((_, port)) = addr.rsplit_once(':')
        {
            return (app, port.to_owned(), token);
        }
        assert!(app.0.try_wait().unwrap().is_none(), "the editor exited before it started its control server");
        assert!(started.elapsed() < Duration::from_secs(90), "the editor did not start its control server");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn keep(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("ANICRAFT_SHOT_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Path::new(&dir).join(name), bytes).unwrap();
    }
}

#[test]
fn an_agent_drives_the_running_editor_and_screenshots_it() {
    if std::env::var_os("ANICRAFT_GUI_TESTS").is_none() || !app_binary().is_file() {
        eprintln!("skipped: set ANICRAFT_GUI_TESTS=1 (with a display) and build `anicraft` to run the bridge test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (_app, port, token) = start_app(dir.path());
    let mut c = Client::spawn(&["--bridge", &port, "--token-file", token.to_str().unwrap()]);

    let status = c.ok("session_status", json!({}));
    assert_eq!(status["app"]["window"], true, "this is the window, not a headless session");
    c.ok("new_document", json!({}));
    c.ok("document_set_stage", json!({ "width": 480, "height": 270, "frame_rate": 12, "background": "#fff6e5" }));
    // The backdrop on the bottom layer, held for 24 frames.
    c.ok("draw_oval", json!({ "x": 300, "y": 40, "width": 120, "height": 120, "fill": "#2f7de1", "object_drawing": true }));
    c.ok("draw_path", json!({ "d": "M 40 250 L 110 170 L 180 250 Z", "fill": "#3aa655", "object_drawing": true }));
    c.ok("draw_text", json!({ "x": 200, "y": 190, "text": "שלום world مرحبا", "size": 26, "color": "#1b1b1b" }));
    c.ok("timeline_insert_frames", json!({ "layer": 0, "frame": 1, "count": 23 }));
    // The card that moves, alone on a layer above.
    c.ok("layer_add", json!({ "name": "card" }));
    c.ok("editor_goto", json!({ "layer": 0, "frame": 1 }));
    c.ok(
        "draw_rectangle",
        json!({ "x": 30, "y": 30, "width": 140, "height": 90, "fill": "#e85d3f", "stroke": { "color": "#222222", "width": 3 }, "object_drawing": true }),
    );
    c.ok("select_at", json!({ "x": 100, "y": 70 }));
    c.ok("modify_convert_to_symbol", json!({ "name": "Card", "kind": "movie_clip" }));
    c.ok("instance_add_filter", json!({ "kind": "drop_shadow", "distance": 6, "blur_x": 8, "blur_y": 8, "color": "#00000066" }));
    c.ok("timeline_insert_keyframe", json!({ "layer": 0, "frame": 24 }));
    c.ok("editor_goto", json!({ "frame": 24 }));
    c.ok("select_at", json!({ "x": 100, "y": 70 }));
    c.ok("transform_move", json!({ "dx": 120, "dy": 40 }));
    c.ok("timeline_create_classic_tween", json!({ "layer": 0, "frame": 1 }));
    c.ok("editor_goto", json!({ "frame": 12 }));
    assert_eq!(c.ok("query_validate", json!({}))["problems"], json!([]));

    // The window itself.
    let window = decode_png(&c.images("screenshot", json!({}))[0]);
    assert!(window.width >= 1000 && window.height >= 600, "{}x{}", window.width, window.height);
    let first = window.at(window.width / 2, window.height / 2);
    assert!((0..window.width).step_by(7).any(|x| window.at(x, window.height / 2) != first), "the window is not one flat colour");
    keep("anicraft-mcp-window.png", &c.images("screenshot", json!({}))[0]);

    // Panels.
    for panel in ["stage", "timeline", "library", "properties", "tools"] {
        let shot = c.images("screenshot", json!({ "panel": panel }));
        let png = decode_png(&shot[0]);
        assert!(png.width > 20 && png.height > 20, "{panel}");
        keep(&format!("anicraft-mcp-{panel}.png"), &shot[0]);
    }
    let (err, is_error) = c.tool("screenshot", json!({ "panel": "menubar-of-doom" }));
    assert!(is_error);
    assert_eq!(err["error"]["kind"], "invalid_params");

    // The same document through the headless render path, to compare with the window's stage.
    let frame = decode_png(&c.images("render_frame", json!({ "frame": 12 }))[0]);
    assert_eq!((frame.width, frame.height), (480, 270));
    keep("anicraft-mcp-render-frame-12.png", &c.images("render_frame", json!({ "frame": 12 }))[0]);

    // What the agent did is the person's undo history.
    let before = c.ok("journal", json!({}));
    assert!(before.to_string().contains("draw.rectangle") || before.to_string().contains("draw_rectangle"));
    c.ok("undo", json!({}));
}
