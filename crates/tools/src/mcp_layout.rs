//! MCP commands for layout: the Align panel (`layout.align`, `layout.distribute`, `layout.space`,
//! `layout.match_size`), the Transform panel (`transform.apply`), and the guides and grid of the View menu
//! (`guides.get`, `guides.add`, `guides.move`, `guides.remove`, `guides.clear`, `guides.set`, `grid.set`).
//!
//! Snapping switches are tool options: change them with `tools.options` and a `snap` patch.

#![allow(clippy::needless_raw_string_hashes, reason = "the JSON schemas are easier to read and edit with uniform raw string delimiters")]

use anim_core::{Affine, Point, Rect, Rgba8};
use anim_session::{CommandError, CommandSpec, parse_params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::command::ToolCommand;
use crate::editor::Editor;
use crate::guides::{self, Layout, MAX_GRID, MAX_GUIDES, MIN_GRID};
use crate::layout::{self, Axis, Edge};
use crate::mcp::{ToolHost, atomic, fail, finite_all};
use crate::selection::Selection;

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::invalid_params(message.into())
}

/// The selected objects and their stage bounds.
fn selected(editor: &Editor) -> Result<(Vec<crate::selection::ElementRef>, Vec<Rect>), CommandError> {
    let items = editor.state.selection.items.clone();
    let rects: Vec<(crate::selection::ElementRef, Rect)> =
        editor.with_context(|ctx| items.iter().filter_map(|r| ctx.stage_bounds(r).map(|b| (r.clone(), b))).collect());
    if rects.is_empty() {
        return Err(CommandError::failed("nothing is selected; use select_at, select_rect or select_all first"));
    }
    Ok(rects.into_iter().unzip())
}

const fn stage_rect(editor: &Editor) -> Rect {
    Rect::new(0.0, 0.0, editor.document.width, editor.document.height)
}

/// Applies one stage-space matrix to each object, as one undo step, and keeps the selection.
fn apply_each(editor: &mut Editor, refs: &[crate::selection::ElementRef], matrices: &[Affine]) -> Result<Value, CommandError> {
    let keep = editor.state.selection.clone();
    let mut commands = Vec::new();
    let mut moved = 0_usize;
    for (r, m) in refs.iter().zip(matrices) {
        if *m == Affine::IDENTITY {
            continue;
        }
        if !m.as_coeffs().iter().all(|c| c.is_finite()) {
            return Err(invalid("the layout is not finite"));
        }
        commands.push(ToolCommand::SetSelection { selection: Selection { items: vec![r.clone()], scope: keep.scope.clone() } });
        commands.push(ToolCommand::TransformSelection { matrix: *m });
        moved = moved.saturating_add(1);
    }
    commands.push(ToolCommand::SetSelection { selection: keep });
    atomic(editor, |ed| {
        ed.run_commands(&commands).map_err(fail)?;
        Ok(json!({ "moved": moved, "selected": ed.state.selection.items.len() }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AlignParams {
    edge: Edge,
    #[serde(default)]
    to_stage: bool,
}

fn layout_align<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: AlignParams = parse_params(params)?;
    let editor = host.editor_mut();
    let (refs, rects) = selected(editor)?;
    let reference = if p.to_stage { stage_rect(editor) } else { layout::union(&rects).ok_or_else(|| invalid("nothing is selected"))? };
    let matrices = layout::align(&rects, p.edge, reference);
    apply_each(editor, &refs, &matrices)
}

fn layout_distribute<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: AlignParams = parse_params(params)?;
    let editor = host.editor_mut();
    let (refs, rects) = selected(editor)?;
    let minimum = if p.to_stage { 2 } else { 3 };
    if rects.len() < minimum {
        return Err(CommandError::failed(format!("distributing needs at least {minimum} objects")));
    }
    let matrices = layout::distribute(&rects, p.edge, p.to_stage.then(|| stage_rect(editor)));
    apply_each(editor, &refs, &matrices)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceParams {
    axis: Axis,
    /// The distance between neighbours in pixels; equal gaps between the first and the last when absent.
    gap: Option<f64>,
    #[serde(default)]
    to_stage: bool,
}

fn layout_space<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: SpaceParams = parse_params(params)?;
    if let Some(gap) = p.gap {
        finite_all(&[gap])?;
    }
    let editor = host.editor_mut();
    let (refs, rects) = selected(editor)?;
    if rects.len() < 2 {
        return Err(CommandError::failed("spacing needs at least 2 objects"));
    }
    let matrices = layout::space(&rects, p.axis, p.gap, p.to_stage.then(|| stage_rect(editor)));
    apply_each(editor, &refs, &matrices)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatchParams {
    #[serde(default = "yes")]
    width: bool,
    #[serde(default = "yes")]
    height: bool,
    #[serde(default)]
    to_stage: bool,
}

const fn yes() -> bool {
    true
}

fn layout_match_size<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: MatchParams = parse_params(params)?;
    let editor = host.editor_mut();
    let (refs, rects) = selected(editor)?;
    if !p.to_stage && rects.len() < 2 {
        return Err(CommandError::failed("matching size needs at least 2 objects"));
    }
    let matrices = layout::match_size(&rects, p.width, p.height, p.to_stage.then(|| stage_rect(editor)));
    apply_each(editor, &refs, &matrices)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyParams {
    /// Horizontal scale factor (1 is unchanged).
    #[serde(default = "one")]
    sx: f64,
    /// Vertical scale factor; follows `sx` when absent.
    sy: Option<f64>,
    #[serde(default)]
    rotate_degrees: f64,
    #[serde(default)]
    skew_x_degrees: f64,
    #[serde(default)]
    skew_y_degrees: f64,
    /// Fixed point; the centre of the selection by default.
    origin: Option<[f64; 2]>,
    /// Duplicate the selection first and transform the copies (Copy and Apply Transform).
    #[serde(default)]
    copy: bool,
}

const fn one() -> f64 {
    1.0
}

fn transform_apply<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: ApplyParams = parse_params(params)?;
    let sy = p.sy.unwrap_or(p.sx);
    finite_all(&[p.sx, sy, p.rotate_degrees, p.skew_x_degrees, p.skew_y_degrees])?;
    if p.sx == 0.0 || sy == 0.0 {
        return Err(invalid("a scale of zero would flatten the selection"));
    }
    if p.skew_x_degrees.abs() >= 89.0 || p.skew_y_degrees.abs() >= 89.0 {
        return Err(invalid("skew angles must be within 89 degrees"));
    }
    let editor = host.editor_mut();
    let (_, rects) = selected(editor)?;
    let bounds = layout::union(&rects).ok_or_else(|| invalid("nothing is selected"))?;
    let o = p.origin.map_or_else(|| bounds.center(), |[x, y]| Point::new(x, y));
    finite_all(&[o.x, o.y])?;
    let about = Affine::translate(o.to_vec2());
    let matrix = about
        * Affine::rotate(p.rotate_degrees.to_radians())
        * Affine::skew(p.skew_x_degrees.to_radians().tan(), p.skew_y_degrees.to_radians().tan())
        * Affine::scale_non_uniform(p.sx, sy)
        * Affine::translate(-o.to_vec2());
    atomic(editor, |ed| {
        let mut commands = Vec::new();
        if p.copy {
            commands.push(ToolCommand::DuplicateSelection);
        }
        commands.push(ToolCommand::TransformSelection { matrix });
        ed.run_commands(&commands).map_err(fail)?;
        Ok(json!({ "selected": ed.state.selection.items.len(), "copied": p.copy }))
    })
}

// ---- guides and grid -----------------------------------------------------------------------------------------------

fn layout_json(l: &Layout) -> Value {
    json!({
        "guides": { "vertical": l.guides_x, "horizontal": l.guides_y, "locked": l.guides_locked, "visible": l.guides_visible, "color": l.guide_color.to_hex_rgb() },
        "grid": { "visible": l.grid_visible, "spacing_x": l.grid[0], "spacing_y": l.grid[1], "color": l.grid_color.to_hex_rgb() },
    })
}

#[allow(clippy::unnecessary_wraps, reason = "must match the RunFn signature")]
fn guides_get<D: ToolHost>(host: &mut D, _: &Value) -> Result<Value, CommandError> {
    Ok(layout_json(&guides::read(&host.editor().document)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuideAddParams {
    axis: Axis,
    /// Stage pixels: x for a vertical guide, y for a horizontal one.
    position: f64,
}

fn guides_add<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: GuideAddParams = parse_params(params)?;
    finite_all(&[p.position])?;
    let ed = host.editor_mut();
    let mut l = guides::read(&ed.document);
    let list = if p.axis == Axis::Horizontal { &mut l.guides_y } else { &mut l.guides_x };
    if list.len() >= MAX_GUIDES {
        return Err(CommandError::failed("there are too many guides"));
    }
    list.push(p.position);
    guides::write(&mut ed.document, &l);
    Ok(layout_json(&l))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuideMoveParams {
    axis: Axis,
    index: usize,
    position: f64,
}

fn guides_move<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: GuideMoveParams = parse_params(params)?;
    finite_all(&[p.position])?;
    let ed = host.editor_mut();
    let mut l = guides::read(&ed.document);
    let list = if p.axis == Axis::Horizontal { &mut l.guides_y } else { &mut l.guides_x };
    *list.get_mut(p.index).ok_or_else(|| invalid("there is no guide at that index"))? = p.position;
    guides::write(&mut ed.document, &l);
    Ok(layout_json(&l))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuideRemoveParams {
    axis: Axis,
    index: usize,
}

fn guides_remove<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: GuideRemoveParams = parse_params(params)?;
    let ed = host.editor_mut();
    let mut l = guides::read(&ed.document);
    let list = if p.axis == Axis::Horizontal { &mut l.guides_y } else { &mut l.guides_x };
    if p.index >= list.len() {
        return Err(invalid("there is no guide at that index"));
    }
    list.remove(p.index);
    guides::write(&mut ed.document, &l);
    Ok(layout_json(&l))
}

#[allow(clippy::unnecessary_wraps, reason = "must match the RunFn signature")]
fn guides_clear<D: ToolHost>(host: &mut D, _: &Value) -> Result<Value, CommandError> {
    let ed = host.editor_mut();
    let mut l = guides::read(&ed.document);
    l.guides_x.clear();
    l.guides_y.clear();
    guides::write(&mut ed.document, &l);
    Ok(layout_json(&l))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuideSetParams {
    visible: Option<bool>,
    locked: Option<bool>,
    color: Option<String>,
    /// Replaces the vertical guides (x positions).
    vertical: Option<Vec<f64>>,
    /// Replaces the horizontal guides (y positions).
    horizontal: Option<Vec<f64>>,
}

fn colour(text: &str) -> Result<Rgba8, CommandError> {
    Rgba8::from_hex(text).map_err(|e| invalid(format!("not a colour: {e:?}")))
}

fn guides_set<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: GuideSetParams = parse_params(params)?;
    let ed = host.editor_mut();
    let mut l = guides::read(&ed.document);
    if let Some(v) = p.visible {
        l.guides_visible = v;
    }
    if let Some(v) = p.locked {
        l.guides_locked = v;
    }
    if let Some(c) = &p.color {
        l.guide_color = colour(c)?;
    }
    for (new, slot) in [(p.vertical, &mut l.guides_x), (p.horizontal, &mut l.guides_y)] {
        if let Some(values) = new {
            finite_all(&values)?;
            if values.len() > MAX_GUIDES {
                return Err(CommandError::failed("there are too many guides"));
            }
            *slot = values;
        }
    }
    guides::write(&mut ed.document, &l);
    Ok(layout_json(&l))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GridSetParams {
    visible: Option<bool>,
    /// Spacing in pixels for both directions.
    spacing: Option<f64>,
    spacing_x: Option<f64>,
    spacing_y: Option<f64>,
    color: Option<String>,
}

fn grid_set<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: GridSetParams = parse_params(params)?;
    let ed = host.editor_mut();
    let mut l = guides::read(&ed.document);
    if let Some(v) = p.visible {
        l.grid_visible = v;
    }
    for (i, value) in [p.spacing_x.or(p.spacing), p.spacing_y.or(p.spacing)].into_iter().enumerate() {
        if let Some(v) = value {
            finite_all(&[v])?;
            if !(MIN_GRID..=MAX_GRID).contains(&v) {
                return Err(invalid(format!("the grid spacing must be between {MIN_GRID} and {MAX_GRID} pixels")));
            }
            if let Some(slot) = l.grid.get_mut(i) {
                *slot = v;
            }
        }
    }
    if let Some(c) = &p.color {
        l.grid_color = colour(c)?;
    }
    guides::write(&mut ed.document, &l);
    Ok(layout_json(&l))
}

// ---- registry ------------------------------------------------------------------------------------------------------

const SCHEMA_EDGE: &str = r#"{"type":"object","required":["edge"],"properties":{"edge":{"enum":["left","horizontal_center","right","top","vertical_center","bottom"]},"to_stage":{"type":"boolean","description":"Use the stage instead of the bounds of the selection"}},"additionalProperties":false}"#;
const SCHEMA_SPACE: &str = r#"{"type":"object","required":["axis"],"properties":{"axis":{"enum":["horizontal","vertical"]},"gap":{"type":"number","description":"Distance between neighbours in pixels; equal gaps when absent"},"to_stage":{"type":"boolean"}},"additionalProperties":false}"#;
const SCHEMA_MATCH: &str = r#"{"type":"object","properties":{"width":{"type":"boolean","description":"Match widths (default true)"},"height":{"type":"boolean","description":"Match heights (default true)"},"to_stage":{"type":"boolean","description":"Match the size of the stage instead of the largest object"}},"additionalProperties":false}"#;
const SCHEMA_APPLY: &str = r#"{"type":"object","properties":{"sx":{"type":"number","description":"Horizontal scale factor, 1 is unchanged"},"sy":{"type":"number","description":"Vertical scale factor; follows sx"},"rotate_degrees":{"type":"number"},"skew_x_degrees":{"type":"number"},"skew_y_degrees":{"type":"number"},"origin":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"copy":{"type":"boolean","description":"Duplicate the selection first and transform the copies"}},"additionalProperties":false}"#;
const SCHEMA_NONE: &str = r#"{"type":"object","properties":{},"additionalProperties":false}"#;
const SCHEMA_GUIDE_ADD: &str = r#"{"type":"object","required":["axis","position"],"properties":{"axis":{"enum":["horizontal","vertical"],"description":"A horizontal guide has a y position, a vertical one an x position"},"position":{"type":"number"}},"additionalProperties":false}"#;
const SCHEMA_GUIDE_MOVE: &str = r#"{"type":"object","required":["axis","index","position"],"properties":{"axis":{"enum":["horizontal","vertical"]},"index":{"type":"integer","minimum":0},"position":{"type":"number"}},"additionalProperties":false}"#;
const SCHEMA_GUIDE_REMOVE: &str = r#"{"type":"object","required":["axis","index"],"properties":{"axis":{"enum":["horizontal","vertical"]},"index":{"type":"integer","minimum":0}},"additionalProperties":false}"#;
const SCHEMA_GUIDE_SET: &str = r#"{"type":"object","properties":{"visible":{"type":"boolean"},"locked":{"type":"boolean"},"color":{"type":"string"},"vertical":{"type":"array","items":{"type":"number"},"description":"Replaces the x positions of the vertical guides"},"horizontal":{"type":"array","items":{"type":"number"},"description":"Replaces the y positions of the horizontal guides"}},"additionalProperties":false}"#;
const SCHEMA_GRID: &str = r#"{"type":"object","properties":{"visible":{"type":"boolean"},"spacing":{"type":"number","minimum":1,"maximum":1000},"spacing_x":{"type":"number","minimum":1,"maximum":1000},"spacing_y":{"type":"number","minimum":1,"maximum":1000},"color":{"type":"string"}},"additionalProperties":false}"#;

/// The layout commands, ready for `CommandRegistry::register_all`.
pub fn commands<D: ToolHost>() -> Vec<CommandSpec<D>> {
    vec![
        CommandSpec::new("layout.align", "Align", layout_align::<D>)
            .with_schema(SCHEMA_EDGE)
            .describe("The Align panel: line the selected objects up on their left, horizontal centre, right, top, vertical centre or bottom edge, relative to the selection or the stage."),
        CommandSpec::new("layout.distribute", "Distribute", layout_distribute::<D>)
            .with_schema(SCHEMA_EDGE)
            .describe("The Align panel: spread three or more objects so that the chosen edges or centres are evenly spaced; with to_stage the first and the last go flush with the stage."),
        CommandSpec::new("layout.space", "Space", layout_space::<D>)
            .with_schema(SCHEMA_SPACE)
            .describe("The Align panel: make the gaps between the selected objects equal along an axis, or a fixed gap in pixels."),
        CommandSpec::new("layout.match_size", "Match Size", layout_match_size::<D>)
            .with_schema(SCHEMA_MATCH)
            .describe("The Align panel: scale the selected objects to the width and/or height of the largest, or of the stage."),
        CommandSpec::new("transform.apply", "Transform", transform_apply::<D>)
            .with_schema(SCHEMA_APPLY)
            .describe("The Transform panel: scale, rotate and skew the selection about a fixed point in one step; copy applies it to a duplicate instead."),
        CommandSpec::new("guides.get", "Get Guides and Grid", guides_get::<D>).transient().with_schema(SCHEMA_NONE).describe("The guides and the grid of the document: positions, lock, visibility, colours and grid spacing."),
        CommandSpec::new("guides.add", "Add Guide", guides_add::<D>).with_schema(SCHEMA_GUIDE_ADD).describe("Add a horizontal or vertical guide (dragging out of a ruler)."),
        CommandSpec::new("guides.move", "Move Guide", guides_move::<D>).with_schema(SCHEMA_GUIDE_MOVE).describe("Move a guide to a new position."),
        CommandSpec::new("guides.remove", "Remove Guide", guides_remove::<D>).with_schema(SCHEMA_GUIDE_REMOVE).describe("Remove one guide."),
        CommandSpec::new("guides.clear", "Clear Guides", guides_clear::<D>).with_schema(SCHEMA_NONE).describe("View > Guides > Clear Guides: remove every guide."),
        CommandSpec::new("guides.set", "Edit Guides", guides_set::<D>)
            .transient()
            .with_schema(SCHEMA_GUIDE_SET)
            .describe("The Edit Guides dialog: show or hide the guides, lock them, change their colour, or replace their positions."),
        CommandSpec::new("grid.set", "Edit Grid", grid_set::<D>).transient().with_schema(SCHEMA_GRID).describe("The Edit Grid dialog: show or hide the grid, its spacing in pixels and its colour."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_json_objects() {
        for s in
            [SCHEMA_EDGE, SCHEMA_SPACE, SCHEMA_MATCH, SCHEMA_APPLY, SCHEMA_GUIDE_ADD, SCHEMA_GUIDE_MOVE, SCHEMA_GUIDE_REMOVE, SCHEMA_GUIDE_SET, SCHEMA_GRID]
        {
            let v: Value = serde_json::from_str(s).unwrap();
            assert_eq!(v["type"], "object");
        }
    }
}
