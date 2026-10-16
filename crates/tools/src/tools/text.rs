//! The Text tool: place a text field, type into it, edit existing fields.
//!
//! The tool keeps a logical (typing order) copy of the field's text and the caret as byte offsets. It never
//! shapes text: typing is inserted in logical order, so Hebrew and Arabic need no special treatment, and caret
//! motion that depends on layout (visual left and right in right-to-left text, up and down, line ends) is
//! delegated to the host with [`ToolCommand::CaretMotion`]; the host answers with a `SetCaret` event.

use anim_core::{Affine, Point, Vec2};
use anim_scene::{Element, TextElement, TextRun};
use unicode_segmentation::UnicodeSegmentation;

use crate::command::{CaretMotion, ToolCommand};
use crate::context::ToolContext;
use crate::event::{Key, Modifiers, PointerEvent, PointerKind, ToolEvent};
use crate::hit::pick;
use crate::overlay::{Cursor, Overlay, Role};
use crate::selection::{ElementRef, Selection};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Placeholder width of a new auto-expanding field, in pixels.
const NEW_FIELD_WIDTH: f64 = 8.0;

#[derive(Debug, Clone)]
struct Editing {
    target: ElementRef,
    /// The field's text in logical order, mirrored from the document.
    text: String,
    caret: usize,
    anchor: usize,
}

#[derive(Debug, Clone, Default)]
enum State {
    #[default]
    Idle,
    /// Dragging out the box of a new field.
    Placing {
        start: Point,
        current: Point,
    },
    Editing(Editing),
}

/// Creates and edits text fields.
#[derive(Debug, Clone, Default)]
pub struct TextTool {
    state: State,
}

/// The offset of the grapheme boundary before `offset`.
fn previous_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true).map(|(i, _)| i).take_while(|&i| i < offset).last().unwrap_or(0)
}

/// The offset of the grapheme boundary after `offset`.
fn next_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true).map(|(i, g)| i.saturating_add(g.len())).find(|&end| end > offset).unwrap_or(text.len())
}

fn clamp_to_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    offset
}

const fn motion_for(key: Key, modifiers: Modifiers) -> Option<CaretMotion> {
    Some(match (key, modifiers.ctrl) {
        (Key::Left, false) => CaretMotion::Left,
        (Key::Right, false) => CaretMotion::Right,
        (Key::Left, true) => CaretMotion::WordLeft,
        (Key::Right, true) => CaretMotion::WordRight,
        (Key::Up, _) => CaretMotion::Up,
        (Key::Down, _) => CaretMotion::Down,
        (Key::Home, false) => CaretMotion::LineStart,
        (Key::End, false) => CaretMotion::LineEnd,
        (Key::Home, true) => CaretMotion::DocumentStart,
        (Key::End, true) => CaretMotion::DocumentEnd,
        _ => return None,
    })
}

impl TextTool {
    const fn editing_mut(&mut self) -> Option<&mut Editing> {
        match &mut self.state {
            State::Editing(e) => Some(e),
            _ => None,
        }
    }

    /// Re-reads the text of the edited field from the document.
    fn refresh(&mut self, ctx: &ToolContext<'_>) {
        let Some(edit) = self.editing_mut() else { return };
        if let Some(resolved) = ctx.resolve(&edit.target)
            && let Element::Text(t) = resolved.element
        {
            edit.text = t.plain_text();
            edit.caret = clamp_to_boundary(&edit.text, edit.caret);
            edit.anchor = clamp_to_boundary(&edit.text, edit.anchor);
        }
    }

    fn start_editing(&mut self, target: ElementRef, ctx: &ToolContext<'_>) {
        let text = ctx.resolve(&target).and_then(|r| if let Element::Text(t) = r.element { Some(t.plain_text()) } else { None }).unwrap_or_default();
        let end = text.len();
        self.state = State::Editing(Editing { target, text, caret: end, anchor: end });
    }

    /// Commands that end editing, removing the field if it is empty.
    fn finish(&mut self) -> Vec<ToolCommand> {
        let State::Editing(edit) = std::mem::take(&mut self.state) else { return Vec::new() };
        let mut commands = vec![ToolCommand::EndTextEdit];
        if edit.text.is_empty() {
            commands.push(ToolCommand::DeleteElements { targets: vec![edit.target] });
        }
        commands
    }

    fn new_field(ctx: &ToolContext<'_>, origin: Point, size: Option<Vec2>) -> Option<(Element, ElementRef)> {
        let layer = ctx.layer_ref(ctx.layer)?;
        let (keyframe, count) = layer.keyframe_at(ctx.frame).map_or((ctx.frame, 0), |f| (f.index, f.elements.len()));
        let s = &ctx.settings.text;
        let mut run = TextRun::new("", s.face.clone(), s.size);
        run.fill_color = s.color;
        run.bold = s.bold;
        run.italic = s.italic;
        let (width, auto_expand) = size.map_or((NEW_FIELD_WIDTH, true), |v| (v.x.max(1.0), false));
        let height = f64::from(s.size) * 1.4;
        let mut field = TextElement::new(s.kind, Affine::translate(origin.to_vec2()), width, height, vec![run]);
        field.auto_expand = auto_expand;
        Some((Element::Text(Box::new(field)), ElementRef::top_level(ctx.layer, keyframe, count)))
    }

    fn down(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        self.refresh(ctx);
        if let Some(hit) = pick(ctx, p.pos)
            && let Some(resolved) = ctx.resolve(&hit.element)
            && matches!(resolved.element, Element::Text(_))
        {
            // A click inside the field being edited belongs to the host's text layout (it answers with SetCaret).
            if matches!(&self.state, State::Editing(e) if e.target == hit.element) {
                return ToolOutput::cursor(Cursor::Text);
            }
            let mut commands = self.finish();
            self.start_editing(hit.element.clone(), ctx);
            commands.push(ToolCommand::SetSelection { selection: Selection { items: vec![hit.element.clone()], scope: ctx.selection.scope.clone() } });
            commands.push(ToolCommand::BeginTextEdit { target: hit.element });
            return ToolOutput::with(Cursor::Text, commands);
        }
        let commands = self.finish();
        self.state = State::Placing { start: p.pos, current: p.pos };
        ToolOutput::with(Cursor::Text, commands)
    }

    fn up(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let State::Placing { start, .. } = self.state else { return ToolOutput::cursor(Cursor::Text) };
        self.state = State::Idle;
        if !ctx.layer_editable(ctx.layer) {
            return ToolOutput::cursor(Cursor::Text);
        }
        let dragged = p.pos - start;
        let size = (dragged.x.abs() > ctx.drag_threshold()).then_some(dragged);
        let origin = Point::new(start.x.min(p.pos.x), start.y.min(p.pos.y));
        let Some((element, target)) = Self::new_field(ctx, origin, size) else { return ToolOutput::cursor(Cursor::Text) };
        self.state = State::Editing(Editing { target: target.clone(), text: String::new(), caret: 0, anchor: 0 });
        ToolOutput::with(
            Cursor::Text,
            vec![
                ToolCommand::AddElement { layer: ctx.layer, frame: ctx.frame, element },
                ToolCommand::SetSelection { selection: Selection { items: vec![target.clone()], scope: ctx.selection.scope.clone() } },
                ToolCommand::BeginTextEdit { target },
            ],
        )
    }

    /// Replaces the selection (if any) with `text` at the caret.
    fn insert(&mut self, text: &str) -> Vec<ToolCommand> {
        let Some(edit) = self.editing_mut() else { return Vec::new() };
        let mut commands = Vec::new();
        let (start, end) = (edit.caret.min(edit.anchor), edit.caret.max(edit.anchor));
        if end > start {
            commands.push(ToolCommand::DeleteText { target: edit.target.clone(), start, end });
            edit.text.replace_range(start..end, "");
        }
        if !text.is_empty() {
            commands.push(ToolCommand::InsertText { target: edit.target.clone(), offset: start, text: text.to_owned() });
            edit.text.insert_str(start, text);
        }
        edit.caret = start.saturating_add(text.len());
        edit.anchor = edit.caret;
        commands
    }

    fn delete(&mut self, forward: bool) -> Vec<ToolCommand> {
        let Some(edit) = self.editing_mut() else { return Vec::new() };
        if edit.caret != edit.anchor {
            return self.insert("");
        }
        let (start, end) = if forward { (edit.caret, next_boundary(&edit.text, edit.caret)) } else { (previous_boundary(&edit.text, edit.caret), edit.caret) };
        if start == end {
            return Vec::new();
        }
        let command = ToolCommand::DeleteText { target: edit.target.clone(), start, end };
        edit.text.replace_range(start..end, "");
        edit.caret = start;
        edit.anchor = start;
        vec![command]
    }

    fn key(&mut self, key: Key, modifiers: Modifiers, ctx: &ToolContext<'_>) -> ToolOutput {
        self.refresh(ctx);
        if !matches!(self.state, State::Editing(_)) {
            return ToolOutput::cursor(Cursor::Text);
        }
        let commands = match key {
            Key::Escape => self.finish(),
            Key::Enter => self.insert("\n"),
            Key::Backspace => self.delete(false),
            Key::Delete => self.delete(true),
            Key::Tab => self.insert("\t"),
            Key::Space => self.insert(" "),
            Key::Char('a') if modifiers.ctrl => {
                if let Some(edit) = self.editing_mut() {
                    edit.anchor = 0;
                    edit.caret = edit.text.len();
                }
                Vec::new()
            }
            Key::Char(c) if !modifiers.ctrl => self.insert(&c.to_string()),
            other => motion_for(other, modifiers).map(|motion| ToolCommand::CaretMotion { motion, extend: modifiers.shift }).into_iter().collect(),
        };
        ToolOutput::with(Cursor::Text, commands)
    }
}

impl Tool for TextTool {
    fn id(&self) -> ToolId {
        ToolId::Text
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => self.down(p, ctx),
                PointerKind::Move => {
                    if let State::Placing { current, .. } = &mut self.state {
                        *current = p.pos;
                    }
                    ToolOutput::cursor(Cursor::Text)
                }
                PointerKind::Up => self.up(p, ctx),
                PointerKind::Hover => ToolOutput::cursor(Cursor::Text),
            },
            ToolEvent::Key(k) if k.pressed => self.key(k.key, k.modifiers, ctx),
            ToolEvent::Text { text } => {
                self.refresh(ctx);
                ToolOutput::with(Cursor::Text, self.insert(text))
            }
            ToolEvent::SetCaret { offset, anchor } => {
                if let Some(edit) = self.editing_mut() {
                    edit.caret = clamp_to_boundary(&edit.text, *offset);
                    edit.anchor = clamp_to_boundary(&edit.text, *anchor);
                }
                ToolOutput::cursor(Cursor::Text)
            }
            ToolEvent::Cancel => {
                let commands = self.finish();
                ToolOutput::with(Cursor::Text, commands)
            }
            ToolEvent::Key(_) => ToolOutput::cursor(Cursor::Text),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        match &self.state {
            State::Placing { start, current } if (current.x - start.x).abs() > ctx.drag_threshold() => {
                vec![Overlay::Rect {
                    rect: anim_core::Rect::from_points(*start, Point::new(current.x, f64::from(ctx.settings.text.size).mul_add(1.4, start.y))),
                    role: Role::Preview,
                }]
            }
            State::Editing(e) => {
                let mut out = Vec::new();
                if let Some(bounds) = ctx.stage_bounds(&e.target) {
                    out.push(Overlay::Rect { rect: bounds, role: Role::Selection });
                }
                out.push(Overlay::TextCursor { target: e.target.clone(), offset: e.caret, anchor: e.anchor });
                out
            }
            _ => Vec::new(),
        }
    }

    fn activated(&mut self, ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        if let [only] = ctx.selection.items.as_slice()
            && ctx.resolve(only).is_some_and(|r| matches!(r.element, Element::Text(_)))
        {
            self.start_editing(only.clone(), ctx);
            return vec![ToolCommand::BeginTextEdit { target: only.clone() }];
        }
        Vec::new()
    }

    fn cancel(&mut self) {
        self.state = State::Idle;
    }

    fn is_busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    fn captures_keys(&self) -> bool {
        matches!(self.state, State::Editing(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_respect_graphemes() {
        // 'e' + combining acute is one grapheme of three bytes.
        let text = "ae\u{301}b";
        assert_eq!(previous_boundary(text, 4), 1);
        assert_eq!(next_boundary(text, 1), 4);
        assert_eq!(previous_boundary(text, 0), 0);
        assert_eq!(next_boundary(text, text.len()), text.len());
    }

    #[test]
    fn hebrew_boundaries_are_per_letter_and_per_mark() {
        // shin + shin dot + lamed: the point attaches to the shin.
        let text = "\u{5E9}\u{5C1}\u{5DC}";
        assert_eq!(next_boundary(text, 0), 4);
        assert_eq!(previous_boundary(text, text.len()), 4);
    }

    #[test]
    fn clamping_lands_on_character_boundaries() {
        let text = "\u{5D0}\u{5D1}";
        assert_eq!(clamp_to_boundary(text, 1), 0);
        assert_eq!(clamp_to_boundary(text, 3), 2);
        assert_eq!(clamp_to_boundary(text, 99), 4);
    }
}
