//! The Align and Transform panels and the guides and grid, through the MCP commands the panels use.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::too_many_lines, clippy::float_cmp, reason = "test code")]

use crate::common;

use anicraft_tools::{Editor, ToolId, mcp_commands};
use anim_core::Rect;
use anim_session::{CommandRegistry, McpServer, Session};
use common::*;
use serde_json::{Value, json};

struct Client {
    server: McpServer,
    session: Session<Editor>,
    next_id: u64,
}

impl Client {
    fn new() -> Self {
        let mut editor = editor();
        editor.keep_history = false;
        let mut registry = CommandRegistry::with_history_commands();
        registry.register_all(mcp_commands::<Editor>()).unwrap();
        Self { server: McpServer::new("layout-test", "0.1.0"), session: Session::new(editor, registry), next_id: 1 }
    }

    fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        self.next_id += 1;
        let message = json!({ "jsonrpc": "2.0", "id": self.next_id, "method": "tools/call", "params": { "name": name, "arguments": arguments } });
        let reply = self.server.handle(&mut self.session, &message).expect("a response");
        let result = &reply["result"];
        let content = result.get("structuredContent").cloned().unwrap_or_else(|| result["content"][0]["text"].clone());
        (result["isError"].as_bool().unwrap(), content)
    }

    fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let (is_error, content) = self.call(name, arguments);
        assert!(!is_error, "{name} failed: {content}");
        content
    }

    fn bounds(&self) -> Vec<Rect> {
        let e = self.session.doc();
        (0..elements(e).len())
            .map(|i| e.with_context(|ctx| ctx.stage_bounds(&anicraft_tools::ElementRef::top_level(0, 0, i)).unwrap()))
            .collect()
    }
}

/// Three rectangles of different sizes, all selected.
fn three_objects() -> Client {
    let mut c = Client::new();
    {
        let e = c.session.doc_mut();
        add_object_rect(e, 10.0, 10.0, 30.0, 30.0, RED);
        add_object_rect(e, 50.0, 20.0, 100.0, 60.0, BLUE);
        add_object_rect(e, 200.0, 0.0, 210.0, 10.0, RED);
    }
    c.ok("select_all", json!({}));
    c
}

#[test]
fn align_to_the_selection_and_to_the_stage() {
    let mut c = three_objects();
    assert_eq!(c.ok("layout_align", json!({ "edge": "left" }))["moved"], 2, "the leftmost stays");
    for b in c.bounds() {
        assert!((b.x0 - 10.0).abs() < 0.01, "{b:?}");
    }
    c.ok("layout_align", json!({ "edge": "bottom", "to_stage": true }));
    for b in c.bounds() {
        assert!((b.y1 - 400.0).abs() < 0.01, "{b:?}");
    }
    assert_eq!(c.session.doc().state.selection.items.len(), 3, "the selection is kept");
}

#[test]
fn distribute_space_and_match_size() {
    let mut c = three_objects();
    c.ok("layout_distribute", json!({ "edge": "horizontal_center" }));
    let b = c.bounds();
    let centres: Vec<f64> = b.iter().map(|r| f64::midpoint(r.x0, r.x1)).collect();
    assert!(((centres[1] - centres[0]) - (centres[2] - centres[1])).abs() < 0.01, "{centres:?}");

    c.ok("layout_space", json!({ "axis": "horizontal", "gap": 7.0 }));
    let b = c.bounds();
    assert!((b[1].x0 - b[0].x1 - 7.0).abs() < 0.01 && (b[2].x0 - b[1].x1 - 7.0).abs() < 0.01, "{b:?}");

    c.ok("layout_match_size", json!({}));
    for r in c.bounds() {
        assert!((r.width() - 50.0).abs() < 0.01 && (r.height() - 40.0).abs() < 0.01, "{r:?}");
    }
    c.ok("layout_match_size", json!({ "to_stage": true, "height": false }));
    for r in c.bounds() {
        assert!((r.width() - 550.0).abs() < 0.01 && (r.height() - 40.0).abs() < 0.01, "{r:?}");
    }
    let (failed, _) = c.call("layout_distribute", json!({ "edge": "left", "bogus": 1 }));
    assert!(failed);
}

#[test]
fn a_layout_command_is_one_undo_step() {
    let mut c = three_objects();
    let before = c.session.doc().document.clone();
    c.ok("layout_align", json!({ "edge": "top" }));
    assert_ne!(c.session.doc().document, before);
    c.ok("edit_undo", json!({}));
    assert_eq!(c.session.doc().document, before);
}

#[test]
fn the_transform_panel_scales_rotates_and_copies() {
    let mut c = Client::new();
    add_object_rect(c.session.doc_mut(), 100.0, 100.0, 200.0, 150.0, RED);
    c.ok("select_all", json!({}));
    c.ok("transform_apply", json!({ "sx": 2.0, "sy": 1.0 }));
    let b = c.bounds()[0];
    assert!((b.width() - 200.0).abs() < 0.01 && (b.height() - 50.0).abs() < 0.01 && (f64::midpoint(b.x0, b.x1) - 150.0).abs() < 0.01, "{b:?}");
    c.ok("transform_apply", json!({ "rotate_degrees": 90.0, "copy": true }));
    let all = c.bounds();
    assert_eq!(all.len(), 2, "a copy was made");
    assert!((all[1].width() - 50.0).abs() < 0.01 && (all[1].height() - 200.0).abs() < 0.01, "{:?}", all[1]);
    assert!((all[0].width() - 200.0).abs() < 0.01, "the original did not turn");
    let (failed, _) = c.call("transform_apply", json!({ "sx": 0.0 }));
    assert!(failed);
}

#[test]
fn guides_and_the_grid_live_in_the_document() {
    let mut c = Client::new();
    c.ok("guides_add", json!({ "axis": "vertical", "position": 120.0 }));
    c.ok("guides_add", json!({ "axis": "horizontal", "position": 80.5 }));
    c.ok("guides_move", json!({ "axis": "vertical", "index": 0, "position": 130.0 }));
    c.ok("guides_set", json!({ "locked": true, "color": "#ff0000" }));
    c.ok("grid_set", json!({ "visible": true, "spacing": 25.0, "color": "#00ff00" }));
    let g = c.ok("guides_get", json!({}));
    assert_eq!(g["guides"]["vertical"], json!([130.0]));
    assert_eq!(g["guides"]["horizontal"], json!([80.5]));
    assert_eq!(g["guides"]["locked"], true);
    assert_eq!(g["grid"]["spacing_x"], 25.0);
    assert!(c.call("grid_set", json!({ "spacing": 0.0 })).0, "a zero spacing is refused");
    assert!(c.call("guides_remove", json!({ "axis": "vertical", "index": 4 })).0);

    // Snapping to them draws on the guide.
    c.ok("tools_options", json!({ "snap": { "guides": true } }));
    c.ok("guides_set", json!({ "locked": false }));
    c.ok("draw_rectangle", json!({ "x": 128.0, "y": 100.0, "width": 40.0, "height": 40.0, "fill": "#00f", "stroke": "none" }));
    assert!((c.bounds()[0].x0 - 130.0).abs() < 0.1, "{:?}", c.bounds()[0]);

    c.ok("guides_remove", json!({ "axis": "horizontal", "index": 0 }));
    c.ok("guides_clear", json!({}));
    let g = c.ok("guides_get", json!({}));
    assert_eq!(g["guides"]["vertical"], json!([]));
    c.ok("edit_undo", json!({}));
    let g = c.ok("guides_get", json!({}));
    assert_eq!(g["guides"]["horizontal"].as_array().unwrap().len(), 0, "undo went back one step only");
}

#[test]
fn dragging_the_selection_snaps_to_a_guide_and_shows_smart_guides() {
    let mut e = editor();
    add_object_rect(&mut e, 100.0, 100.0, 140.0, 140.0, RED);
    add_object_rect(&mut e, 300.0, 200.0, 340.0, 240.0, BLUE);
    let mut layout = anicraft_tools::guides::read(&e.document);
    layout.guides_x = vec![215.0];
    anicraft_tools::guides::write(&mut e.document, &layout);
    e.state.settings.snap.guides = true;
    e.state.settings.snap.smart_guides = true;
    e.select_tool(ToolId::Selection);
    // Press on the first object and drag 73 px right: its right edge would end 2 px short of the guide.
    e.send(&down(120.0, 120.0)).unwrap();
    e.send(&mv(150.0, 121.0)).unwrap();
    let mid = e.send(&mv(193.0, 121.0)).unwrap();
    let ghost = mid.overlay.iter().any(|o| matches!(o, anicraft_tools::Overlay::Rect { rect, role: anicraft_tools::Role::Preview } if (rect.x1 - 215.0).abs() < 0.01));
    assert!(ghost, "the preview box locks on to the guide: {:?}", mid.overlay);
    e.send(&up(193.0, 121.0)).unwrap();
    let r = e.with_context(|ctx| ctx.stage_bounds(&anicraft_tools::ElementRef::top_level(0, 0, 0)).unwrap());
    assert!((r.x1 - 215.0).abs() < 0.01, "{r:?}");
}
