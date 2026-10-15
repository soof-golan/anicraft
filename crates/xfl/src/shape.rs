//! `<DOMShape>`: fill and stroke styles plus the flat list of `<Edge>` elements.

use anim_core::{Affine, Rgba8};
use anim_display::{Cap, GradientStop, Interpolation, Join, Spread, StrokeScale};
use anim_scene::{Extra, ShapeElement};
use anim_shape::{BitmapFill, EdgeShape, FillStyle, GradientFill, OtherStroke, SolidStroke, StrokeKind, StrokeStyle, XflEdge, XflName};
use roxmltree::Node;

use crate::xml::{Attrs, Ctx, element_children, read_matrix, read_point};

/// Most `<Edge>` elements read for one shape; more are dropped with a warning.
pub const MAX_EDGES_PER_SHAPE: usize = 2_000_000;
/// Most style slots (the largest `index` honoured).
const MAX_STYLES: u32 = 10_000;
const MAX_STYLE_COUNT: usize = 10_000;

/// Reads a `<DOMShape>`.
pub fn read_shape(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> ShapeElement {
    let mut attrs = Attrs::new(node);
    let is_drawing_object = attrs.bool(ctx, "isDrawingObject").unwrap_or(false);
    let mut shape = ShapeElement { is_drawing_object, ..ShapeElement::default() };
    let mut extra = Extra::new();
    let mut fills = Vec::new();
    let mut strokes = Vec::new();
    let mut edges: Vec<XflEdge> = Vec::new();
    for c in element_children(node) {
        match c.tag_name().name() {
            "matrix" => shape.matrix = read_matrix(ctx, c),
            "transformationPoint" => shape.transformation_point = Some(read_point(ctx, c)),
            "fills" => fills = read_fill_list(ctx, c),
            "strokes" => strokes = read_stroke_list(ctx, c),
            "edges" => read_edges(ctx, c, &mut edges),
            _ => ctx.keep_child(&mut extra, "DOMShape", c),
        }
    }
    attrs.rest(ctx, &mut extra);
    let (edge_shape, errors) = EdgeShape::from_xfl_lenient(fills, strokes, &edges);
    for error in errors {
        ctx.warn(format!("DOMShape: {error}"));
    }
    shape.shape = edge_shape;
    shape.extra = extra;
    shape
}

fn read_edges(ctx: &mut Ctx<'_>, node: Node<'_, '_>, out: &mut Vec<XflEdge>) {
    for c in element_children(node) {
        if c.tag_name().name() != "Edge" {
            ctx.lost_attr("edges", c.tag_name().name());
            continue;
        }
        if out.len() >= MAX_EDGES_PER_SHAPE {
            ctx.warn("DOMShape: too many edges, the rest are dropped");
            break;
        }
        let mut attrs = Attrs::new(c);
        let fill_style0 = attrs.u32(ctx, "fillStyle0").unwrap_or(0);
        let fill_style1 = attrs.u32(ctx, "fillStyle1").unwrap_or(0);
        let stroke_style = attrs.u32(ctx, "strokeStyle").unwrap_or(0);
        let edges = attrs.string("edges");
        let cubics = attrs.string("cubics");
        attrs.rest_lost(ctx);
        out.push(XflEdge { fill_style0, fill_style1, stroke_style, edges, cubics });
    }
}

/// Puts `(index, style)` pairs into a vector addressed by the 1-based index.
fn place_by_index<T: Clone>(ctx: &mut Ctx<'_>, what: &str, items: Vec<(Option<u32>, T)>, filler: &T) -> Vec<T> {
    let sequential = items.iter().enumerate().all(|(i, (index, _))| index.is_none_or(|n| usize::try_from(n).ok() == i.checked_add(1)));
    if sequential {
        return items.into_iter().map(|(_, v)| v).collect();
    }
    ctx.warn(format!("{what} indices are not 1..n in order; placing them by index"));
    let mut out: Vec<T> = Vec::new();
    for (position, (index, value)) in items.into_iter().enumerate() {
        let slot = index.map_or_else(|| position.saturating_add(1), |n| usize::try_from(n.min(MAX_STYLES)).unwrap_or(0));
        let Some(zero_based) = slot.checked_sub(1) else {
            continue;
        };
        if out.len() <= zero_based {
            out.resize(zero_based.saturating_add(1), filler.clone());
        }
        if let Some(cell) = out.get_mut(zero_based) {
            *cell = value;
        }
    }
    out
}

fn read_fill_list(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> Vec<FillStyle> {
    let mut items = Vec::new();
    for c in element_children(node) {
        if c.tag_name().name() != "FillStyle" {
            ctx.lost_attr("fills", c.tag_name().name());
            continue;
        }
        if items.len() >= MAX_STYLE_COUNT {
            ctx.warn("DOMShape: too many fill styles");
            break;
        }
        let mut attrs = Attrs::new(c);
        let index = attrs.u32(ctx, "index");
        attrs.rest_lost(ctx);
        items.push((index, read_fill_content(ctx, c)));
    }
    place_by_index(ctx, "fill style", items, &FillStyle::Solid(Rgba8::TRANSPARENT))
}

/// The paint inside a `<FillStyle>` or a stroke's `<fill>`: the first recognised child.
fn read_fill_content(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> FillStyle {
    let mut result = None;
    for c in element_children(node) {
        let parsed = match c.tag_name().name() {
            "SolidColor" => Some(read_solid(ctx, c)),
            "LinearGradient" => Some(FillStyle::LinearGradient(read_gradient(ctx, c))),
            "RadialGradient" => Some(FillStyle::RadialGradient(read_gradient(ctx, c))),
            "BitmapFill" => Some(read_bitmap_fill(ctx, c)),
            other => {
                ctx.lost_attr(node.tag_name().name(), other);
                None
            }
        };
        if result.is_none() {
            result = parsed;
        }
    }
    result.unwrap_or_else(|| {
        ctx.warn(format!("{} without a known paint", node.tag_name().name()));
        FillStyle::Solid(Rgba8::TRANSPARENT)
    })
}

fn read_solid(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> FillStyle {
    let mut attrs = Attrs::new(node);
    let color = attrs.color(ctx, "color").unwrap_or(Rgba8::BLACK);
    let alpha = attrs.f32(ctx, "alpha").unwrap_or(1.0);
    attrs.rest_lost(ctx);
    FillStyle::Solid(color.with_alpha_f32(alpha))
}

fn read_gradient(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> GradientFill {
    let mut attrs = Attrs::new(node);
    let spread = attrs.take_with(ctx, "spreadMethod", Spread::from_xfl).unwrap_or(Spread::Pad);
    let interpolation = attrs.take_with(ctx, "interpolationMethod", Interpolation::from_xfl).unwrap_or(Interpolation::Rgb);
    let focal = attrs.f64(ctx, "focalPointRatio").unwrap_or(0.0);
    attrs.rest_lost(ctx);
    let mut gradient = GradientFill { spread, interpolation, focal, ..GradientFill::default() };
    for c in element_children(node) {
        match c.tag_name().name() {
            "matrix" => gradient.matrix = read_matrix(ctx, c),
            "GradientEntry" => {
                let mut entry = Attrs::new(c);
                let color = entry.color(ctx, "color").unwrap_or(Rgba8::BLACK);
                let alpha = entry.f32(ctx, "alpha").unwrap_or(1.0);
                let ratio = entry.f32(ctx, "ratio").unwrap_or(0.0);
                entry.rest_lost(ctx);
                gradient.stops.push(GradientStop { offset: ratio.clamp(0.0, 1.0), color: color.with_alpha_f32(alpha) });
            }
            other => ctx.lost_attr("LinearGradient", other),
        }
    }
    gradient.stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    gradient
}

fn read_bitmap_fill(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> FillStyle {
    let mut attrs = Attrs::new(node);
    let path = attrs.string("bitmapPath").unwrap_or_default();
    let clipped = attrs.bool(ctx, "bitmapIsClipped").unwrap_or(false);
    let smoothing = attrs.bool(ctx, "allowSmoothing").unwrap_or(true);
    attrs.rest_lost(ctx);
    let mut matrix = Affine::IDENTITY;
    for c in element_children(node) {
        if c.tag_name().name() == "matrix" {
            matrix = read_matrix(ctx, c);
        } else {
            ctx.lost_attr("BitmapFill", c.tag_name().name());
        }
    }
    FillStyle::Bitmap(BitmapFill { path, matrix, clipped, smoothing })
}

fn read_stroke_list(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> Vec<StrokeStyle> {
    let mut items = Vec::new();
    for c in element_children(node) {
        if c.tag_name().name() != "StrokeStyle" {
            ctx.lost_attr("strokes", c.tag_name().name());
            continue;
        }
        if items.len() >= MAX_STYLE_COUNT {
            ctx.warn("DOMShape: too many stroke styles");
            break;
        }
        let mut attrs = Attrs::new(c);
        let index = attrs.u32(ctx, "index");
        attrs.rest_lost(ctx);
        items.push((index, read_stroke_content(ctx, c)));
    }
    place_by_index(ctx, "stroke style", items, &StrokeStyle::default())
}

fn read_stroke_content(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> StrokeStyle {
    for c in element_children(node) {
        let name = c.tag_name().name();
        if name == "SolidStroke" {
            let (stroke, _) = read_solid_stroke(ctx, c, false);
            return StrokeStyle::Solid(stroke);
        }
        if let Some(kind) = StrokeKind::from_element(name) {
            let (base, attrs) = read_solid_stroke(ctx, c, true);
            return StrokeStyle::Other(OtherStroke { kind, base, attrs });
        }
        ctx.lost_attr("StrokeStyle", name);
    }
    ctx.warn("StrokeStyle without a stroke");
    StrokeStyle::default()
}

/// Reads the attributes every stroke kind shares. With `keep_rest` the remaining attributes are returned (for the
/// patterned kinds); otherwise they are counted as lost.
fn read_solid_stroke(ctx: &mut Ctx<'_>, node: Node<'_, '_>, keep_rest: bool) -> (SolidStroke, Vec<(String, String)>) {
    let mut attrs = Attrs::new(node);
    let default = SolidStroke::default();
    let weight = attrs.f64(ctx, "weight").unwrap_or(1.0).max(0.0);
    let scale_mode = attrs.take_with(ctx, "scaleMode", StrokeScale::from_xfl).unwrap_or(default.scale_mode);
    let caps = attrs.take_with(ctx, "caps", Cap::from_xfl).unwrap_or(default.caps);
    let joints = attrs.take_with(ctx, "joints", Join::from_xfl).unwrap_or(default.joints);
    let miter_limit = attrs.f64(ctx, "miterLimit").unwrap_or(default.miter_limit);
    let pixel_hinting = attrs.bool(ctx, "pixelHinting").unwrap_or(false);
    let rest = if keep_rest {
        let mut extra = Extra::new();
        attrs.rest(ctx, &mut extra);
        extra.attrs
    } else {
        attrs.rest_lost(ctx);
        Vec::new()
    };
    let mut fill = FillStyle::default();
    for c in element_children(node) {
        if c.tag_name().name() == "fill" {
            fill = read_fill_content(ctx, c);
        } else {
            ctx.lost_attr(node.tag_name().name(), c.tag_name().name());
        }
    }
    (SolidStroke { weight, fill, caps, joints, miter_limit, scale_mode, pixel_hinting }, rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Sink;

    fn parse(text: &str) -> (ShapeElement, Sink) {
        let doc = roxmltree::Document::parse(text).unwrap();
        let mut sink = Sink::default();
        let shape = {
            let mut ctx = Ctx::new(text, "t.xml", &mut sink);
            read_shape(&mut ctx, doc.root_element())
        };
        (shape, sink)
    }

    #[test]
    fn solid_fill_stroke_and_edges() {
        let (shape, sink) = parse(
            r##"<DOMShape isDrawingObject="true"><fills><FillStyle index="1"><SolidColor color="#FF0000" alpha="0.5"/></FillStyle></fills>
            <strokes><StrokeStyle index="1"><SolidStroke scaleMode="normal" weight="2" caps="none"><fill><SolidColor color="#00FF00"/></fill></SolidStroke></StrokeStyle></strokes>
            <edges><Edge fillStyle1="1" strokeStyle="1" edges="!0 0|100 0|100 100|0 100|0 0"/></edges></DOMShape>"##,
        );
        assert!(shape.is_drawing_object);
        assert_eq!(shape.shape.fills.len(), 1);
        assert_eq!(shape.shape.fills.first().and_then(FillStyle::solid_color), Some(Rgba8::new(255, 0, 0, 128)));
        let stroke = shape.shape.strokes.first().unwrap().solid();
        assert!((stroke.weight - 2.0).abs() < 1e-9);
        assert_eq!(stroke.caps, Cap::None);
        assert_eq!(stroke.fill.solid_color(), Some(Rgba8::opaque(0, 255, 0)));
        assert_eq!(shape.shape.edges.len(), 1);
        assert!(shape.extra.is_empty());
        assert_eq!(sink.unknowns.total(), 0);
    }

    #[test]
    fn gradients_sort_stops_and_keep_matrix() {
        let (shape, _) = parse(
            r##"<DOMShape><fills><FillStyle index="1"><LinearGradient spreadMethod="reflect"><matrix><Matrix a="2" tx="5"/></matrix>
            <GradientEntry color="#FFFFFF" ratio="1"/><GradientEntry color="#000000" ratio="0.25" alpha="0.5"/></LinearGradient></FillStyle></fills></DOMShape>"##,
        );
        let Some(FillStyle::LinearGradient(g)) = shape.shape.fills.first() else {
            panic!("expected a linear gradient");
        };
        assert_eq!(g.spread, Spread::Reflect);
        assert_eq!(g.stops.len(), 2);
        assert!((g.stops.first().unwrap().offset - 0.25).abs() < 1e-6);
        assert!((g.matrix.as_coeffs()[0] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_children_and_attributes_are_kept() {
        let (shape, sink) = parse(r#"<DOMShape selected="true" foo="bar"><weird a="1"/></DOMShape>"#);
        assert_eq!(shape.extra.attr("foo"), Some("bar"));
        assert_eq!(shape.extra.children, vec![r#"<weird a="1"/>"#.to_owned()]);
        assert_eq!(sink.unknowns.ui_state, 1);
        assert_eq!(sink.unknowns.top(5).len(), 2);
    }

    #[test]
    fn style_indices_out_of_order_are_placed_by_index() {
        let (shape, sink) = parse(r##"<DOMShape><fills><FillStyle index="3"><SolidColor color="#0000FF"/></FillStyle></fills></DOMShape>"##);
        assert_eq!(shape.shape.fills.len(), 3);
        assert_eq!(shape.shape.fills.get(2).and_then(FillStyle::solid_color), Some(Rgba8::opaque(0, 0, 255)));
        assert!(!sink.warnings.is_empty());
    }

    #[test]
    fn patterned_strokes_keep_their_attributes() {
        let (shape, _) = parse(
            r#"<DOMShape><strokes><StrokeStyle index="1"><DashedStroke scaleMode="normal" weight="3" dash1="4" gap1="2"><fill><SolidColor/></fill></DashedStroke></StrokeStyle></strokes></DOMShape>"#,
        );
        let Some(StrokeStyle::Other(o)) = shape.shape.strokes.first() else {
            panic!("expected a patterned stroke");
        };
        assert_eq!(o.kind, StrokeKind::Dashed);
        assert_eq!(o.attrs, vec![("dash1".to_owned(), "4".to_owned()), ("gap1".to_owned(), "2".to_owned())]);
    }
}
