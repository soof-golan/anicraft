//! Frame scripts in exported SWFs, read back with Ruffle's `swf` crate (an independent parser of both the
//! `ActionScript` 2 actions and the `ActionScript` 3 byte code).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::panic, clippy::too_many_lines)]

use anicraft_swf::export::{ExportOptions, export_display_lists};
use anicraft_swf::movie::Compression;
use anicraft_swf::script::{PlanFrame, ScriptPlan};
use anim_core::{Rgba8, Size};
use anim_display::{DisplayList, ImageStore};
use anim_script::{Version, parse};
use swf::avm2::types::Op;

fn empty_list() -> DisplayList {
    DisplayList { size: Size::new(100.0, 80.0), background: Some(Rgba8::WHITE), nodes: Vec::new() }
}

fn frame(label: Option<&str>, src: &str) -> PlanFrame {
    let script = parse(src);
    assert!(script.diagnostics.is_empty(), "{:?}", script.diagnostics);
    PlanFrame { label: label.map(str::to_owned), stmts: script.stmts }
}

fn export(version: Version, frames: Vec<PlanFrame>, loop_playback: bool) -> (Vec<u8>, Vec<String>) {
    let list = empty_list();
    let lists: Vec<&DisplayList> = frames.iter().map(|_| &list).collect();
    let mut options = ExportOptions::for_list(&list, 24.0).with_scripts(ScriptPlan { version, frames });
    options.movie.compression = Compression::None;
    options.loop_playback = loop_playback;
    let out = export_display_lists(lists, &ImageStore::default(), options).unwrap();
    (out.swf, out.report.script_notes)
}

fn avm1_actions(body: &[u8], version: u8) -> Vec<String> {
    let mut reader = swf::avm1::read::Reader::new(body, version);
    let mut out = Vec::new();
    while let Ok(action) = reader.read_action() {
        if matches!(action, swf::avm1::types::Action::End) {
            break;
        }
        out.push(format!("{action:?}"));
    }
    out
}

#[test]
fn as2_frame_scripts_become_do_action_tags_and_labels_are_written() {
    let frames =
        vec![frame(None, "stop();"), frame(Some("loop"), "var n = 1; if (n < 3) { n++; gotoAndPlay(\"loop\"); } else { gotoAndStop(2); }"), frame(None, "")];
    let (bytes, notes) = export(Version::As2, frames, true);
    assert!(notes.is_empty(), "{notes:?}");
    let buf = swf::decompress_swf(&bytes[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    assert!(!parsed.header.is_action_script_3());
    let mut per_frame: Vec<Vec<Vec<String>>> = vec![Vec::new()];
    let mut labels: Vec<String> = Vec::new();
    for tag in &parsed.tags {
        match tag {
            swf::Tag::DoAction(body) => per_frame.last_mut().unwrap().push(avm1_actions(body, parsed.header.version())),
            swf::Tag::FrameLabel(l) => labels.push(l.label.to_string_lossy(swf::UTF_8)),
            swf::Tag::ShowFrame => per_frame.push(Vec::new()),
            _ => {}
        }
    }
    assert_eq!(labels, vec!["loop"]);
    assert_eq!(per_frame[0], vec![vec!["Stop".to_owned()]]);
    let second = &per_frame[1][0];
    assert!(second.iter().any(|a| a.starts_with("GotoLabel")), "{second:?}");
    assert!(second.iter().any(|a| a.starts_with("GotoFrame")), "{second:?}");
    assert!(second.iter().any(|a| a == "Play") && second.iter().any(|a| a == "Stop"), "{second:?}");
    assert!(second.iter().any(|a| a.starts_with("If")) && second.iter().any(|a| a.starts_with("Jump")), "{second:?}");
    assert!(per_frame[2].is_empty());
}

#[test]
fn as2_closures_and_handlers_compile_to_functions() {
    let frames = vec![frame(None, "function go(n) { if (n > 2) { return n * 2; } return 0; } btn.onRelease = function () { go(5); gotoAndStop(\"end\"); };")];
    let (bytes, notes) = export(Version::As2, frames, true);
    assert!(notes.is_empty(), "{notes:?}");
    let buf = swf::decompress_swf(&bytes[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    let body = parsed.tags.iter().find_map(|t| if let swf::Tag::DoAction(b) = t { Some(*b) } else { None }).unwrap();
    let actions = avm1_actions(body, parsed.header.version());
    assert_eq!(actions.iter().filter(|a| a.starts_with("DefineFunction")).count(), 2, "{actions:?}");
    assert!(actions.iter().any(|a| a == "SetMember"), "{actions:?}");
}

#[test]
fn the_last_frame_stops_when_playback_does_not_loop_in_as2() {
    let (bytes, _) = export(Version::As2, vec![frame(None, "trace(\"hi\");"), frame(None, "")], false);
    let buf = swf::decompress_swf(&bytes[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    let actions: Vec<Vec<String>> =
        parsed.tags.iter().filter_map(|t| if let swf::Tag::DoAction(b) = t { Some(avm1_actions(b, parsed.header.version())) } else { None }).collect();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[1], vec!["Stop".to_owned()]);
}

fn decode_ops(code: &[u8]) -> Vec<Op> {
    let mut reader = swf::avm2::read::Reader::new(code);
    let mut ops = Vec::new();
    while let Ok(op) = reader.read_op() {
        ops.push(op);
    }
    ops
}

#[test]
fn as3_frame_scripts_become_a_document_class_with_add_frame_script() {
    let frames = vec![
        frame(
            None,
            "var count = 0; function go() { count += 1; gotoAndStop(\"end\"); } btn.addEventListener(MouseEvent.CLICK, function (e) { go(); }); stop();",
        ),
        frame(None, ""),
        frame(Some("end"), "trace(\"done\"); if (count > 0 && currentFrame < 5) { play(); } else { stop(); }"),
    ];
    let (bytes, notes) = export(Version::As3, frames, true);
    assert!(notes.is_empty(), "{notes:?}");
    let buf = swf::decompress_swf(&bytes[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    assert!(parsed.header.is_action_script_3());

    let mut abc_data = None;
    let mut symbols = Vec::new();
    for tag in &parsed.tags {
        match tag {
            swf::Tag::DoAbc2(d) => abc_data = Some(d.data),
            swf::Tag::SymbolClass(s) => symbols.extend(s.iter().map(|l| (l.id, l.class_name.to_string_lossy(swf::UTF_8)))),
            _ => {}
        }
    }
    assert_eq!(symbols, vec![(0, "Main".to_owned())]);
    let abc = swf::avm2::read::Reader::new(abc_data.unwrap()).read().unwrap();
    assert_eq!((abc.major_version, abc.minor_version), (46, 16));
    assert_eq!(abc.instances.len(), 1);
    assert_eq!(abc.classes.len(), 1);
    assert_eq!(abc.scripts.len(), 1);

    // Every method has a body and its byte code decodes to the end.
    for body in &abc.method_bodies {
        let ops = decode_ops(&body.code);
        assert!(!ops.is_empty());
        assert!(matches!(ops.last(), Some(Op::ReturnVoid | Op::ReturnValue)), "{ops:?}");
    }
    // The constructor registers the two frame scripts that exist (frames 1 and 3).
    let iinit = abc.instances[0].init_method.0 as usize;
    let ctor = abc.method_bodies.iter().find(|b| b.method.0 as usize == iinit).unwrap();
    let ops = decode_ops(&ctor.code);
    let registered = ops.iter().filter(|o| matches!(o, Op::CallPropVoid { num_args: 2, .. })).count();
    assert_eq!(registered, 2, "{ops:?}");
    assert!(ops.iter().any(|o| matches!(o, Op::ConstructSuper { num_args: 0 })));
}

#[test]
fn as3_stops_on_the_last_frame_from_its_script() {
    let (bytes, _) = export(Version::As3, vec![frame(None, "trace(1);"), frame(None, "")], false);
    let buf = swf::decompress_swf(&bytes[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    assert!(!parsed.tags.iter().any(|t| matches!(t, swf::Tag::DoAction(_))));
    let data = parsed.tags.iter().find_map(|t| if let swf::Tag::DoAbc2(d) = t { Some(d.data) } else { None }).unwrap();
    let abc = swf::avm2::read::Reader::new(data).read().unwrap();
    // Two frame scripts: frame 1 and the synthesised one that stops on frame 2.
    let ctor = abc.method_bodies.iter().find(|b| b.method.0 == abc.instances[0].init_method.0).unwrap();
    let registered = decode_ops(&ctor.code).iter().filter(|o| matches!(o, Op::CallPropVoid { num_args: 2, .. })).count();
    assert_eq!(registered, 2);
}

#[test]
fn documents_without_scripts_are_unchanged() {
    let list = empty_list();
    let plain = export_display_lists([&list, &list], &ImageStore::default(), ExportOptions::for_list(&list, 24.0)).unwrap();
    let with_plan = export_display_lists(
        [&list, &list],
        &ImageStore::default(),
        ExportOptions::for_list(&list, 24.0).with_scripts(ScriptPlan { version: Version::As3, frames: vec![PlanFrame::default(), PlanFrame::default()] }),
    )
    .unwrap();
    assert_eq!(plain.swf, with_plan.swf);
}

/// Four frames, each a different solid colour over the whole stage.
fn coloured_frames() -> (Vec<DisplayList>, [[u8; 3]; 4]) {
    use anim_core::kurbo::{self, Shape};
    use anim_core::{Affine, kurbo::BezPath};
    use anim_display::{Fill, FillRule, Node, Paint};
    let colours = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
    let lists = colours
        .iter()
        .map(|c| {
            let path: BezPath = kurbo::Rect::new(0.0, 0.0, 100.0, 80.0).to_path(0.1);
            let fill = Node::Fill(Fill { transform: Affine::IDENTITY, path, rule: FillRule::NonZero, paint: Paint::Solid(Rgba8::opaque(c[0], c[1], c[2])) });
            DisplayList { size: Size::new(100.0, 80.0), background: Some(Rgba8::WHITE), nodes: vec![fill] }
        })
        .collect();
    (lists, colours)
}

/// Exports the coloured frames with `scripts[i]` on frame `i` and renders `shown` frames with Ruffle, returning the
/// colour at the stage centre of each. `None` when `ANICRAFT_RUFFLE_EXPORTER` is not set.
fn ruffle_colours(name: &str, version: Version, scripts: [&str; 4], labels: [Option<&str>; 4], shown: usize) -> Option<Vec<[u8; 3]>> {
    let (lists, _) = coloured_frames();
    let frames: Vec<PlanFrame> = scripts.iter().zip(labels).map(|(s, l)| frame(l, s)).collect();
    let mut options = ExportOptions::for_list(&lists[0], 24.0).with_scripts(ScriptPlan { version, frames });
    options.movie.compression = Compression::None;
    let out = export_display_lists(lists.iter(), &ImageStore::default(), options).unwrap();
    let rendered = crate::common::render_swf_frames(name, &out.swf, shown)?;
    Some(rendered.iter().map(|r| r.at(50, 40)).collect())
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const YELLOW: [u8; 3] = [255, 255, 0];

/// Prediction (checked in Ruffle): without a script the movie plays red, green, blue, yellow, then loops to red. With
/// `stop();` on the third frame the playhead stays on blue from the third frame on.
#[test]
fn ruffle_stops_where_the_frame_script_says_in_both_languages() {
    for (version, name) in [(Version::As3, "script_stop_as3"), (Version::As2, "script_stop_as2")] {
        let Some(colours) = ruffle_colours(name, version, ["", "", "stop();", ""], [None; 4], 6) else { return };
        assert_eq!(colours[..3], [RED, GREEN, BLUE], "{name}");
        assert!(colours[3..].iter().all(|c| *c == BLUE), "{name}: {colours:?}");
    }
}

/// Prediction (checked in Ruffle): `gotoAndStop("end")` on the first frame lands on the frame labelled "end" (the
/// last, yellow) and stays there.
#[test]
fn ruffle_jumps_to_a_frame_label() {
    for (version, name) in [(Version::As3, "script_label_as3"), (Version::As2, "script_label_as2")] {
        let Some(colours) = ruffle_colours(name, version, ["gotoAndStop(\"end\");", "", "", ""], [None, None, None, Some("end")], 4) else { return };
        assert!(colours[1..].iter().all(|c| *c == YELLOW), "{name}: {colours:?}");
    }
}

/// Prediction (checked in Ruffle): a script with variables and a function: `n` counts up to 2 and the movie jumps to
/// frame 4 (yellow) from frame 2 only when the condition holds.
#[test]
fn ruffle_runs_variables_functions_and_conditions() {
    let script = "var n = 1; function bump() { n = n + 1; } bump(); if (n == 2 && !(n > 5)) { gotoAndStop(4); } else { gotoAndStop(1); }";
    for (version, name) in [(Version::As3, "script_logic_as3"), (Version::As2, "script_logic_as2")] {
        let Some(colours) = ruffle_colours(name, version, ["", script, "", ""], [None; 4], 4) else { return };
        assert_eq!(colours[0], RED, "{name}");
        assert!(colours[2..].iter().all(|c| *c == YELLOW), "{name}: {colours:?}");
    }
}

#[test]
fn unsupported_statements_are_reported_not_exported() {
    let script = parse("for (var i = 0; i < 3; i++) { trace(i); } stop();");
    let frames = vec![PlanFrame { label: None, stmts: script.stmts }];
    let (_, notes) = export(Version::As2, frames, true);
    assert_eq!(notes.len(), 1, "{notes:?}");
}
