//! The Text tool: placing fields, typing (including right-to-left text), editing, caret requests.

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
    clippy::many_single_char_names
)]

use crate::common;

use anicraft_tools::{CaretMotion, ElementRef, Key, KeyEvent, Modifiers, Overlay, Selection, ToolCommand, ToolEvent, ToolId, apply};
use anim_core::{Affine, Rgba8, Vec2};
use anim_scene::{Element, TextElement, TextKind, TextRun, TimelineId, edit};
use common::*;

fn text_of(e: &anicraft_tools::Editor, index: usize) -> String {
    match elements(e)[index] {
        Element::Text(t) => t.plain_text(),
        other => panic!("not text: {}", other.kind_name()),
    }
}

fn type_text(e: &mut anicraft_tools::Editor, s: &str) -> Vec<ToolCommand> {
    e.send(&ToolEvent::Text { text: s.to_owned() }).unwrap().commands
}

fn key(e: &mut anicraft_tools::Editor, k: Key) -> Vec<ToolCommand> {
    e.send(&KeyEvent::press(k).into()).unwrap().commands
}

fn new_field(e: &mut anicraft_tools::Editor, at: (f64, f64)) {
    e.select_tool(ToolId::Text);
    send_all(e, vec![down(at.0, at.1), up(at.0, at.1)]);
}

#[test]
fn clicking_places_an_empty_auto_expanding_field_and_starts_editing() {
    let mut e = editor();
    e.state.settings.text.size = 30.0;
    e.state.settings.text.color = Rgba8::opaque(10, 20, 30);
    e.select_tool(ToolId::Text);
    let commands = send_all(&mut e, vec![down(120.0, 80.0), up(120.0, 80.0)]);
    assert!(matches!(
        commands.as_slice(),
        [ToolCommand::AddElement { element: Element::Text(_), .. }, ToolCommand::SetSelection { .. }, ToolCommand::BeginTextEdit { .. }]
    ));
    let Element::Text(t) = elements(&e)[0] else { panic!("text") };
    assert!(t.auto_expand);
    assert_eq!(t.matrix, Affine::translate(Vec2::new(120.0, 80.0)));
    assert_eq!(t.text_runs[0].size, 30.0);
    assert_eq!(t.text_runs[0].fill_color, Rgba8::opaque(10, 20, 30));
    assert_eq!(e.state.text_editing, Some(ElementRef::top_level(0, 0, 0)));
    assert!(e.overlay().iter().any(|o| matches!(o, Overlay::TextCursor { offset: 0, anchor: 0, .. })));
}

#[test]
fn dragging_makes_a_fixed_width_field() {
    let mut e = editor();
    e.select_tool(ToolId::Text);
    send_all(&mut e, vec![down(300.0, 50.0), mv(350.0, 60.0), up(420.0, 70.0)]);
    let Element::Text(t) = elements(&e)[0] else { panic!("text") };
    assert!(!t.auto_expand);
    assert!((t.width - 120.0).abs() < 1.0e-9);
}

fn box_handle(e: &anicraft_tools::Editor) -> Option<(anim_core::Point, anicraft_tools::Marker)> {
    e.overlay().iter().find_map(|o| match o {
        Overlay::Marker { pos, marker, role: anicraft_tools::Role::Handle, .. } => Some((*pos, *marker)),
        _ => None,
    })
}

#[test]
fn dragging_the_box_handle_fixes_the_width_and_double_clicking_it_frees_it() {
    use anicraft_tools::{Marker, PointerEvent};
    let mut e = editor();
    new_field(&mut e, (100.0, 50.0));
    type_text(&mut e, "Hello wide world");
    let (handle, marker) = box_handle(&e).expect("a handle on the field being edited");
    assert_eq!(marker, Marker::Circle, "a circle while the box grows with its text");

    // Dragging the handle makes the box a fixed width, to where the pointer was released.
    let target = anim_core::Point::new(handle.x + 40.0, handle.y);
    let commands = send_all(&mut e, vec![down(handle.x, handle.y), mv(handle.x + 20.0, handle.y), up(target.x, target.y)]);
    assert!(matches!(commands.as_slice(), [ToolCommand::SetTextBox { auto_expand: false, .. }]));
    let Element::Text(t) = elements(&e)[0] else { panic!("text") };
    assert!(!t.auto_expand);
    assert!((t.width - (target.x - 100.0)).abs() < 1.0e-6, "the width runs from the field's left edge to the pointer: {}", t.width);
    assert_eq!(e.state.text_editing, Some(ElementRef::top_level(0, 0, 0)), "still editing the field");
    assert_eq!(text_of(&e, 0), "Hello wide world");

    // A square marks a fixed width; double-clicking the handle lets the box grow with its text again.
    let (handle, marker) = box_handle(&e).expect("handle");
    assert_eq!(marker, Marker::Square);
    send_all(&mut e, vec![PointerEvent::down(handle.x, handle.y).with_click_count(2).into(), up(handle.x, handle.y)]);
    let Element::Text(t) = elements(&e)[0] else { panic!("text") };
    assert!(t.auto_expand);
    assert_eq!(box_handle(&e).map(|(_, m)| m), Some(Marker::Circle));
    assert!(e.undo(), "the width change is an undo step");
}

#[test]
fn the_text_type_option_decides_what_kind_of_field_is_created() {
    for kind in [TextKind::Static, TextKind::Dynamic, TextKind::Input] {
        let mut e = editor();
        e.state.settings.text.kind = kind;
        new_field(&mut e, (50.0, 50.0));
        let Element::Text(t) = elements(&e)[0] else { panic!("text") };
        assert_eq!(t.kind, kind);
    }
}

#[test]
fn typing_inserts_in_order_at_the_caret() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    let c = type_text(&mut e, "Hello");
    assert!(matches!(c.as_slice(), [ToolCommand::InsertText { offset: 0, text, .. }] if text == "Hello"));
    let c = type_text(&mut e, ", world");
    assert!(matches!(c.as_slice(), [ToolCommand::InsertText { offset: 5, .. }]));
    key(&mut e, Key::Char('!'));
    key(&mut e, Key::Enter);
    key(&mut e, Key::Char('x'));
    assert_eq!(text_of(&e, 0), "Hello, world!\nx");
    key(&mut e, Key::Backspace);
    key(&mut e, Key::Backspace);
    assert_eq!(text_of(&e, 0), "Hello, world!");
    assert_eq!(e.undo_depth(), 8, "adding the field and each edit is one undo step");
}

#[test]
fn hebrew_and_mixed_text_type_and_delete_correctly() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    type_text(&mut e, "\u{5E9}\u{5DC}\u{5D5}\u{5DD}");
    type_text(&mut e, " hello ");
    type_text(&mut e, "\u{5E2}\u{5D5}\u{5DC}\u{5DD}");
    assert_eq!(text_of(&e, 0), "\u{5E9}\u{5DC}\u{5D5}\u{5DD} hello \u{5E2}\u{5D5}\u{5DC}\u{5DD}");
    // Backspace removes one Hebrew letter (two bytes), not half of it.
    key(&mut e, Key::Backspace);
    assert_eq!(text_of(&e, 0), "\u{5E9}\u{5DC}\u{5D5}\u{5DD} hello \u{5E2}\u{5D5}\u{5DC}");
    // A letter with its niqqud is deleted as one unit.
    type_text(&mut e, "\u{5E9}\u{5C1}");
    key(&mut e, Key::Backspace);
    assert_eq!(text_of(&e, 0), "\u{5E9}\u{5DC}\u{5D5}\u{5DD} hello \u{5E2}\u{5D5}\u{5DC}");
}

#[test]
fn arabic_and_combining_marks_delete_by_cluster() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    type_text(&mut e, "\u{645}\u{631}\u{62D}\u{628}\u{627}");
    type_text(&mut e, "e\u{301}");
    key(&mut e, Key::Backspace);
    assert_eq!(text_of(&e, 0), "\u{645}\u{631}\u{62D}\u{628}\u{627}");
    // Delete at the caret removes forward.
    e.send(&ToolEvent::SetCaret { offset: 0, anchor: 0 }).unwrap();
    key(&mut e, Key::Delete);
    assert_eq!(text_of(&e, 0), "\u{631}\u{62D}\u{628}\u{627}");
}

#[test]
fn selection_is_replaced_by_typing() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    type_text(&mut e, "abcdef");
    e.send(&ToolEvent::SetCaret { offset: 4, anchor: 1 }).unwrap();
    let c = type_text(&mut e, "XY");
    assert!(matches!(c.as_slice(), [ToolCommand::DeleteText { start: 1, end: 4, .. }, ToolCommand::InsertText { offset: 1, .. }]));
    assert_eq!(text_of(&e, 0), "aXYef");
    e.send(&ToolEvent::SetCaret { offset: 3, anchor: 1 }).unwrap();
    key(&mut e, Key::Backspace);
    assert_eq!(text_of(&e, 0), "aef");
}

#[test]
fn set_caret_clamps_to_character_boundaries() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    type_text(&mut e, "\u{5D0}\u{5D1}");
    e.send(&ToolEvent::SetCaret { offset: 1, anchor: 1 }).unwrap();
    type_text(&mut e, "x");
    assert_eq!(text_of(&e, 0), "x\u{5D0}\u{5D1}");
    e.send(&ToolEvent::SetCaret { offset: 999, anchor: 999 }).unwrap();
    type_text(&mut e, "y");
    assert_eq!(text_of(&e, 0), "x\u{5D0}\u{5D1}y");
}

#[test]
fn arrow_keys_ask_the_host_to_move_the_caret() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    type_text(&mut e, "abc");
    assert_eq!(key(&mut e, Key::Left), vec![ToolCommand::CaretMotion { motion: CaretMotion::Left, extend: false }]);
    let c = e.send(&KeyEvent::press(Key::Right).with_modifiers(Modifiers { shift: true, ctrl: true, alt: false }).into()).unwrap().commands;
    assert_eq!(c, vec![ToolCommand::CaretMotion { motion: CaretMotion::WordRight, extend: true }]);
    assert_eq!(key(&mut e, Key::Home), vec![ToolCommand::CaretMotion { motion: CaretMotion::LineStart, extend: false }]);
    assert_eq!(key(&mut e, Key::Down), vec![ToolCommand::CaretMotion { motion: CaretMotion::Down, extend: false }]);
}

#[test]
fn letters_are_typed_not_treated_as_tool_shortcuts_while_editing() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    key(&mut e, Key::Char('v'));
    key(&mut e, Key::Char('r'));
    key(&mut e, Key::Space);
    assert_eq!(e.tools.active(), ToolId::Text);
    assert_eq!(text_of(&e, 0), "vr ");
}

#[test]
fn escape_removes_an_empty_field_and_keeps_a_filled_one() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    let c = key(&mut e, Key::Escape);
    assert!(matches!(c.as_slice(), [ToolCommand::EndTextEdit, ToolCommand::DeleteElements { .. }]));
    assert!(elements(&e).is_empty());
    assert_eq!(e.state.text_editing, None);

    new_field(&mut e, (50.0, 50.0));
    type_text(&mut e, "keep");
    let c = key(&mut e, Key::Escape);
    assert_eq!(c, vec![ToolCommand::EndTextEdit]);
    assert_eq!(text_of(&e, 0), "keep");
}

#[test]
fn clicking_elsewhere_ends_editing_and_starts_a_new_field() {
    let mut e = editor();
    new_field(&mut e, (50.0, 50.0));
    type_text(&mut e, "one");
    send_all(&mut e, vec![down(300.0, 300.0), up(300.0, 300.0)]);
    type_text(&mut e, "two");
    assert_eq!(elements(&e).len(), 2);
    assert_eq!(text_of(&e, 0), "one");
    assert_eq!(text_of(&e, 1), "two");
}

fn add_text(e: &mut anicraft_tools::Editor, runs: &[&str], at: (f64, f64)) {
    let runs = runs.iter().map(|s| TextRun::new(*s, "Arial", 24.0)).collect();
    let field = TextElement::new(TextKind::Static, Affine::translate(Vec2::new(at.0, at.1)), 200.0, 40.0, runs);
    edit::add_element(&mut e.document, &TimelineId::Scene(0), 0, 0, Element::Text(Box::new(field))).unwrap();
}

#[test]
fn clicking_an_existing_field_edits_it() {
    let mut e = editor();
    add_text(&mut e, &["hello"], (100.0, 100.0));
    e.select_tool(ToolId::Text);
    let c = send_all(&mut e, vec![down(150.0, 115.0), up(150.0, 115.0)]);
    assert!(c.iter().any(|c| matches!(c, ToolCommand::BeginTextEdit { .. })));
    assert_eq!(elements(&e).len(), 1);
    // The caret starts at the end; the host may move it with SetCaret.
    type_text(&mut e, "!");
    assert_eq!(text_of(&e, 0), "hello!");
    e.send(&ToolEvent::SetCaret { offset: 0, anchor: 0 }).unwrap();
    type_text(&mut e, ">");
    assert_eq!(text_of(&e, 0), ">hello!");
}

#[test]
fn double_clicking_a_field_with_the_selection_tool_opens_it_for_typing() {
    let mut e = editor();
    add_text(&mut e, &["abc"], (100.0, 100.0));
    e.select_tool(ToolId::Selection);
    send_all(&mut e, vec![down(150.0, 115.0), up(150.0, 115.0)]);
    assert_eq!(e.state.selection, Selection::one(ElementRef::top_level(0, 0, 0)));
    let commands = send_all(&mut e, vec![anicraft_tools::PointerEvent::down(150.0, 115.0).with_click_count(2).into(), up(150.0, 115.0)]);
    assert!(commands.contains(&ToolCommand::ActivateTool { tool: ToolId::Text }));
    assert!(commands.iter().any(|c| matches!(c, ToolCommand::BeginTextEdit { .. })));
    assert_eq!(e.tools.active(), ToolId::Text);
    type_text(&mut e, "d");
    assert_eq!(text_of(&e, 0), "abcd");
}

#[test]
fn edits_across_runs_land_in_the_right_run() {
    let mut e = editor();
    add_text(&mut e, &["ab", "cd"], (100.0, 100.0));
    let target = ElementRef::top_level(0, 0, 0);
    apply(&mut e.document, &mut e.state, &ToolCommand::InsertText { target: target.clone(), offset: 3, text: "X".to_owned() }).unwrap();
    assert_eq!(text_of(&e, 0), "abcXd");
    apply(&mut e.document, &mut e.state, &ToolCommand::DeleteText { target: target.clone(), start: 1, end: 4 }).unwrap();
    assert_eq!(text_of(&e, 0), "ad");
    let Element::Text(t) = elements(&e)[0] else { panic!("text") };
    assert_eq!(t.text_runs.len(), 2, "both runs keep their style");
    apply(&mut e.document, &mut e.state, &ToolCommand::DeleteText { target, start: 0, end: 99 }).unwrap();
    assert_eq!(text_of(&e, 0), "");
    let Element::Text(t) = elements(&e)[0] else { panic!("text") };
    assert_eq!(t.text_runs.len(), 1);
}

#[test]
fn text_events_without_a_field_do_nothing() {
    let mut e = editor();
    e.select_tool(ToolId::Text);
    assert!(type_text(&mut e, "nobody home").is_empty());
    assert!(key(&mut e, Key::Backspace).is_empty());
    assert!(elements(&e).is_empty());
}
