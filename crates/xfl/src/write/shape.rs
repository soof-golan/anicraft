//! `<DOMShape>`: fill and stroke styles plus the `<Edge>` list.

use anim_core::{Affine, Rgba8};
use anim_scene::ShapeElement;
use anim_shape::{BitmapFill, FillStyle, GradientFill, SolidStroke, StrokeStyle, XflName};

use super::out::{Attrs, XmlOut};
use super::scene::{matrix, point};

/// Writes a shape element.
pub fn write_shape(out: &mut XmlOut, shape: &ShapeElement) {
    out.open("DOMShape", &Attrs::new().b_unless("isDrawingObject", shape.is_drawing_object, false).extra(&shape.extra));
    // Merge-mode shapes carry no matrix in XFL; the reader gives them the identity.
    if shape.matrix != Affine::IDENTITY {
        matrix(out, shape.matrix);
    }
    if let Some(p) = shape.transformation_point {
        point(out, "transformationPoint", p);
    }
    let inner = &shape.shape;
    if !inner.fills.is_empty() {
        out.open("fills", &Attrs::new());
        for (i, fill) in inner.fills.iter().enumerate() {
            out.open("FillStyle", &Attrs::new().u("index", index_of(i)));
            fill_content(out, fill);
            out.close();
        }
        out.close();
    }
    if !inner.strokes.is_empty() {
        out.open("strokes", &Attrs::new());
        for (i, stroke) in inner.strokes.iter().enumerate() {
            out.open("StrokeStyle", &Attrs::new().u("index", index_of(i)));
            stroke_content(out, stroke);
            out.close();
        }
        out.close();
    }
    let edges = inner.to_xfl_edges();
    if !edges.is_empty() {
        out.open("edges", &Attrs::new());
        for edge in &edges {
            let attrs = Attrs::new()
                .u_unless("fillStyle0", edge.fill_style0, 0)
                .u_unless("fillStyle1", edge.fill_style1, 0)
                .u_unless("strokeStyle", edge.stroke_style, 0)
                .s_opt("edges", edge.edges.as_deref())
                .s_opt("cubics", edge.cubics.as_deref());
            out.leaf("Edge", &attrs);
        }
        out.close();
    }
    for child in &shape.extra.children {
        out.raw("DOMShape", child);
    }
    out.close();
}

/// XFL style indices count from 1.
fn index_of(position: usize) -> u32 {
    u32::try_from(position).unwrap_or(u32::MAX).saturating_add(1)
}

fn fill_content(out: &mut XmlOut, fill: &FillStyle) {
    match fill {
        FillStyle::Solid(color) => solid_color(out, *color),
        FillStyle::LinearGradient(g) => gradient(out, "LinearGradient", g),
        FillStyle::RadialGradient(g) => gradient(out, "RadialGradient", g),
        FillStyle::Bitmap(b) => bitmap_fill(out, b),
    }
}

fn solid_color(out: &mut XmlOut, color: Rgba8) {
    out.leaf("SolidColor", &Attrs::new().color_unless("color", color, Rgba8::BLACK).alpha("alpha", color));
}

fn gradient(out: &mut XmlOut, tag: &'static str, g: &GradientFill) {
    let attrs = Attrs::new()
        .s_unless("spreadMethod", g.spread.xfl_name(), "extend")
        .s_unless("interpolationMethod", g.interpolation.xfl_name(), "RGB")
        .f_unless("focalPointRatio", g.focal, 0.0);
    out.open(tag, &attrs);
    matrix(out, g.matrix);
    for stop in &g.stops {
        let entry = Attrs::new().color_unless("color", stop.color, Rgba8::BLACK).alpha("alpha", stop.color).f32("ratio", stop.offset);
        out.leaf("GradientEntry", &entry);
    }
    out.close();
}

fn bitmap_fill(out: &mut XmlOut, b: &BitmapFill) {
    let attrs = Attrs::new().s("bitmapPath", &b.path).b_unless("bitmapIsClipped", b.clipped, false).b_unless("allowSmoothing", b.smoothing, true);
    out.open("BitmapFill", &attrs);
    matrix(out, b.matrix);
    out.close();
}

fn stroke_content(out: &mut XmlOut, stroke: &StrokeStyle) {
    match stroke {
        StrokeStyle::Solid(s) => stroke_element(out, "SolidStroke", s, &[]),
        StrokeStyle::Other(o) => stroke_element(out, o.kind.element_name(), &o.base, &o.attrs),
    }
}

fn stroke_element(out: &mut XmlOut, tag: &'static str, s: &SolidStroke, extra: &[(String, String)]) {
    let default = SolidStroke::default();
    let attrs = Attrs::new()
        .s("scaleMode", s.scale_mode.xfl_name())
        .f_unless("weight", s.weight, default.weight)
        .s_unless("caps", s.caps.xfl_name(), default.caps.xfl_name())
        .s_unless("joints", s.joints.xfl_name(), default.joints.xfl_name())
        .f_unless("miterLimit", s.miter_limit, default.miter_limit)
        .b_unless("pixelHinting", s.pixel_hinting, false)
        .pairs(extra);
    out.open(tag, &attrs);
    out.open("fill", &Attrs::new());
    fill_content(out, &s.fill);
    out.close();
    out.close();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::{Ctx, Sink};

    fn round_trip(text: &str) -> (ShapeElement, String) {
        let doc = roxmltree::Document::parse(text).unwrap();
        let mut sink = Sink::default();
        let shape = {
            let mut ctx = Ctx::new(text, "t.xml", &mut sink);
            crate::shape::read_shape(&mut ctx, doc.root_element())
        };
        let mut out = XmlOut::new();
        write_shape(&mut out, &shape);
        (shape, out.finish().0)
    }

    #[test]
    fn shapes_read_back_equal() {
        let source = r##"<DOMShape isDrawingObject="true" foo="bar"><matrix><Matrix a="2" tx="5"/></matrix><transformationPoint><Point x="1"/></transformationPoint>
        <fills><FillStyle index="1"><SolidColor color="#FF0000" alpha="0.5"/></FillStyle>
        <FillStyle index="2"><RadialGradient spreadMethod="reflect" focalPointRatio="0.5"><matrix><Matrix a="0.01"/></matrix><GradientEntry color="#FFFFFF" ratio="0"/><GradientEntry alpha="0.25" ratio="1"/></RadialGradient></FillStyle>
        <FillStyle index="3"><BitmapFill bitmapPath="p" bitmapIsClipped="true" allowSmoothing="false"><matrix><Matrix a="20" d="20"/></matrix></BitmapFill></FillStyle></fills>
        <strokes><StrokeStyle index="1"><SolidStroke scaleMode="none" weight="2" caps="square" joints="bevel" miterLimit="8" pixelHinting="true"><fill><SolidColor color="#00FF00"/></fill></SolidStroke></StrokeStyle>
        <StrokeStyle index="2"><DashedStroke scaleMode="normal" weight="3" dash1="4"><fill><SolidColor/></fill></DashedStroke></StrokeStyle></strokes>
        <edges><Edge fillStyle1="1" strokeStyle="1" edges="!0 0|100 0|100 100|0 0"/><Edge fillStyle0="2" strokeStyle="2" edges="!0 0[100 0 100 100!100 100|0 100" cubics="!0 0(;0,0 50,0 100,50q0 0 100 50);"/></edges><weird a="1"/></DOMShape>"##;
        let (shape, written) = round_trip(source);
        let (again, _) = round_trip(&written);
        assert_eq!(shape, again, "{written}");
        assert!(written.contains("<weird a=\"1\"/>"));
        assert!(written.contains("foo=\"bar\""));
    }
}
