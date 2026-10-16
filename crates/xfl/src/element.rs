//! The things inside a keyframe's `<elements>`: shapes, symbol and bitmap instances, text fields and groups.

use anim_core::{Affine, BlendMode, ColorTransform, Matrix3D, Point, Rgba8};
use anim_display::Filter;
use anim_scene::{
    BitmapInstance, ColorEffect, Element, Extra, Group, InstanceFilter, LoopMode, RawXml, SymbolInstance, SymbolKind, TextAlign, TextElement, TextKind,
    TextOrientation, TextRun,
};
use roxmltree::Node;

use crate::shape::read_shape;
use crate::xml::{Attrs, Ctx, child, element_children, read_matrix, read_point, text_of};

/// Most elements read from one `<elements>` or `<members>` list.
pub const MAX_ELEMENTS_PER_LIST: usize = 200_000;
/// Deepest `<DOMGroup>` nesting that is parsed; deeper groups are kept as raw XML.
pub const MAX_GROUP_DEPTH: usize = anim_scene::MAX_GROUP_DEPTH;

/// Reads the children of an `<elements>` or `<members>` node.
pub fn read_elements(ctx: &mut Ctx<'_>, list: Node<'_, '_>, depth: usize) -> Vec<Element> {
    let mut out = Vec::new();
    for c in element_children(list) {
        if out.len() >= MAX_ELEMENTS_PER_LIST {
            ctx.warn(format!("{}: too many elements, the rest are dropped", list.tag_name().name()));
            break;
        }
        out.push(read_element(ctx, c, depth));
    }
    out
}

fn read_element(ctx: &mut Ctx<'_>, node: Node<'_, '_>, depth: usize) -> Element {
    match node.tag_name().name() {
        "DOMShape" => Element::Shape(read_shape(ctx, node)),
        "DOMSymbolInstance" => Element::SymbolInstance(Box::new(read_symbol_instance(ctx, node))),
        "DOMBitmapInstance" => Element::Bitmap(read_bitmap_instance(ctx, node)),
        "DOMStaticText" => Element::Text(Box::new(read_text(ctx, node, TextKind::Static))),
        "DOMDynamicText" => Element::Text(Box::new(read_text(ctx, node, TextKind::Dynamic))),
        "DOMInputText" => Element::Text(Box::new(read_text(ctx, node, TextKind::Input))),
        "DOMGroup" if depth < MAX_GROUP_DEPTH => Element::Group(read_group(ctx, node, depth)),
        other => {
            let slot = ctx.sink.unknowns.children.entry(format!("elements/{other}")).or_insert(0);
            *slot = slot.saturating_add(1);
            Element::Unknown(RawXml { tag: other.to_owned(), xml: ctx.raw(node) })
        }
    }
}

fn read_group(ctx: &mut Ctx<'_>, node: Node<'_, '_>, depth: usize) -> Group {
    let mut group = Group::default();
    let attrs = Attrs::new(node);
    for c in element_children(node) {
        match c.tag_name().name() {
            "members" => group.members = read_elements(ctx, c, depth.saturating_add(1)),
            "matrix" => group.matrix = read_matrix(ctx, c),
            "transformationPoint" => group.transformation_point = Some(read_point(ctx, c)),
            _ => ctx.keep_child(&mut group.extra, "DOMGroup", c),
        }
    }
    attrs.rest(ctx, &mut group.extra);
    make_members_relative(&mut group);
    group
}

/// Animate stores a member's own matrix as its placement in the parent timeline, not relative to the group (the
/// shapes of a drawing and the shapes of a group around part of it share one edge space and one matrix). The model
/// composes the group's matrix over its members, so members with a matrix of their own are re-expressed relative to
/// the group; [`crate::write`] undoes this. Members without a matrix are placed by the group's matrix alone.
fn make_members_relative(group: &mut Group) {
    if group.matrix == Affine::IDENTITY || group.matrix.determinant().abs() < 1e-12 || !group.matrix.is_finite() {
        return;
    }
    let inverse = group.matrix.inverse();
    for member in &mut group.members {
        if let Some(own) = member.matrix()
            && own != Affine::IDENTITY
        {
            member.set_matrix(inverse * own);
        }
    }
}

fn read_bitmap_instance(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> BitmapInstance {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("libraryItemName").unwrap_or_default();
    let mut instance = BitmapInstance::new(name, Affine::IDENTITY);
    for c in element_children(node) {
        if c.tag_name().name() == "matrix" {
            instance.matrix = read_matrix(ctx, c);
        } else {
            ctx.keep_child(&mut instance.extra, "DOMBitmapInstance", c);
        }
    }
    attrs.rest(ctx, &mut instance.extra);
    instance
}

fn read_symbol_instance(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> SymbolInstance {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("libraryItemName").unwrap_or_default();
    let mut instance = SymbolInstance::new(name, Affine::IDENTITY);
    instance.symbol_type = attrs.take_with(ctx, "symbolType", SymbolKind::from_xfl_name);
    instance.loop_mode = attrs.take_with(ctx, "loop", LoopMode::from_xfl_name);
    instance.first_frame = attrs.u32(ctx, "firstFrame").unwrap_or(0);
    instance.last_frame = attrs.u32(ctx, "lastFrame");
    instance.name = attrs.string("name");
    instance.blend_mode = attrs
        .take_with(ctx, "blendMode", |s| {
            let mode = BlendMode::from_xfl_name(s);
            (mode != BlendMode::Normal || s == "normal").then_some(mode)
        })
        .unwrap_or(BlendMode::Normal);
    instance.visible = attrs.bool(ctx, "isVisible").unwrap_or(true);
    instance.cache_as_bitmap = attrs.bool(ctx, "cacheAsBitmap").unwrap_or(false);
    let center_x = attrs.f64(ctx, "centerPoint3DX");
    let center_y = attrs.f64(ctx, "centerPoint3DY");
    if center_x.is_some() || center_y.is_some() {
        instance.center_point_3d = Some(Point::new(center_x.unwrap_or(0.0), center_y.unwrap_or(0.0)));
    }
    for c in element_children(node) {
        match c.tag_name().name() {
            "matrix" => instance.matrix = read_matrix(ctx, c),
            "matrix3D" => instance.matrix_3d = read_matrix_3d(ctx, c),
            "transformationPoint" => instance.transformation_point = read_point(ctx, c),
            "color" => {
                if let Some(effect) = read_color(ctx, c) {
                    instance.set_color_effect(effect);
                }
            }
            "filters" => instance.filters = read_filters(ctx, c),
            _ => ctx.keep_child(&mut instance.extra, "DOMSymbolInstance", c),
        }
    }
    attrs.rest(ctx, &mut instance.extra);
    instance
}

const NAMES: [[&str; 4]; 4] = [
    ["m11", "m12", "m13", "m14"],
    ["m21", "m22", "m23", "m24"],
    ["m31", "m32", "m33", "m34"],
    ["m41", "m42", "m43", "m44"],
];

/// Reads `<matrix3D><Matrix3D m11 .. m44/></matrix3D>`; missing entries are the identity's. A matrix with a
/// non-finite entry is dropped.
fn read_matrix_3d(ctx: &mut Ctx<'_>, wrapper: Node<'_, '_>) -> Option<Matrix3D> {
    let node = child(wrapper, "Matrix3D")?;
    let mut attrs = Attrs::new(node);
    let mut matrix = Matrix3D::IDENTITY;
    for (values, names) in matrix.m.iter_mut().zip(NAMES) {
        for (value, name) in values.iter_mut().zip(names) {
            if let Some(v) = attrs.f64(ctx, name) {
                *value = v;
            }
        }
    }
    attrs.rest_lost(ctx);
    matrix.is_finite().then_some(matrix)
}

/// Reads `<color><Color .../></color>` into the style Animate would show in its Color property.
fn read_color(ctx: &mut Ctx<'_>, wrapper: Node<'_, '_>) -> Option<ColorEffect> {
    let node = child(wrapper, "Color")?;
    let mut attrs = Attrs::new(node);
    let tint_color = attrs.color(ctx, "tintColor");
    let tint_multiplier = attrs.f32(ctx, "tintMultiplier");
    let brightness = attrs.f32(ctx, "brightness");
    let mul = [attrs.f32(ctx, "redMultiplier"), attrs.f32(ctx, "greenMultiplier"), attrs.f32(ctx, "blueMultiplier"), attrs.f32(ctx, "alphaMultiplier")];
    let add = [attrs.f32(ctx, "redOffset"), attrs.f32(ctx, "greenOffset"), attrs.f32(ctx, "blueOffset"), attrs.f32(ctx, "alphaOffset")];
    attrs.rest_lost(ctx);
    if tint_color.is_some() || tint_multiplier.is_some() {
        return Some(ColorEffect::Tint { color: tint_color.unwrap_or(Rgba8::BLACK), amount: tint_multiplier.unwrap_or(0.0).clamp(0.0, 1.0) });
    }
    if let Some(b) = brightness {
        return Some(ColorEffect::Brightness(b.clamp(-1.0, 1.0)));
    }
    let offsets_used = add.iter().any(Option::is_some);
    let only_alpha = !offsets_used && mul.iter().take(3).all(Option::is_none);
    if only_alpha {
        return mul.get(3).copied().flatten().map(|a| ColorEffect::Alpha(a.clamp(0.0, 1.0)));
    }
    let m = |i: usize| mul.get(i).copied().flatten().unwrap_or(1.0);
    let a = |i: usize| add.get(i).copied().flatten().unwrap_or(0.0) / 255.0;
    Some(ColorEffect::Advanced(ColorTransform { mul: [m(0), m(1), m(2), m(3)], add: [a(0), a(1), a(2), a(3)] }))
}

fn read_filters(ctx: &mut Ctx<'_>, list: Node<'_, '_>) -> Vec<InstanceFilter> {
    let mut out = Vec::new();
    for c in element_children(list) {
        if out.len() >= 64 {
            ctx.warn("filters: too many filters, the rest are dropped");
            break;
        }
        out.push(read_filter(ctx, c));
    }
    out
}

fn read_filter(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> InstanceFilter {
    let tag = node.tag_name().name();
    let mut attrs = Attrs::new(node);
    let filter = match tag {
        "BlurFilter" => {
            let x = attrs.f64(ctx, "blurX").unwrap_or(5.0);
            let y = attrs.f64(ctx, "blurY").unwrap_or(5.0);
            let quality = attrs.u32(ctx, "quality").unwrap_or(1);
            Some(Filter::Blur { x, y, quality: u8::try_from(quality.min(3)).unwrap_or(1) })
        }
        "DropShadowFilter" => {
            let blur_x = attrs.f64(ctx, "blurX").unwrap_or(5.0);
            let blur_y = attrs.f64(ctx, "blurY").unwrap_or(5.0);
            let angle_deg = attrs.f64(ctx, "angle").unwrap_or(45.0);
            let distance = attrs.f64(ctx, "distance").unwrap_or(5.0);
            let strength = attrs.f64(ctx, "strength").unwrap_or(1.0);
            let color = attrs.color(ctx, "color").unwrap_or(Rgba8::BLACK);
            let alpha = attrs.f32(ctx, "alpha").unwrap_or(1.0);
            let inner = attrs.bool(ctx, "inner").unwrap_or(false);
            let knockout = attrs.bool(ctx, "knockout").unwrap_or(false);
            let hide_object = attrs.bool(ctx, "hideObject").unwrap_or(false);
            Some(Filter::DropShadow { color: color.with_alpha_f32(alpha), angle_deg, distance, blur_x, blur_y, strength, inner, knockout, hide_object })
        }
        "GlowFilter" => {
            let blur_x = attrs.f64(ctx, "blurX").unwrap_or(5.0);
            let blur_y = attrs.f64(ctx, "blurY").unwrap_or(5.0);
            let strength = attrs.f64(ctx, "strength").unwrap_or(1.0);
            let color = attrs.color(ctx, "color").unwrap_or(Rgba8::opaque(255, 0, 0));
            let alpha = attrs.f32(ctx, "alpha").unwrap_or(1.0);
            let inner = attrs.bool(ctx, "inner").unwrap_or(false);
            let knockout = attrs.bool(ctx, "knockout").unwrap_or(false);
            Some(Filter::Glow { color: color.with_alpha_f32(alpha), blur_x, blur_y, strength, inner, knockout })
        }
        _ => None,
    };
    if let Some(filter) = filter {
        let mut extra = Extra::new();
        // `quality` of shadows and glows has no slot in the display model; it stays here.
        if matches!(filter, Filter::DropShadow { .. } | Filter::Glow { .. })
            && let Some(q) = attrs.string("quality")
        {
            extra.attrs.push(("quality".to_owned(), q));
        }
        attrs.rest(ctx, &mut extra);
        for c in element_children(node) {
            ctx.keep_child(&mut extra, tag, c);
        }
        InstanceFilter::Mapped { filter, extra }
    } else {
        let slot = ctx.sink.unknowns.children.entry(format!("filters/{tag}")).or_insert(0);
        *slot = slot.saturating_add(1);
        InstanceFilter::Raw(RawXml { tag: tag.to_owned(), xml: ctx.raw(node) })
    }
}

fn read_text(ctx: &mut Ctx<'_>, node: Node<'_, '_>, kind: TextKind) -> TextElement {
    let tag = kind.element_name();
    let mut attrs = Attrs::new(node);
    let width = attrs.f64(ctx, "width").unwrap_or(0.0);
    let height = attrs.f64(ctx, "height").unwrap_or(0.0);
    let mut text = TextElement::new(kind, Affine::IDENTITY, width, height, Vec::new());
    text.auto_expand = attrs.bool(ctx, "autoExpand").unwrap_or(false);
    text.orientation = attrs.take_with(ctx, "orientation", TextOrientation::from_xfl_name).unwrap_or_default();
    text.instance_name = attrs.string("instanceName");
    for c in element_children(node) {
        match c.tag_name().name() {
            "matrix" => text.matrix = read_matrix(ctx, c),
            "textRuns" => {
                for run in element_children(c) {
                    if run.tag_name().name() == "DOMTextRun" {
                        text.text_runs.push(read_text_run(ctx, run));
                    } else {
                        ctx.keep_child(&mut text.extra, "textRuns", run);
                    }
                }
            }
            _ => ctx.keep_child(&mut text.extra, tag, c),
        }
    }
    attrs.rest(ctx, &mut text.extra);
    text
}

fn read_text_run(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> TextRun {
    let mut run = TextRun::new(String::new(), String::new(), 12.0);
    let mut seen_attrs = false;
    let attrs = Attrs::new(node);
    for c in element_children(node) {
        match c.tag_name().name() {
            "characters" => run.characters = text_of(c),
            "textAttrs" => {
                for a in element_children(c) {
                    if a.tag_name().name() == "DOMTextAttrs" && !seen_attrs {
                        seen_attrs = true;
                        read_text_attrs(ctx, a, &mut run);
                    } else {
                        ctx.keep_child(&mut run.extra, "textAttrs", a);
                    }
                }
            }
            _ => ctx.keep_child(&mut run.extra, "DOMTextRun", c),
        }
    }
    attrs.rest(ctx, &mut run.extra);
    run
}

fn read_text_attrs(ctx: &mut Ctx<'_>, node: Node<'_, '_>, run: &mut TextRun) {
    let mut attrs = Attrs::new(node);
    run.face = attrs.string("face").unwrap_or_default();
    run.size = attrs.f32(ctx, "size").unwrap_or(12.0);
    run.bold = attrs.bool(ctx, "bold").unwrap_or(false);
    run.italic = attrs.bool(ctx, "italic").unwrap_or(false);
    run.fill_color = attrs.color(ctx, "fillColor").unwrap_or(Rgba8::BLACK).with_alpha_f32(1.0);
    run.alpha = attrs.f32(ctx, "alpha").unwrap_or(1.0);
    run.letter_spacing = attrs.f32(ctx, "letterSpacing").unwrap_or(0.0);
    run.alignment = attrs.take_with(ctx, "alignment", TextAlign::from_xfl_name).unwrap_or_default();
    attrs.rest(ctx, &mut run.extra);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Sink;

    fn read(text: &str) -> (Vec<Element>, Sink) {
        let doc = roxmltree::Document::parse(text).unwrap();
        let mut sink = Sink::default();
        let list = {
            let mut ctx = Ctx::new(text, "t.xml", &mut sink);
            read_elements(&mut ctx, doc.root_element(), 0)
        };
        (list, sink)
    }

    #[test]
    fn symbol_instance_with_colour_filters_and_blend() {
        let (els, sink) = read(
            r#"<elements><DOMSymbolInstance libraryItemName="a/b" symbolType="graphic" loop="play once" firstFrame="3" blendMode="multiply" name="x" centerPoint3DX="1">
            <matrix><Matrix a="2" d="2" tx="10" ty="20"/></matrix><transformationPoint><Point y="0.3"/></transformationPoint>
            <color><Color alphaMultiplier="0.5"/></color>
            <filters><BlurFilter blurX="4" blurY="6" quality="2"/><DropShadowFilter distance="3" quality="1" alpha="0.5"/><BevelFilter blurX="2"/></filters></DOMSymbolInstance></elements>"#,
        );
        let Some(Element::SymbolInstance(i)) = els.first() else {
            panic!("expected an instance");
        };
        assert_eq!(i.library_item_name, "a/b");
        assert_eq!(i.symbol_type, Some(SymbolKind::Graphic));
        assert_eq!(i.loop_mode, Some(LoopMode::PlayOnce));
        assert_eq!(i.first_frame, 3);
        assert_eq!(i.blend_mode, BlendMode::Multiply);
        assert_eq!(i.name.as_deref(), Some("x"));
        assert_eq!(i.color_effect, ColorEffect::Alpha(0.5));
        assert!((i.color.mul[3] - 0.5).abs() < 1e-6);
        assert!((i.transformation_point.y - 0.3).abs() < 1e-9);
        assert_eq!(i.matrix, Affine::new([2.0, 0.0, 0.0, 2.0, 10.0, 20.0]));
        assert_eq!(i.filters.len(), 3);
        assert!(matches!(i.filters.first(), Some(InstanceFilter::Mapped { filter: Filter::Blur { quality: 2, .. }, .. })));
        assert!(matches!(i.filters.get(2), Some(InstanceFilter::Raw(r)) if r.tag == "BevelFilter"));
        assert_eq!(i.center_point_3d, Some(Point::new(1.0, 0.0)));
        assert!(sink.unknowns.attrs.is_empty());
    }

    #[test]
    fn matrix_3d_and_centre_point_are_modelled() {
        let (els, sink) = read(
            r#"<elements><DOMSymbolInstance libraryItemName="a" centerPoint3DX="5" centerPoint3DY="6"><matrix3D><Matrix3D m11="0.5" m33="2" m43="40" m41="10"/></matrix3D></DOMSymbolInstance></elements>"#,
        );
        let Some(Element::SymbolInstance(i)) = els.first() else { panic!("instance") };
        let m = i.matrix_3d.expect("matrix3D");
        assert_eq!((m.m[0][0], m.m[2][2], m.m[3][2], m.m[3][0], m.m[1][1]), (0.5, 2.0, 40.0, 10.0, 1.0));
        assert_eq!(i.center_point_3d, Some(Point::new(5.0, 6.0)));
        assert!(sink.unknowns.attrs.is_empty());
    }

    #[test]
    fn tint_brightness_and_advanced_colour_effects() {
        let wrap = |inner: &str| format!(r#"<elements><DOMSymbolInstance libraryItemName="s"><color><Color {inner}/></color></DOMSymbolInstance></elements>"#);
        let effect = |inner: &str| {
            let (els, _) = read(&wrap(inner));
            els.first().and_then(Element::as_symbol_instance).map(|i| i.color_effect).unwrap()
        };
        assert!(matches!(effect(r##"tintColor="#FF0000" tintMultiplier="0.5""##), ColorEffect::Tint { amount, .. } if (amount - 0.5).abs() < 1e-6));
        assert_eq!(effect(r#"brightness="-0.5""#), ColorEffect::Brightness(-0.5));
        let ColorEffect::Advanced(t) = effect(r#"redMultiplier="0.5" blueOffset="51""#) else {
            panic!("expected advanced");
        };
        assert!((t.mul[0] - 0.5).abs() < 1e-6 && (t.add[2] - 0.2).abs() < 1e-6 && (t.mul[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn groups_nest_and_deep_nesting_becomes_raw_xml() {
        let mut text = String::new();
        let depth = MAX_GROUP_DEPTH + 5;
        for _ in 0..depth {
            text.push_str("<DOMGroup><members>");
        }
        for _ in 0..depth {
            text.push_str("</members></DOMGroup>");
        }
        let (els, _) = read(&format!("<elements>{text}</elements>"));
        let mut kinds = 0;
        let mut unknown = 0;
        for e in &els {
            e.walk(&mut |x| {
                kinds += 1;
                if matches!(x, Element::Unknown(_)) {
                    unknown += 1;
                }
            });
        }
        assert!(kinds > MAX_GROUP_DEPTH);
        assert_eq!(unknown, 1);
    }

    #[test]
    fn hebrew_static_text_survives() {
        let (els, _) = read(
            r##"<elements><DOMStaticText width="100" height="50" autoExpand="true" isSelectable="false"><matrix><Matrix tx="5"/></matrix>
            <textRuns><DOMTextRun><characters>שלום world</characters><textAttrs><DOMTextAttrs aliasText="false" size="24" bold="true" face="Arial" fillColor="#0066CC" alignment="right" letterSpacing="1.5"/></textAttrs></DOMTextRun></textRuns></DOMStaticText></elements>"##,
        );
        let Some(Element::Text(t)) = els.first() else {
            panic!("expected text");
        };
        assert_eq!(t.plain_text(), "שלום world");
        assert!(t.auto_expand);
        let run = t.text_runs.first().unwrap();
        assert!(run.bold);
        assert_eq!(run.alignment, TextAlign::Right);
        assert_eq!(run.fill_color, Rgba8::opaque(0, 0x66, 0xCC));
        assert_eq!(run.extra.attr("aliasText"), Some("false"));
        assert_eq!(t.extra.attr("isSelectable"), Some("false"));
    }

    #[test]
    fn unknown_elements_are_kept_verbatim() {
        let (els, sink) = read(r#"<elements><DOMVideoInstance libraryItemName="v"/></elements>"#);
        assert!(matches!(els.first(), Some(Element::Unknown(r)) if r.tag == "DOMVideoInstance" && r.xml == r#"<DOMVideoInstance libraryItemName="v"/>"#));
        assert_eq!(sink.unknowns.children.get("elements/DOMVideoInstance"), Some(&1));
    }
}
