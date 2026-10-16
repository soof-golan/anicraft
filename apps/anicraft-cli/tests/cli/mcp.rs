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

use crate::common;

use std::path::Path;
use std::process::{Command, Stdio};

use common::client::Client;
use common::{decode_png, mover};
use serde_json::{Value, json};

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

#[test]
fn bridge_options_are_validated_before_the_protocol_starts() {
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_anicraft-cli")).arg("mcp").args(args).env_remove("ANIM_MCP_TOKEN").stdin(Stdio::null()).output().unwrap()
    };
    // Nothing listens, and there is no token: a clear error, nothing on stdout.
    let out = run(&["--bridge", "1"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "nothing may reach stdout");
    let message = String::from_utf8_lossy(&out.stderr);
    assert!(message.contains("--token-file") || message.contains("could not"), "{message}");
    // A bridge drives the editor's document: it cannot be combined with a headless file or roots.
    for extra in [["--file", "x.fla"], ["--root", "/tmp"], ["--read-root", "/tmp"], ["--write-root", "/tmp"]] {
        let out = run(&["--bridge", "1", extra[0], extra[1]]);
        assert!(!out.status.success(), "{extra:?}");
        assert!(out.stdout.is_empty(), "nothing may reach stdout");
    }
    assert!(!run(&["--token-file", "/tmp/x"]).status.success(), "--token-file needs --bridge");
}

fn tiny_png(path: &Path) {
    anim_testkit::write_png(path, 4, 4, &[200_u8; 64]).unwrap();
}

#[test]
fn every_editor_command_is_a_tool_with_a_schema_and_a_description() {
    let mut client = Client::spawn(&[]);
    let tools = client.request("tools/list", json!({}));
    let list = tools["tools"].as_array().unwrap();
    let names: Vec<&str> = list.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        // files, queries, rendering, export
        "new_document",
        "open_document",
        "save_document",
        "inspect_document",
        "get_library",
        "get_property",
        "set_property",
        "render_frame",
        "screenshot",
        "export",
        "list_export_formats",
        "query_timeline",
        "query_element",
        "query_stats",
        "undo",
        "redo",
        "run_batch",
        // document, scenes, layers
        "document_set_stage",
        "scene_add",
        "layer_add",
        "layer_duplicate",
        "layer_set_state",
        "layer_move",
        // timeline
        "timeline_insert_keyframe",
        "timeline_create_classic_tween",
        "timeline_create_shape_tween",
        "timeline_set_tween_options",
        "timeline_set_ease",
        "timeline_set_sound",
        "timeline_set_script",
        "timeline_set_component_param",
        "query_frame_script",
        "query_test_movie",
        "timeline_move_keyframe",
        "timeline_set_label",
        // tools as commands
        "draw_rectangle",
        "draw_oval",
        "draw_line",
        "draw_path",
        "draw_freehand",
        "draw_text",
        "select_at",
        "select_rect",
        "select_all",
        "transform_move",
        "transform_scale",
        "transform_rotate",
        "transform_skew",
        "transform_matrix",
        "paint_bucket",
        "paint_ink",
        "edit_enter",
        "edit_exit",
        "editor_goto",
        // objects, symbols, library
        "modify_group",
        "modify_convert_to_symbol",
        "modify_arrange",
        "modify_flip",
        "insert_symbol",
        "instance_set_color_effect",
        "instance_add_filter",
        "instance_clear_filters",
        "instance_set_options",
        "text_set_style",
        "stage_add_symbol_instance",
        "library_import",
        "library_rename_item",
    ] {
        assert!(names.contains(&expected), "{expected} is missing");
    }
    for tool in list {
        let name = tool["name"].as_str().unwrap();
        assert!(tool["description"].as_str().is_some_and(|d| d.len() > 15), "{name} needs a real description");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
    }
    let png_tool = list.iter().find(|t| t["name"] == "render_frame").unwrap();
    assert_eq!(png_tool["annotations"]["readOnlyHint"], true);
    // Export lists every format.
    let formats = client.ok("list_export_formats", json!({}));
    let ids: Vec<&str> = formats["formats"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    for id in ["png-seq", "gif", "mp4", "mov", "svg", "svg-anim", "spritesheet", "swf"] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
}

#[test]
fn a_whole_animation_is_built_looked_at_exported_saved_and_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Client::spawn(&["--read-root", &root(&dir), "--write-root", &root(&dir)]);
    c.ok("new_document", json!({}));
    c.ok("document_set_stage", json!({ "width": 160, "height": 100, "frame_rate": 12, "background": "#ffffff" }));

    // Static artwork on the bottom layer, drawn with the tools: a rectangle, an oval, a triangle and mixed-direction text.
    c.ok("draw_rectangle", json!({ "x": 10, "y": 10, "width": 50, "height": 40, "fill": "#ff0000", "object_drawing": true }));
    c.ok("draw_oval", json!({ "x": 90, "y": 10, "width": 50, "height": 40, "fill": "#0000ff", "object_drawing": true }));
    c.ok("draw_path", json!({ "d": "M 10 95 L 60 70 L 110 95 Z", "fill": "#00aa00", "object_drawing": true }));
    let hebrew = "שלום world مرحبا";
    c.ok("draw_text", json!({ "x": 70, "y": 60, "text": hebrew, "size": 20, "color": "#000000" }));
    assert_eq!(c.ok("stage_elements", json!({}))["result"]["elements"].as_array().unwrap().len(), 4);

    // Right-to-left text is data, not an afterthought: it reads back as typed and can be forced to rtl.
    let table = c.ok("query_timeline", json!({}));
    let keyframe = &table["layers"][0]["keyframes"][0];
    let text_index = keyframe["elements"].as_array().unwrap().iter().find(|e| e["kind"] == "text").unwrap()["index"].as_u64().unwrap();
    let element = c.ok("query_element", json!({ "layer": 0, "frame": 1, "index": text_index }));
    assert_eq!(element["element"]["Text"]["text_runs"][0]["characters"], hebrew);
    c.ok("text_set_style", json!({ "layer": 0, "frame": 1, "index": text_index, "direction": "rtl", "align": "right", "size": 18 }));
    let element = c.ok("query_element", json!({ "layer": 0, "frame": 1, "index": text_index }));
    assert_eq!(element["element"]["Text"]["text_runs"][0]["direction"], "RightToLeft");
    assert_eq!(element["element"]["Text"]["text_runs"][0]["alignment"], "Right");
    assert!(c.tool("text_set_style", json!({ "layer": 0, "frame": 1, "index": 0, "direction": "diagonal" })).1);

    // A moving symbol on a new top layer: convert, colour, filter, keyframe, move, tween.
    c.ok("layer_add", json!({ "name": "mover" }));
    c.ok("editor_goto", json!({ "layer": 0, "frame": 1 }));
    c.ok("draw_rectangle", json!({ "x": 10, "y": 56, "width": 20, "height": 20, "fill": "#ff8800", "object_drawing": true }));
    c.ok("modify_convert_to_symbol", json!({ "name": "Block", "kind": "movie_clip" }));
    c.ok("instance_set_color_effect", json!({ "effect": "alpha", "value": 90 }));
    c.ok("instance_add_filter", json!({ "kind": "drop_shadow", "distance": 3, "blur_x": 3, "blur_y": 3, "color": "#00000080" }));
    c.ok("instance_set_options", json!({ "blend_mode": "normal", "name": "block" }));
    c.ok("timeline_insert_keyframe", json!({ "layer": 0, "frame": 12 }));
    c.ok("editor_goto", json!({ "frame": 12 }));
    c.ok("select_at", json!({ "x": 20, "y": 66 }));
    c.ok("transform_move", json!({ "dx": 120, "dy": 0 }));
    c.ok("timeline_create_classic_tween", json!({ "layer": 0, "frame": 1 }));
    c.ok("timeline_set_tween_options", json!({ "layer": 0, "frame": 1, "rotate": "clockwise", "rotate_times": 1 }));
    c.ok("timeline_set_label", json!({ "layer": 0, "frame": 1, "label": "run" }));
    assert_eq!(c.ok("query_validate", json!({}))["problems"], json!([]));
    let table = c.ok("query_timeline", json!({}));
    assert_eq!(table["frames"], 12);
    assert_eq!(table["layers"][0]["name"], "mover");
    assert_eq!(table["layers"][0]["keyframes"][0]["tween"], "Classic");
    assert_eq!(table["layers"][0]["keyframes"][0]["rotate"], "Clockwise");
    let symbol = c.ok("query_timeline", json!({ "symbol": "Block" }));
    assert_eq!(symbol["layers"][0]["keyframes"][0]["elements"].as_array().unwrap().len(), 1);

    // Look at it: the images are real PNGs that differ over time, and the text left ink on the page.
    let first = decode_png(&c.images("render_frame", json!({ "frame": 1 }))[0]);
    let last = decode_png(&c.images("render_frame", json!({ "frame": 12 }))[0]);
    let middle = decode_png(&c.images("render_frame", json!({ "frame": 6 }))[0]);
    assert_eq!((first.width, first.height), (160, 100));
    assert_ne!(first.rgba, last.rgba);
    assert_ne!(first.rgba, middle.rgba);
    // The orange block tweens from x=10..30 to x=130..150 (shadow and 90 % alpha aside, it stays orange-ish).
    let orange_columns = |p: &common::Pixels| -> Vec<u32> {
        (0..p.width).filter(|&x| (56..76).any(|y| matches!(p.at(x, y), [r, g, b, _] if r > 200 && (100..200).contains(&g) && b < 120))).collect()
    };
    let (a, m, z) = (orange_columns(&first), orange_columns(&middle), orange_columns(&last));
    assert!(a.first().is_some_and(|x| *x < 20), "frame 1: {a:?}");
    assert!(m.first().is_some_and(|x| (50..110).contains(x)), "frame 6 is part way: {m:?}");
    assert!(z.first().is_some_and(|x| *x > 110), "frame 12: {z:?}");
    let ink = (70..155).flat_map(|x| (60..95).map(move |y| (x, y))).filter(|&(x, y)| first.at(x, y)[..3].iter().all(|v| *v < 100)).count();
    assert!(ink > 30, "the Hebrew and Arabic text should be drawn ({ink} dark pixels)");

    // Import an image, and see it in the library and on the stage.
    let png_path = dir.path().join("dot.png");
    tiny_png(&png_path);
    let imported = c.ok("library_import", json!({ "path": png_path.to_str().unwrap(), "place": true }));
    assert_eq!(imported["result"]["name"], "dot.png", "{imported}");
    assert_eq!(c.ok("get_library", json!({}))["bitmaps"][0], "dot.png");

    // Export a GIF, save, reopen and find the same animation.
    let gif = dir.path().join("anim.gif");
    let exported = c.ok("export", json!({ "format": "gif", "path": gif.to_str().unwrap(), "options": { "from": 1, "to": 12 } }));
    assert_eq!(exported["frames"], 12);
    assert!(std::fs::read(&gif).unwrap().starts_with(b"GIF8"));
    let fla = dir.path().join("anim.fla");
    let before_save = decode_png(&c.images("render_frame", json!({ "frame": 12 }))[0]);
    c.ok("save_document", json!({ "path": fla.to_str().unwrap() }));
    c.ok("new_document", json!({}));
    assert_eq!(c.ok("inspect_document", json!({ "depth": 0 }))["scenes"][0]["frames"], 1);
    c.ok("open_document", json!({ "path": fla.to_str().unwrap() }));
    let reopened = c.ok("query_timeline", json!({}));
    assert_eq!(reopened["frames"], 12);
    assert_eq!(reopened["layers"][0]["keyframes"][0]["tween"], "Classic");
    let text = c.ok("query_element", json!({ "layer": 1, "frame": 1, "index": text_index }));
    assert_eq!(text["element"]["Text"]["text_runs"][0]["characters"], hebrew);
    // (XFL has no slot for a forced direction; `auto` resolves Hebrew and Arabic runs right-to-left by their letters.)
    assert_eq!(text["element"]["Text"]["text_runs"][0]["alignment"], "Right");
    let again = decode_png(&c.images("render_frame", json!({ "frame": 12 }))[0]);
    assert_eq!(again.rgba, before_save.rgba, "the reopened file renders like the document that was saved");
    // The saved file is a valid Animate document for the one-shot commands too.
    let info = common::cli_ok(&["info", fla.to_str().unwrap(), "--json"]);
    assert!(info.contains("Block"));
}

/// Prediction: a 10-frame movie whose last frame says `stop()` is on frame 10, stopped, after 30 ticks, and a script
/// that jumps to the label "mid" lands on frame 5 and prints what the script wrote.
#[test]
fn frame_scripts_are_stored_tested_and_exported_to_swf() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Client::spawn(&["--read-root", &root(&dir), "--write-root", &root(&dir)]);
    c.ok("new_document", json!({}));
    for frame in [5, 10] {
        c.ok("timeline_insert_keyframe", json!({ "layer": 0, "frame": frame }));
    }
    c.ok("timeline_set_label", json!({ "layer": 0, "frame": 5, "label": "mid" }));
    c.ok("timeline_set_script", json!({ "layer": 0, "frame": 10, "script": "trace(\"done\");\nstop();" }));
    let script = c.ok("query_frame_script", json!({ "layer": 0, "frame": 10 }));
    assert_eq!(script["script"], "trace(\"done\");\nstop();");
    assert_eq!(script["diagnostics"].as_array().unwrap().len(), 0);

    let run = c.ok("query_test_movie", json!({ "steps": [{ "tick": 30 }] }));
    assert_eq!(run["frame"], 10, "{run}");
    assert_eq!(run["playing"], false);
    assert_eq!(run["output"], json!(["done"]));

    // Frame 1 jumps to the label: "mid" is frame 5, where the movie stops.
    c.ok("timeline_set_script", json!({ "layer": 0, "frame": 1, "script": "gotoAndStop(\"mid\");" }));
    let run = c.ok("query_test_movie", json!({ "steps": [{ "tick": 1 }] }));
    assert_eq!(run["frame"], 5, "{run}");
    assert_eq!(run["playing"], false);

    // Statements outside the supported subset are reported, not hidden.
    c.ok("timeline_set_script", json!({ "layer": 0, "frame": 2, "script": "for (var i = 0; i < 3; i++) { trace(i); }" }));
    let script = c.ok("query_frame_script", json!({ "layer": 0, "frame": 2 }));
    assert_eq!(script["diagnostics"].as_array().unwrap().len(), 1, "{script}");

    // The SWF export carries the scripts: an ActionScript 3 document gets a DoABC tag and a SymbolClass for `Main`.
    let swf = dir.path().join("out.swf");
    let result = c.ok("export", json!({ "format": "swf", "path": swf.to_str().unwrap() }));
    assert_eq!(result["frames"], 10);
    let notes = result["notes"].as_array().unwrap();
    assert!(notes.iter().any(|n| n.as_str().is_some_and(|n| n.starts_with("swf script:"))), "{notes:?}");
    let bytes = std::fs::read(&swf).unwrap();
    assert_eq!(&bytes[..3], b"CWS");
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&bytes[8..]), &mut body).unwrap();
    assert!(body.windows(5).any(|w| w == b"Main\0"), "the SymbolClass tag names the document class");
}
