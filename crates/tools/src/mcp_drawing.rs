//! MCP commands for the drawing tools of the second half: `draw.rect_primitive`, `draw.oval_primitive`,
//! `draw.polystar`, `primitive.set`, `select.lasso`, `select.wand`, `anchor.add`, `anchor.delete` and
//! `anchor.convert`.
//!
//! Like the other façades they build the events of the matching tool and run them through the editor, so a call
//! and the mouse behave identically; coordinates are stage pixels, y down, angles are degrees.

#![allow(clippy::needless_raw_string_hashes, reason = "the JSON schemas are easier to read and edit with uniform raw string delimiters")]

use anim_core::Point;
use anim_scene::{Element, Primitive, PrimitiveKind};
use anim_session::{CommandError, CommandSpec, parse_params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::command::ToolCommand;
use crate::mcp::{ToolHost, atomic, down, drag, draw_box, fail, finite_all, result_of, run, split_style, up, with_style};
use crate::script::ToolScript;
use crate::settings::PolyStarStyle;
use crate::tool::ToolId;

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::invalid_params(message.into())
}

fn draw_rect_primitive<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    draw_box(host, params, ToolId::RectanglePrimitive)
}

fn draw_oval_primitive<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    draw_box(host, params, ToolId::OvalPrimitive)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolyStarParams {
    x: f64,
    y: f64,
    radius: f64,
    sides: Option<u32>,
    star: Option<bool>,
    point_size: Option<f64>,
    rotation_degrees: Option<f64>,
}

fn draw_polystar<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let (style, rest) = split_style(params)?;
    let p: PolyStarParams = parse_params(&rest)?;
    finite_all(&[p.x, p.y, p.radius, p.point_size.unwrap_or(0.5), p.rotation_degrees.unwrap_or(0.0)])?;
    if p.radius <= 0.0 {
        return Err(invalid("radius must be positive"));
    }
    if p.sides.is_some_and(|s| !(3..=32).contains(&s)) {
        return Err(invalid("sides must be between 3 and 32"));
    }
    if p.point_size.is_some_and(|s| !(0.0..=1.0).contains(&s)) {
        return Err(invalid("point_size must be between 0 and 1"));
    }
    let angle = p.rotation_degrees.unwrap_or(0.0).to_radians();
    let (end_x, end_y) = (p.radius.mul_add(angle.cos(), p.x), p.radius.mul_add(angle.sin(), p.y));
    atomic(host.editor_mut(), |ed| {
        with_style(ed, &style, |ed| {
            let options = &mut ed.state.settings.polystar;
            if let Some(sides) = p.sides {
                options.sides = sides;
            }
            if let Some(star) = p.star {
                options.style = if star { PolyStarStyle::Star } else { PolyStarStyle::Polygon };
            }
            if let Some(size) = p.point_size {
                options.star_point_size = size;
            }
            let script = ToolScript { tool: Some(ToolId::PolyStar), events: vec![down(p.x, p.y), drag(f64::midpoint(p.x, end_x), f64::midpoint(p.y, end_y)), up(end_x, end_y)] };
            let report = run(ed, &script)?;
            Ok(result_of(ed, &report))
        })
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrimitiveParams {
    width: Option<f64>,
    height: Option<f64>,
    /// One radius for all four corners.
    corner_radius: Option<f64>,
    /// Top left, top right, bottom right, bottom left.
    radii: Option<[f64; 4]>,
    lock: Option<bool>,
    start_angle: Option<f64>,
    end_angle: Option<f64>,
    inner_radius: Option<f64>,
    close_path: Option<bool>,
}

/// The primitive with the given changes; the ones that do not apply to its kind are ignored.
fn patched(primitive: &Primitive, p: &PrimitiveParams) -> Primitive {
    let mut out = primitive.clone();
    match &mut out.kind {
        PrimitiveKind::Rectangle(r) => {
            r.width = p.width.unwrap_or(r.width);
            r.height = p.height.unwrap_or(r.height);
            if let Some(radius) = p.corner_radius {
                r.radii = [radius; 4];
            }
            if let Some(radii) = p.radii {
                r.radii = radii;
            }
            r.lock = p.lock.unwrap_or(r.lock);
        }
        PrimitiveKind::Oval(o) => {
            o.width = p.width.unwrap_or(o.width);
            o.height = p.height.unwrap_or(o.height);
            o.start_angle = p.start_angle.unwrap_or(o.start_angle);
            o.end_angle = p.end_angle.unwrap_or(o.end_angle);
            o.inner_radius = p.inner_radius.unwrap_or(o.inner_radius).clamp(0.0, 99.0);
            o.close_path = p.close_path.unwrap_or(o.close_path);
        }
    }
    out
}

fn primitive_set<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: PrimitiveParams = parse_params(params)?;
    finite_all(&[
        p.width.unwrap_or(0.0),
        p.height.unwrap_or(0.0),
        p.corner_radius.unwrap_or(0.0),
        p.start_angle.unwrap_or(0.0),
        p.end_angle.unwrap_or(0.0),
        p.inner_radius.unwrap_or(0.0),
    ])?;
    if p.radii.is_some_and(|r| r.iter().any(|v| !v.is_finite())) {
        return Err(invalid("radii must be finite numbers"));
    }
    if p.width.is_some_and(|w| w < 0.0) || p.height.is_some_and(|h| h < 0.0) {
        return Err(invalid("width and height must not be negative"));
    }
    atomic(host.editor_mut(), |ed| {
        let mut commands = Vec::new();
        for target in &ed.state.selection.items {
            let current = ed.with_context(|ctx| match ctx.resolve(target)?.element {
                Element::Shape(s) => s.primitive.clone(),
                _ => None,
            });
            if let Some(current) = current {
                commands.push(ToolCommand::SetPrimitive { target: target.clone(), primitive: patched(&current, &p) });
            }
        }
        if commands.is_empty() {
            return Err(invalid("select a rectangle or oval primitive first"));
        }
        ed.run_commands(&commands).map_err(fail)?;
        Ok(json!({ "updated": commands.len() }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LassoParams {
    points: Vec<[f64; 2]>,
    #[serde(default)]
    add: bool,
}

fn select_lasso<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: LassoParams = parse_params(params)?;
    if p.points.len() < 3 {
        return Err(invalid("a lasso needs at least three points"));
    }
    if p.points.iter().any(|q| !q[0].is_finite() || !q[1].is_finite()) {
        return Err(invalid("coordinates must be finite numbers"));
    }
    let polygon = p.points.iter().map(|q| Point::new(q[0], q[1])).collect();
    atomic(host.editor_mut(), |ed| {
        ed.run_commands(&[ToolCommand::LassoSelect { polygon, additive: p.add }]).map_err(fail)?;
        Ok(json!({ "selected": ed.state.selection.items.len() }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WandParams {
    x: f64,
    y: f64,
    threshold: Option<f64>,
    #[serde(default)]
    add: bool,
}

fn select_wand<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: WandParams = parse_params(params)?;
    finite_all(&[p.x, p.y, p.threshold.unwrap_or(0.0)])?;
    atomic(host.editor_mut(), |ed| {
        let threshold = p.threshold.unwrap_or(ed.state.settings.lasso.threshold);
        ed.run_commands(&[ToolCommand::WandSelect { point: Point::new(p.x, p.y), threshold, additive: p.add }]).map_err(fail)?;
        Ok(json!({ "selected": ed.state.selection.items.len() }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorParams {
    x: f64,
    y: f64,
    /// Convert only: pull the handles out by this stage offset.
    pull: Option<[f64; 2]>,
}

fn anchor_with<D: ToolHost>(host: &mut D, params: &Value, tool: ToolId) -> Result<Value, CommandError> {
    let p: AnchorParams = parse_params(params)?;
    finite_all(&[p.x, p.y])?;
    if p.pull.is_some() && tool != ToolId::ConvertAnchor {
        return Err(invalid("only anchor.convert takes pull"));
    }
    let events = match p.pull {
        Some([dx, dy]) => {
            finite_all(&[dx, dy])?;
            vec![down(p.x, p.y), drag(p.x + dx / 2.0, p.y + dy / 2.0), up(p.x + dx, p.y + dy)]
        }
        None => vec![down(p.x, p.y), up(p.x, p.y)],
    };
    atomic(host.editor_mut(), |ed| {
        let before = ed.document.clone();
        let report = run(ed, &ToolScript { tool: Some(tool), events })?;
        if ed.document == before {
            return Err(invalid("there is no matching anchor or segment at that point"));
        }
        Ok(result_of(ed, &report))
    })
}

fn anchor_add<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    anchor_with(host, params, ToolId::AddAnchor)
}

fn anchor_delete<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    anchor_with(host, params, ToolId::DeleteAnchor)
}

fn anchor_convert<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    anchor_with(host, params, ToolId::ConvertAnchor)
}

const SCHEMA_BOX: &str = r##"{"type":"object","required":["x","y","width","height"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number","exclusiveMinimum":0},"height":{"type":"number","exclusiveMinimum":0},"corner_radius":{"type":"number","minimum":0},"fill":{"type":"string"},"stroke":{"oneOf":[{"type":"string"},{"type":"object","properties":{"color":{"type":"string"},"width":{"type":"number","minimum":0}},"additionalProperties":false}]},"object_drawing":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_POLYSTAR: &str = r##"{"type":"object","required":["x","y","radius"],"properties":{"x":{"type":"number","description":"Centre"},"y":{"type":"number"},"radius":{"type":"number","exclusiveMinimum":0},"sides":{"type":"integer","minimum":3,"maximum":32},"star":{"type":"boolean"},"point_size":{"type":"number","minimum":0,"maximum":1,"description":"Star point size: inner radius as a fraction of the outer one"},"rotation_degrees":{"type":"number"},"fill":{"type":"string"},"stroke":{"oneOf":[{"type":"string"},{"type":"object","properties":{"color":{"type":"string"},"width":{"type":"number","minimum":0}},"additionalProperties":false}]},"object_drawing":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_PRIMITIVE: &str = r##"{"type":"object","description":"Change the selected rectangle or oval primitives","properties":{"width":{"type":"number","minimum":0},"height":{"type":"number","minimum":0},"corner_radius":{"type":"number","minimum":0},"radii":{"type":"array","items":{"type":"number"},"minItems":4,"maxItems":4,"description":"top left, top right, bottom right, bottom left"},"lock":{"type":"boolean"},"start_angle":{"type":"number"},"end_angle":{"type":"number"},"inner_radius":{"type":"number","minimum":0,"maximum":99},"close_path":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_LASSO: &str = r##"{"type":"object","required":["points"],"properties":{"points":{"type":"array","minItems":3,"items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}},"add":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_WAND: &str = r##"{"type":"object","required":["x","y"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"threshold":{"type":"number","minimum":0,"maximum":255},"add":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_ANCHOR: &str = r##"{"type":"object","required":["x","y"],"properties":{"x":{"type":"number"},"y":{"type":"number"}},"additionalProperties":false}"##;
const SCHEMA_CONVERT: &str = r##"{"type":"object","required":["x","y"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"pull":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2,"description":"Drag the handles out by this offset instead of toggling corner and smooth"}},"additionalProperties":false}"##;

/// The commands of this module.
pub fn commands<D: ToolHost>() -> Vec<CommandSpec<D>> {
    vec![
        CommandSpec::new("draw.rect_primitive", "Draw Rectangle Primitive", draw_rect_primitive::<D>)
            .with_schema(SCHEMA_BOX)
            .describe("Draw a rectangle primitive that stays editable (see primitive.set); corner_radius rounds all four corners."),
        CommandSpec::new("draw.oval_primitive", "Draw Oval Primitive", draw_oval_primitive::<D>)
            .with_schema(SCHEMA_BOX)
            .describe("Draw an oval primitive that stays editable (see primitive.set); the arc comes from the oval tool options."),
        CommandSpec::new("draw.polystar", "Draw PolyStar", draw_polystar::<D>)
            .with_schema(SCHEMA_POLYSTAR)
            .describe("Draw a polygon or star around a centre with the PolyStar tool."),
        CommandSpec::new("primitive.set", "Set Primitive", primitive_set::<D>)
            .with_schema(SCHEMA_PRIMITIVE)
            .describe("Change the size, corner radii, start and end angle or inner radius of the selected rectangle or oval primitives."),
        CommandSpec::new("select.lasso", "Lasso Select", select_lasso::<D>)
            .with_schema(SCHEMA_LASSO)
            .describe("Select what lies inside a polygon: shapes are cut along it and the part inside is selected, whole objects are selected when it encloses them."),
        CommandSpec::new("select.wand", "Magic Wand Select", select_wand::<D>)
            .with_schema(SCHEMA_WAND)
            .describe("Select the fills whose colour is within a threshold of the fill at a point."),
        CommandSpec::new("anchor.add", "Add Anchor Point", anchor_add::<D>)
            .with_schema(SCHEMA_ANCHOR)
            .describe("Add an anchor point on the path under the given point without changing its shape."),
        CommandSpec::new("anchor.delete", "Delete Anchor Point", anchor_delete::<D>)
            .with_schema(SCHEMA_ANCHOR)
            .describe("Delete the anchor point at the given point; its neighbours are joined."),
        CommandSpec::new("anchor.convert", "Convert Anchor Point", anchor_convert::<D>)
            .with_schema(SCHEMA_CONVERT)
            .describe("Convert the anchor point at the given point between corner and smooth, or pull its handles out with pull."),
    ]
}
