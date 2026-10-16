//! The Colour panel's edits: load a fill and stroke into the toolbox and apply them to the selected shapes.

use anim_core::Rgba8;
use anim_scene::Element;
use anim_shape::{FillStyle, StrokeStyle};

use crate::command::ToolCommand;
use crate::editor::Editor;

/// Loads `fill` and/or `stroke` into the toolbox (when `toolbox`) and applies them to every selected shape.
/// Gradients and bitmaps are fitted to each shape. Returns how many shapes changed.
///
/// # Errors
/// When the editor refuses the resulting commands.
pub fn apply(editor: &mut Editor, fill: Option<&FillStyle>, stroke: Option<&StrokeStyle>, toolbox: bool) -> Result<usize, String> {
    if toolbox {
        if let Some(fill) = fill {
            editor.state.settings.fill = fill.clone();
            editor.state.settings.fill_enabled = true;
        }
        if let Some(stroke) = stroke {
            editor.state.settings.stroke = stroke.clone();
            editor.state.settings.stroke_enabled = true;
        }
    }
    let changes: Vec<ToolCommand> = editor.with_context(|ctx| {
        ctx.selection
            .items
            .iter()
            .filter_map(|r| {
                let resolved = ctx.resolve(r)?;
                let Element::Shape(s) = resolved.element else { return None };
                let mut shape = s.shape.clone();
                if let Some(fill) = fill {
                    let placed = shape.bounds().map_or_else(|| fill.clone(), |b| crate::fillfit::fit_fill(fill, b));
                    shape.fills.iter_mut().for_each(|f| *f = placed.clone());
                }
                if let Some(stroke) = stroke {
                    shape.strokes.iter_mut().for_each(|s| *s = stroke.clone());
                }
                (shape != s.shape).then(|| ToolCommand::ReplaceShape { target: r.clone(), shape })
            })
            .collect()
    });
    let updated = changes.len();
    if !changes.is_empty() {
        editor.run_commands(&changes).map_err(|e| e.to_string())?;
    }
    Ok(updated)
}

/// Swaps the fill and stroke colours of the toolbox.
pub fn swap(editor: &mut Editor) {
    let s = &mut editor.state.settings;
    let fill = s.fill.solid_color();
    let stroke = s.stroke.solid().fill.solid_color();
    if let Some(c) = stroke {
        s.fill = FillStyle::Solid(c);
    }
    if let Some(c) = fill {
        s.stroke.solid_mut().fill = FillStyle::Solid(c);
    }
    std::mem::swap(&mut s.fill_enabled, &mut s.stroke_enabled);
}

/// A black stroke and a white fill, the toolbox's "default colours" button.
pub fn defaults(editor: &mut Editor) {
    let s = &mut editor.state.settings;
    s.fill = FillStyle::Solid(Rgba8::WHITE);
    s.stroke.solid_mut().fill = FillStyle::Solid(Rgba8::BLACK);
    s.fill_enabled = true;
    s.stroke_enabled = true;
}
