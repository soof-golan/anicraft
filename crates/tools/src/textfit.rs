//! Sizing text fields to the text they hold.
//!
//! A text field's box is stored in the document (`width` and `height`), and picking, selecting, aligning and the
//! width handle all read it. Typing changes what the box has to hold, so after every edit the editor lays the text
//! out with `anim-text` and writes the size back: a box that grows with its text takes the width of the widest line,
//! and every box is as tall as its lines (never shorter than one line of its first run).

use anim_scene::{Document, Element};
use anim_text::TextEngine;

use crate::apply::element_mut;
use crate::selection::ElementRef;
use crate::state::EditorState;

/// Narrowest a box that grows with its text can be, in pixels (an empty field is still something to click).
const MIN_AUTO_WIDTH: f64 = 8.0;
/// Shortest a box can be, as a multiple of the size of its first run.
const MIN_HEIGHT_EM: f64 = 1.4;
/// Size assumed for a field without runs, in pixels.
const DEFAULT_SIZE: f64 = 24.0;

/// The text layout engine of an editor, made when the first field is measured.
///
/// Cloning an editor (the undo snapshots do) gives the copy an engine of its own, again made when first needed, so a
/// clone costs nothing.
#[derive(Debug, Default)]
pub struct TextMeter {
    engine: Option<TextEngine>,
}

impl Clone for TextMeter {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl TextMeter {
    /// Writes the size the text of the field `target` needs into the document. Does nothing when `target` is not a
    /// text field.
    pub fn fit(&mut self, doc: &mut Document, state: &EditorState, target: &ElementRef) {
        let Ok(Element::Text(field)) = element_mut(doc, &state.timeline, target) else { return };
        let engine = self.engine.get_or_insert_with(TextEngine::new);
        let rich = anim_timeline::rich_text(engine, field);
        let max_width = (!field.auto_expand && field.width.is_finite() && field.width > 0.0).then_some(field.width);
        let content = engine.layout(&rich, max_width).content_bounds();
        if !content.x1.is_finite() || !content.y1.is_finite() {
            return;
        }
        let line = field.text_runs.first().map_or(DEFAULT_SIZE, |r| f64::from(r.size)) * MIN_HEIGHT_EM;
        if field.auto_expand {
            field.width = content.x1.max(MIN_AUTO_WIDTH);
        }
        field.height = content.y1.max(line);
    }
}
