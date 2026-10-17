//! The host side of editing text on the stage.
//!
//! The Text tool keeps the text of the field being edited and the caret as byte offsets and never shapes anything. What
//! depends on how the text is laid out is answered here with `anim-text`: the caret and the highlight of the selection
//! drawn over the field, caret motion (arrow keys, home and end, words), and placing the caret with the pointer (click,
//! drag, double-click for a word, triple-click for a paragraph).

use anicraft_tools::{CaretMotion, Editor, ElementRef, Overlay as ToolOverlay, PointerKind, Role, ToolCommand, ToolEvent, resolve_ref};
use anim_core::kurbo::Shape as _;
use anim_core::{Affine, BezPath, Point};
use anim_scene::{Element, TextElement};
use anim_text::{Motion, Selection, TextEngine, TextLayout};

/// Screen points around the width handle of a field in which a press belongs to the handle, not to the text.
const HANDLE_ZONE: f64 = 9.0;

/// Text layout and the state of a caret gesture.
#[derive(Debug, Default)]
pub struct TextEdit {
    /// Made when the first field is laid out: loading the installed fonts takes a moment.
    engine: Option<TextEngine>,
    /// The x position the caret keeps over consecutive up and down moves.
    goal_x: Option<f64>,
    /// The pointer went down in the field being edited and is still down.
    dragging: bool,
}

/// The field `target` with the matrix from its own space to the space of the edited timeline.
fn field_of<'a>(editor: &'a Editor, target: &ElementRef) -> Option<(&'a TextElement, Affine)> {
    let resolved = resolve_ref(&editor.document, &editor.state.timeline, target)?;
    let Element::Text(field) = resolved.element else { return None };
    Some((field, resolved.local_to_stage()))
}

const fn motion_of(motion: CaretMotion) -> Motion {
    match motion {
        CaretMotion::Left => Motion::Left,
        CaretMotion::Right => Motion::Right,
        CaretMotion::Up => Motion::Up,
        CaretMotion::Down => Motion::Down,
        CaretMotion::LineStart => Motion::LineStart,
        CaretMotion::LineEnd => Motion::LineEnd,
        CaretMotion::WordLeft => Motion::WordLeft,
        CaretMotion::WordRight => Motion::WordRight,
        CaretMotion::DocumentStart => Motion::TextStart,
        CaretMotion::DocumentEnd => Motion::TextEnd,
    }
}

/// The caret and the anchor of the text cursor the tool reports, as offsets.
fn cursor_of(overlay: &[ToolOverlay]) -> Option<(usize, usize)> {
    overlay.iter().find_map(|o| match o {
        ToolOverlay::TextCursor { offset, anchor, .. } => Some((*offset, *anchor)),
        _ => None,
    })
}

impl TextEdit {
    /// `field` laid out the way it is drawn: wrapped at its width unless the box grows with its text.
    fn layout(&mut self, field: &TextElement) -> TextLayout {
        let engine = self.engine.get_or_insert_with(TextEngine::new);
        let rich = anim_timeline::rich_text(engine, field);
        let max_width = (!field.auto_expand && field.width.is_finite() && field.width > 0.0).then_some(field.width);
        engine.layout(&rich, max_width)
    }

    /// The tool overlay with the text cursor replaced by the highlight of the selected text and a line for the caret.
    pub fn cursor_overlays(&mut self, editor: &Editor, overlay: Vec<ToolOverlay>) -> Vec<ToolOverlay> {
        let mut out = Vec::with_capacity(overlay.len().saturating_add(2));
        for item in overlay {
            let ToolOverlay::TextCursor { target, offset, anchor } = &item else {
                out.push(item);
                continue;
            };
            let Some((field, to_space)) = field_of(editor, target) else { continue };
            let layout = self.layout(field);
            if offset != anchor {
                let mut path = BezPath::new();
                for rect in layout.selection_rects(*offset.min(anchor)..*offset.max(anchor)) {
                    path.extend(to_space * rect.to_path(0.1));
                }
                out.push(ToolOverlay::Path { path, role: Role::Selection, width: None, filled: true });
            }
            if let Some(rect) = layout.caret_rect(layout.caret_at(*offset), 0.0) {
                out.push(ToolOverlay::Line { a: to_space * Point::new(rect.x0, rect.y0), b: to_space * Point::new(rect.x0, rect.y1), role: Role::Selection });
            }
        }
        out
    }

    /// The events that answer what the layout of the field being edited decides: caret motion the tool asked for, and
    /// the pointer pressed, dragged or released in the field. `overlay` is the tool's, with its text cursor.
    pub fn follow_up(&mut self, editor: &Editor, event: &ToolEvent, commands: &[ToolCommand], overlay: &[ToolOverlay], zoom: f64) -> Vec<ToolEvent> {
        let Some(target) = editor.state.text_editing.as_ref() else {
            self.dragging = false;
            self.goal_x = None;
            return Vec::new();
        };
        let (Some((field, to_space)), Some((offset, anchor))) = (field_of(editor, target), cursor_of(overlay)) else { return Vec::new() };
        let mut out = Vec::new();
        let mut moved = false;
        for command in commands {
            if let ToolCommand::CaretMotion { motion, extend } = command {
                let layout = self.layout(field);
                let selection = Selection { anchor: layout.caret_at(anchor), focus: layout.caret_at(offset) };
                let motion = motion_of(*motion);
                let goal = matches!(motion, Motion::Up | Motion::Down).then(|| self.goal_x.or_else(|| layout.caret_x(selection.focus))).flatten();
                self.goal_x = goal;
                let next = layout.move_selection(selection, motion, *extend, goal);
                out.push(ToolEvent::SetCaret { offset: next.focus.offset, anchor: next.anchor.offset });
                moved = true;
            }
        }
        if !moved {
            self.goal_x = None;
        }
        let ToolEvent::Pointer(p) = event else { return out };
        let det = to_space.determinant();
        if !det.is_finite() || det.abs() < 1.0e-12 {
            return out;
        }
        let local = to_space.inverse() * p.pos;
        match p.kind {
            PointerKind::Down => {
                let in_box = (0.0..=field.width).contains(&local.x) && (0.0..=field.height).contains(&local.y);
                let near_handle = (to_space * Point::new(field.width, 0.0)).distance(p.pos) <= HANDLE_ZONE / zoom.max(1.0e-6);
                if in_box && !near_handle {
                    let layout = self.layout(field);
                    let hit = layout.hit_test_caret(local).offset;
                    let (focus, start) = match p.click_count {
                        0 | 1 if p.modifiers.shift => (hit, anchor),
                        0 | 1 => (hit, hit),
                        2 => {
                            let word = layout.word_range_at(hit);
                            (word.end, word.start)
                        }
                        _ => {
                            let paragraph = layout.paragraph_range_at(hit);
                            (paragraph.end, paragraph.start)
                        }
                    };
                    self.dragging = true;
                    out.push(ToolEvent::SetCaret { offset: focus, anchor: start });
                }
            }
            PointerKind::Move if self.dragging => {
                let layout = self.layout(field);
                out.push(ToolEvent::SetCaret { offset: layout.hit_test_caret(local).offset, anchor });
            }
            PointerKind::Up => self.dragging = false,
            PointerKind::Move | PointerKind::Hover => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use anicraft_cli::mcp::AniDoc;
    use anicraft_tools::{Key, KeyEvent, PointerEvent};
    use anim_scene::{TextKind, TextRun, TimelineId, edit};

    use super::*;

    /// A document with one fixed-width field of `text`, at (100, 50) and being edited.
    fn editing(text: &str) -> (AniDoc, ElementRef) {
        let mut doc = AniDoc::default();
        let mut field = TextElement::new(TextKind::Static, Affine::translate((100.0, 50.0)), 300.0, 40.0, vec![TextRun::new(text, "Arial", 24.0)]);
        field.auto_expand = false;
        edit::add_element(&mut doc.editor.document, &TimelineId::Scene(0), 0, 0, Element::Text(Box::new(field))).unwrap();
        let target = ElementRef::top_level(0, 0, 0);
        doc.editor.state.text_editing = Some(target.clone());
        (doc, target)
    }

    fn set_caret(offset: usize, anchor: usize) -> ToolEvent {
        ToolEvent::SetCaret { offset, anchor }
    }

    #[test]
    fn caret_motion_asked_for_by_the_tool_is_answered_with_the_new_caret() {
        let (doc, target) = editing("Hello world");
        let mut host = TextEdit::default();
        let (field, _) = field_of(&doc.editor, &target).unwrap();
        if host.layout(field).content_bounds().x1 <= 0.0 {
            // No font is installed on this machine, so nothing can be laid out.
            return;
        }
        let cursor = [ToolOverlay::TextCursor { target, offset: 0, anchor: 0 }];
        let key = ToolEvent::Key(KeyEvent::press(Key::End));
        let end = [ToolCommand::CaretMotion { motion: CaretMotion::LineEnd, extend: false }];
        assert_eq!(host.follow_up(&doc.editor, &key, &end, &cursor, 1.0), vec![set_caret(11, 11)]);
        let word = [ToolCommand::CaretMotion { motion: CaretMotion::WordRight, extend: true }];
        assert_eq!(host.follow_up(&doc.editor, &key, &word, &cursor, 1.0), vec![set_caret(5, 0)], "shift extends the selection from the anchor");
    }

    #[test]
    fn the_pointer_places_the_caret_selects_words_and_drags() {
        let (doc, target) = editing("Hello world");
        let mut host = TextEdit::default();
        let (field, _) = field_of(&doc.editor, &target).unwrap();
        if host.layout(field).content_bounds().x1 <= 0.0 {
            return;
        }
        let cursor = [ToolOverlay::TextCursor { target, offset: 11, anchor: 11 }];
        let press = |x: f64, count: u8| ToolEvent::Pointer(PointerEvent::down(x, 62.0).with_click_count(count));
        // Beyond the end of the line but inside the box: the end of the text. At the left edge: the start.
        assert_eq!(host.follow_up(&doc.editor, &press(390.0, 1), &[], &cursor, 1.0), vec![set_caret(11, 11)]);
        assert_eq!(host.follow_up(&doc.editor, &press(101.0, 1), &[], &cursor, 1.0), vec![set_caret(0, 0)]);
        // Double-click selects the word, drag extends from where the caret is.
        assert_eq!(host.follow_up(&doc.editor, &press(105.0, 2), &[], &cursor, 1.0), vec![set_caret(5, 0)]);
        let drag = ToolEvent::Pointer(PointerEvent::drag(390.0, 62.0));
        assert_eq!(
            host.follow_up(&doc.editor, &drag, &[], &[ToolOverlay::TextCursor { target: ElementRef::top_level(0, 0, 0), offset: 0, anchor: 0 }], 1.0),
            vec![set_caret(11, 0)]
        );
        // A press outside the box belongs to the tool alone.
        assert!(host.follow_up(&doc.editor, &press(30.0, 1), &[], &cursor, 1.0).is_empty());
        // Nothing is answered once no field is being edited.
        let mut idle = doc.clone();
        idle.editor.state.text_editing = None;
        assert!(host.follow_up(&idle.editor, &press(101.0, 1), &[], &cursor, 1.0).is_empty());
    }

    #[test]
    fn the_caret_and_the_selected_text_are_drawn_over_the_field() {
        let (doc, target) = editing("Hello world");
        let mut host = TextEdit::default();
        let (field, _) = field_of(&doc.editor, &target).unwrap();
        if host.layout(field).content_bounds().x1 <= 0.0 {
            return;
        }
        let collapsed = host.cursor_overlays(&doc.editor, vec![ToolOverlay::TextCursor { target: target.clone(), offset: 3, anchor: 3 }]);
        assert!(matches!(collapsed.as_slice(), [ToolOverlay::Line { .. }]), "a caret and nothing else: {collapsed:?}");
        let selected = host.cursor_overlays(&doc.editor, vec![ToolOverlay::TextCursor { target, offset: 2, anchor: 7 }]);
        assert!(matches!(selected.as_slice(), [ToolOverlay::Path { filled: true, .. }, ToolOverlay::Line { .. }]), "{selected:?}");
        let ToolOverlay::Line { a, b, .. } = &selected[1] else { panic!("line") };
        assert!(a.x > 100.0 && (a.x - b.x).abs() < 1.0e-9 && b.y > a.y, "the caret is a vertical line inside the field: {a:?} {b:?}");
    }
}
