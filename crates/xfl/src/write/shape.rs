//! `<DOMShape>`: fill and stroke styles plus the `<Edge>` list.

use anim_core::{Affine, Rgba8};
use anim_scene::{Primitive, PrimitiveKind, ShapeElement};
use anim_shape::{BitmapFill, FillStyle, GradientFill, SolidStroke, StrokeStyle, XflName};

use super::out::{Attrs, XmlOut};
use super::scene::{matrix, point};

/// Writes a shape element.
pub fn write_shape(out: &mut XmlOut, shape: &ShapeElement) {
    if let Some(primitive) = &shape.primitive {
        write_primitive(out, shape, primitive);
        return;
    }
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

/// Writes a Rectangle or Oval Primitive as `<DOMRectangleObject>` or `<DOMOvalObject>`.
fn write_primitive(out: &mut XmlOut, shape: &ShapeElement, primitive: &Primitive) {
    let (tag, attrs) = match &primitive.kind {
        PrimitiveKind::Rectangle(r) => {
            let [tl, tr, br, bl] = r.radii;
            let attrs = Attrs::new()
                .f("objectWidth", r.width)
                .f("objectHeight", r.height)
                .f_unless("topLeftRadius", tl, 0.0)
                .f_unless("topRightRadius", tr, 0.0)
                .f_unless("bottomRightRadius", br, 0.0)
                .f_unless("bottomLeftRadius", bl, 0.0)
                .b_unless("lockFlag", r.lock, true);
            ("DOMRectangleObject", attrs)
        }
        PrimitiveKind::Oval(o) => {
            let attrs = Attrs::new()
                .f("objectWidth", o.width)
                .f("objectHeight", o.height)
                .f_unless("startAngle", o.start_angle, 0.0)
                .f_unless("endAngle", o.end_angle, 0.0)
                .f_unless("innerRadius", o.inner_radius, 0.0)
                .b_unless("closePath", o.close_path, true);
            ("DOMOvalObject", attrs)
        }
    };
    out.open(tag, &attrs.extra(&shape.extra));
    if shape.matrix != Affine::IDENTITY {
        matrix(out, shape.matrix);
    }
    if let Some(p) = shape.transformation_point {
        point(out, "transformationPoint", p);
    }
    if let Some(fill) = &primitive.fill {
        out.open("fill", &Attrs::new());
        out.open("FillStyle", &Attrs::new().u("index", 1));
        fill_content(out, fill);
        out.close();
        out.close();
    }
    if let Some(stroke) = &primitive.stroke {
        out.open("stroke", &Attrs::new());
        out.open("StrokeStyle", &Attrs::new().u("index", 1));
        stroke_content(out, stroke);
        out.close();
        out.close();
    }
    for child in &shape.extra.children {
        out.raw(tag, child);
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

    fn primitive_round_trip(text: &str, oval: bool) -> (ShapeElement, String) {
        let doc = roxmltree::Document::parse(text).unwrap();
        let mut sink = Sink::default();
        let shape = {
            let mut ctx = Ctx::new(text, "t.xml", &mut sink);
            crate::shape::read_primitive(&mut ctx, doc.root_element(), oval)
        };
        let mut out = XmlOut::new();
        write_shape(&mut out, &shape);
        (shape, out.finish().0)
    }

    #[test]
    fn a_rectangle_primitive_keeps_its_parameters() {
        let source = r##"<DOMRectangleObject objectWidth="80" objectHeight="40" topLeftRadius="6" lockFlag="false" foo="1"><matrix><Matrix tx="10" ty="20"/></matrix>
        <fill><FillStyle index="1"><SolidColor color="#FF0000"/></FillStyle></fill>
        <stroke><StrokeStyle index="1"><SolidStroke weight="2"><fill><SolidColor color="#000000"/></fill></SolidStroke></StrokeStyle></stroke></DOMRectangleObject>"##;
        let (shape, written) = primitive_round_trip(source, false);
        assert!(written.starts_with("<DOMRectangleObject"), "{written}");
        let (again, _) = primitive_round_trip(&written, false);
        assert_eq!(shape, again, "{written}");
        let Some(p) = &shape.primitive else { panic!("no primitive") };
        let PrimitiveKind::Rectangle(r) = &p.kind else { panic!("not a rectangle") };
        assert_eq!(r.radii, [6.0, 0.0, 0.0, 0.0]);
        assert!(!r.lock);
        assert!(!shape.shape.is_empty());
        assert!(shape.is_drawing_object);
    }

    #[test]
    fn an_oval_primitive_keeps_its_angles() {
        let source = r##"<DOMOvalObject objectWidth="50" objectHeight="50" startAngle="30" endAngle="200" innerRadius="40" closePath="false"><fill><FillStyle index="1"><SolidColor color="#00FF00"/></FillStyle></fill></DOMOvalObject>"##;
        let (shape, written) = primitive_round_trip(source, true);
        assert!(written.starts_with("<DOMOvalObject"), "{written}");
        let (again, _) = primitive_round_trip(&written, true);
        assert_eq!(shape, again, "{written}");
        assert!(written.contains("startAngle=\"30\"") && written.contains("closePath=\"false\""));
    }
}
