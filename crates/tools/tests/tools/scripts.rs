//! JSON scripts, serde round trips and hostile input.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::float_cmp,
    clippy::manual_assert_eq,
    clippy::assert_is_empty,
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    clippy::disallowed_types
)]

use crate::common;

use anicraft_tools::{
    Editor, Key, KeyEvent, Modifiers, PointerEvent, PointerKind, ScriptReport, ToolCommand, ToolEvent, ToolId, ToolScript, catalog, run_script,
};
use anim_core::Point;
use common::*;
use proptest::prelude::*;

#[test]
fn a_json_script_draws_a_rectangle() {
    let json = r#"{
        "tool": "rectangle",
        "events": [
            {"type":"pointer","kind":"down","pos":{"x":10,"y":10}},
            {"type":"pointer","kind":"move","pos":{"x":90,"y":60}},
            {"type":"pointer","kind":"up","pos":{"x":90,"y":60}}
        ]
    }"#;
    let script: ToolScript = serde_json::from_str(json).unwrap();
    let mut e = editor();
    let report = run_script(&mut e, &script);
    assert_eq!(report.error, None);
    assert_eq!(report.events_run, 3);
    assert_eq!(report.commands_applied, 1);
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.active_tool, ToolId::Rectangle);
    assert_eq!(elements(&e).len(), 1);
    // The report itself is JSON.
    let text = serde_json::to_string(&report).unwrap();
    let back: ScriptReport = serde_json::from_str(&text).unwrap();
    assert_eq!(back, report);
}

#[test]
fn script_builders_and_failures() {
    let mut e = editor();
    let report = run_script(&mut e, &ToolScript::drag(ToolId::Oval, (10.0, 10.0), (60.0, 40.0)));
    assert_eq!(report.commands_applied, 1);
    let report = run_script(&mut e, &ToolScript::click(ToolId::Selection, 30.0, 25.0));
    assert_eq!(report.selection.items.len(), 1);
    let points = [Point::new(0.0, 0.0), Point::new(10.0, 10.0), Point::new(20.0, 0.0), Point::new(30.0, 10.0)];
    let report = run_script(&mut e, &ToolScript::stroke(ToolId::Pencil, &points));
    assert_eq!(report.error, None);

    // A layer that does not exist is not drawn on.
    let mut e = editor();
    e.state.layer = 7;
    let report = run_script(&mut e, &ToolScript::drag(ToolId::Rectangle, (0.0, 0.0), (50.0, 50.0)));
    assert_eq!(report.commands_applied, 0);
    assert_eq!(elements(&e).len(), 0);
}

#[test]
fn a_failing_command_rolls_back_the_whole_event_and_stops_the_script() {
    let mut e = editor();
    add_object_rect(&mut e, 0.0, 0.0, 100.0, 100.0, RED);
    let before = e.document.clone();
    // Alt-drag emits [DuplicateSelection, TransformSelection]; an infinite release position makes the second
    // command fail after the first one was applied.
    let script = ToolScript {
        tool: Some(ToolId::Selection),
        events: vec![
            PointerEvent::down(50.0, 50.0).with_modifiers(Modifiers::ALT).into(),
            PointerEvent::drag(80.0, 50.0).with_modifiers(Modifiers::ALT).into(),
            PointerEvent::up(f64::INFINITY, 50.0).with_modifiers(Modifiers::ALT).into(),
            PointerEvent::down(10.0, 10.0).into(),
        ],
    };
    let report = run_script(&mut e, &script);
    let error = report.error.expect("the script fails");
    assert!(error.starts_with("event 2:"), "{error}");
    assert_eq!(report.events_run, 2, "the failing event and everything after it did not run");
    assert_eq!(e.document, before, "the duplicate was rolled back");
    assert_eq!(e.undo_depth(), 0);
}

#[test]
fn the_tool_catalog_lists_every_tool() {
    let tools = catalog();
    assert_eq!(tools.len(), ToolId::ALL.len());
    assert!(tools.iter().all(|t| !t.description.is_empty() && !t.label.is_empty()));
    let json = serde_json::to_string(&tools).unwrap();
    assert!(json.contains("\"free_transform\"") && json.contains("\"paint_bucket\""));
}

#[test]
fn every_command_and_event_survives_a_json_round_trip() {
    let mut e = editor();
    let mut seen = Vec::new();
    let scripts = [
        ToolScript::drag(ToolId::Rectangle, (10.0, 10.0), (110.0, 70.0)),
        ToolScript::drag(ToolId::Selection, (50.0, 50.0), (80.0, 90.0)),
        ToolScript::click(ToolId::Text, 200.0, 200.0),
        ToolScript::click(ToolId::Zoom, 100.0, 100.0),
        ToolScript::drag(ToolId::Hand, (10.0, 10.0), (50.0, 50.0)),
        ToolScript::drag(ToolId::Eraser, (0.0, 50.0), (200.0, 50.0)),
        ToolScript::click(ToolId::Eyedropper, 30.0, 30.0),
        ToolScript::click(ToolId::PaintBucket, 30.0, 30.0),
    ];
    for s in &scripts {
        let json = serde_json::to_string(s).unwrap();
        let again: ToolScript = serde_json::from_str(&json).unwrap();
        assert_eq!(&again, s);
        let report = run_script(&mut e, s);
        for step in &report.steps {
            seen.extend(step.commands.clone());
        }
    }
    assert!(seen.len() >= 5);
    for c in &seen {
        let json = serde_json::to_string(c).unwrap();
        let back: ToolCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, c, "{json}");
    }
    // The document and state serialise too (the MCP server returns them).
    let doc = serde_json::to_string(&e.document).unwrap();
    let back: anim_scene::Document = serde_json::from_str(&doc).unwrap();
    assert_eq!(back, e.document);
    serde_json::to_string(&e.state).unwrap();
}

#[test]
fn hostile_coordinates_never_panic() {
    let weird = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.0e300, -1.0e300, 0.0, -0.0, 1.0e-300];
    for tool in ToolId::ALL {
        let mut e = editor();
        add_object_rect(&mut e, 10.0, 10.0, 60.0, 60.0, RED);
        e.select_tool(tool);
        e.state.selection = anicraft_tools::Selection::one(anicraft_tools::ElementRef::top_level(0, 0, 0));
        for &x in &weird {
            for &y in &weird {
                for kind in [PointerKind::Down, PointerKind::Move, PointerKind::Hover, PointerKind::Up] {
                    let event = ToolEvent::Pointer(PointerEvent {
                        kind,
                        pos: Point::new(x, y),
                        screen: None,
                        modifiers: Modifiers::NONE,
                        pressure: f32::NAN,
                        click_count: 1,
                    });
                    let _ = e.send(&event);
                }
            }
        }
        // Zero and negative zoom.
        e.state.view.zoom = 0.0;
        let _ = e.send(&down(5.0, 5.0));
        e.state.view.zoom = -3.0;
        let _ = e.send(&up(50.0, 50.0));
        assert!(serde_json::to_string(&e.document).is_ok());
    }
}

#[test]
fn an_empty_document_and_missing_layers_are_fine() {
    let mut e = Editor::new(anim_scene::Document::default());
    for tool in ToolId::ALL {
        e.select_tool(tool);
        for ev in [down(1.0, 1.0), mv(5.0, 5.0), up(9.0, 9.0), hover(2.0, 2.0)] {
            let _ = e.send(&ev);
        }
        let _ = e.send(&KeyEvent::press(Key::Enter).into());
    }
    e.state.timeline = anim_scene::TimelineId::Symbol("missing".to_owned());
    for tool in ToolId::ALL {
        e.select_tool(tool);
        let _ = e.send(&down(1.0, 1.0));
        let _ = e.send(&up(9.0, 9.0));
    }
}

fn arb_event() -> BoxedStrategy<ToolEvent> {
    let coord = prop_oneof![-50.0_f64..650.0, Just(0.0), Just(550.0)];
    let modifiers = (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(shift, ctrl, alt)| Modifiers { shift, ctrl, alt });
    let pointer = (0_u8..4, coord.clone(), coord, modifiers.clone(), 0.0_f32..1.0, 1_u8..3).prop_map(|(k, x, y, m, p, c)| {
        let kind = match k {
            0 => PointerKind::Down,
            1 => PointerKind::Move,
            2 => PointerKind::Up,
            _ => PointerKind::Hover,
        };
        ToolEvent::Pointer(PointerEvent { kind, pos: Point::new(x, y), screen: None, modifiers: m, pressure: p, click_count: c })
    });
    let keys = prop_oneof![
        Just(Key::Escape),
        Just(Key::Enter),
        Just(Key::Backspace),
        Just(Key::Delete),
        Just(Key::Left),
        Just(Key::Right),
        Just(Key::Up),
        Just(Key::Down),
        Just(Key::Home),
        Just(Key::End),
        Just(Key::Tab),
        proptest::char::range('a', 'z').prop_map(Key::Char),
    ];
    let key = (keys, modifiers, any::<bool>()).prop_map(|(key, modifiers, pressed)| ToolEvent::Key(KeyEvent { key, modifiers, pressed }));
    let text = prop_oneof![Just("a".to_owned()), Just("\u{5E9}\u{5DC}".to_owned()), Just("e\u{301}".to_owned()), Just("\n".to_owned())]
        .prop_map(|text| ToolEvent::Text { text });
    let caret = (0_usize..12, 0_usize..12).prop_map(|(offset, anchor)| ToolEvent::SetCaret { offset, anchor });
    prop_oneof![8 => pointer, 2 => key, 1 => text, 1 => caret, 1 => Just(ToolEvent::Cancel)].boxed()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn random_event_streams_never_panic_and_leave_a_serialisable_document(
        tool in 0_usize..16,
        events in proptest::collection::vec(arb_event(), 1..40),
    ) {
        let mut e = editor();
        add_object_rect(&mut e, 100.0, 100.0, 220.0, 180.0, RED);
        let shape = rect_shape(250.0, 250.0, 400.0, 330.0, Some(BLUE), Some(3.0));
        anim_scene::edit::add_element(&mut e.document, &anim_scene::TimelineId::Scene(0), 0, 0, anim_scene::Element::Shape(anim_scene::ShapeElement::new(shape))).unwrap();
        e.select_tool(ToolId::ALL[tool]);
        for event in &events {
            let _ = e.send(event);
        }
        prop_assert!(serde_json::to_string(&e.document).is_ok());
        let warnings: Vec<String> = e.document.validate().iter().map(ToString::to_string).collect();
        prop_assert!(warnings.iter().all(|w| !w.contains("dangling")), "{:?}", warnings);
    }
}
