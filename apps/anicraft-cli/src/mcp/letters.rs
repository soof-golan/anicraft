//! Break Apart on a text field: the first press gives one static text field per letter, each where the letter stood;
//! the letters themselves break apart into shapes with the next press (see `arrange_ops`).

use anim_core::Affine;
use anim_scene::{Extra, TextAlign, TextDirection, TextElement, TextKind, TextOrientation, TextRun};
use anim_text::{TextEngine, TextLayout};

/// Paragraph attributes a letter field must not inherit: the letter's position already includes them.
const PARAGRAPH_ATTRS: [&str; 4] = ["indent", "leftMargin", "rightMargin", "lineSpacing"];

/// The text field's left inset (`left`), in pixels.
pub(super) fn left_of(text: &TextElement) -> f64 {
    text.extra.attr("left").and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0)
}

/// The field laid out the way it is drawn, or `None` when it has no text.
pub(super) fn laid_out(engine: &mut TextEngine, text: &TextElement) -> Option<TextLayout> {
    let rich = anim_timeline::rich_text(engine, text);
    if rich.is_empty() {
        return None;
    }
    let max_width = (!text.auto_expand && text.width.is_finite() && text.width > 0.0).then_some(text.width);
    Some(engine.layout(&rich, max_width))
}

/// The characters of the field with the index of the run each one came from, paragraph breaks as `'\n'`: the text
/// the layout's byte offsets index into.
fn styled_chars(text: &TextElement) -> Vec<(char, usize)> {
    let mut out = Vec::new();
    for (index, run) in text.text_runs.iter().enumerate() {
        let mut chars = run.characters.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    out.push(('\n', index));
                }
                other => out.push((other, index)),
            }
        }
    }
    out
}

/// One static, left-to-right field per visible letter of `text`, positioned where the letter was drawn.
///
/// `None` when the field cannot be split faithfully and its letters should break apart straight into shapes: a
/// single letter, vertical text, and right-to-left text (an Arabic letter on its own would lose the joining form
/// it has in the word).
pub(super) fn split(engine: &mut TextEngine, text: &TextElement, limit: usize) -> Option<Vec<TextElement>> {
    if text.orientation != TextOrientation::Horizontal {
        return None;
    }
    let layout = laid_out(engine, text)?;
    if layout.paragraphs.iter().any(|p| p.rtl) || layout.runs.iter().any(|r| r.rtl) {
        return None;
    }
    let chars = styled_chars(text);
    let rebuilt: String = chars.iter().map(|(c, _)| *c).collect();
    if rebuilt != layout.text() {
        return None;
    }
    let left = left_of(text);
    let mut out = Vec::new();
    let mut offset = 0_usize;
    for (c, run_index) in chars {
        let start = offset;
        offset = offset.saturating_add(c.len_utf8());
        if c.is_whitespace() {
            continue;
        }
        let rect = layout.selection_rects(start..offset).into_iter().next()?;
        let source = text.text_runs.get(run_index)?;
        let mut run: TextRun = source.clone();
        run.characters = c.to_string();
        run.alignment = TextAlign::Left;
        run.direction = TextDirection::LeftToRight;
        for name in PARAGRAPH_ATTRS {
            run.extra.remove_attr(name);
        }
        let matrix = text.matrix * Affine::translate((left + rect.x0, rect.y0));
        let mut field = TextElement::new(TextKind::Static, matrix, rect.width(), rect.height(), vec![run]);
        field.auto_expand = true;
        field.extra = Extra::new();
        out.push(field);
        if out.len() >= limit {
            break;
        }
    }
    (out.len() > 1).then_some(out)
}
