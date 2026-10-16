use anicraft_tools::Selection;
use anim_core::Rgba8;
use anim_scene::edit;
use anim_scene::timeline::{LabelType, TweenType};
use anim_scene::{ColorEffect, Element, SymbolKind, TextAlign, TextDirection, TextElement, TextKind, TextRun};
use anim_shape::{EdgeShape, FillStyle, StrokeKind, StrokeStyle};
use serde_json::json;

use super::*;

const RED: Rgba8 = Rgba8 { r: 255, g: 0, b: 0, a: 255 };
const SCENE: TimelineId = TimelineId::Scene(0);

fn doc_with_instance() -> AniDoc {
    let mut doc = AniDoc::default();
    doc.editor.document.add_symbol("box", SymbolKind::Graphic).unwrap();
    edit::add_symbol_instance(&mut doc.editor.document, &SCENE, 0, 0, "box", Affine::translate((10.0, 20.0))).unwrap();
    doc.editor.state.selection = Selection::one(ElementRef::top_level(0, 0, 0));
    doc
}

fn doc_with(element: Element) -> AniDoc {
    let mut doc = AniDoc::default();
    edit::add_element(&mut doc.editor.document, &SCENE, 0, 0, element).unwrap();
    doc.editor.state.selection = Selection::one(ElementRef::top_level(0, 0, 0));
    doc
}

fn shape_doc() -> AniDoc {
    let stroke = StrokeStyle::plain(3.0, RED);
    doc_with(Element::Shape(anim_scene::ShapeElement::new(EdgeShape::new(vec![FillStyle::Solid(Rgba8 { r: 0, g: 0, b: 255, a: 255 })], vec![stroke]))))
}

fn text_doc(characters: &str) -> AniDoc {
    let run = TextRun::new(characters, "Arial", 24.0);
    doc_with(Element::Text(Box::new(TextElement::new(TextKind::Static, Affine::IDENTITY, 200.0, 50.0, vec![run]))))
}

fn selected(doc: &AniDoc) -> &Element {
    element(&doc.editor.document, &SCENE, &ElementRef::top_level(0, 0, 0)).unwrap()
}

fn set(doc: &mut AniDoc, id: &str, json: serde_json::Value) {
    let value = from_json(doc, id, &json).unwrap_or_else(|e| panic!("{id}: {e}"));
    apply(doc, id, &value).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn rows(doc: &AniDoc) -> Vec<String> {
    describe(doc).into_iter().map(|p| p.id).collect()
}

#[test]
fn nothing_selected_edits_the_document() {
    let mut doc = AniDoc::default();
    let props = describe(&doc);
    assert!(props.iter().any(|p| p.id == "doc.width"));
    apply(&mut doc, "doc.width", &PropValue::number(800.0)).unwrap();
    assert!((doc.editor.document.width - 800.0).abs() < f64::EPSILON);
    assert!(apply(&mut doc, "doc.fps", &PropValue::number(-1.0)).is_err());
    assert!(apply(&mut doc, "doc.fps", &PropValue::number(f64::NAN)).is_err());
}

#[test]
fn nothing_selected_also_shows_the_layer_and_the_keyframe() {
    let doc = AniDoc::default();
    let ids = rows(&doc);
    for id in ["doc.background", "layer.name", "layer.visible", "layer.locked", "layer.color", "frame.label"] {
        assert!(ids.iter().any(|i| i == id), "{id} in {ids:?}");
    }
    // Without a tween there are no tween rows beyond the type.
    assert!(ids.iter().any(|i| i == "tween.type"));
    assert!(!ids.iter().any(|i| i == "tween.rotate"));
}

#[test]
fn instance_properties_round_trip() {
    let mut doc = doc_with_instance();
    let props = describe(&doc);
    let x = props.iter().find(|p| p.id == "x").unwrap();
    assert!(matches!(x.value, PropValue::Number { value, .. } if (value - 10.0).abs() < 1e-9));
    apply(&mut doc, "x", &PropValue::number(50.0)).unwrap();
    apply(&mut doc, "rotation", &PropValue::number(90.0)).unwrap();
    set(&mut doc, "name", json!("hero"));
    set(&mut doc, "effect", json!("Alpha"));
    set(&mut doc, "alpha", json!(40));
    let Some(Element::SymbolInstance(i)) = Some(selected(&doc)) else { panic!("no instance") };
    assert!((i.matrix.translation().x - 50.0).abs() < 1e-9);
    assert_eq!(i.name.as_deref(), Some("hero"));
    assert!(matches!(i.color_effect, ColorEffect::Alpha(a) if (a - 0.4).abs() < 1e-6));
    let (rotation, _, _) = linear(i.matrix);
    assert!((rotation - 90.0).abs() < 1e-6);
}

#[test]
fn json_values_follow_the_property_kind() {
    let doc = doc_with_instance();
    assert!(matches!(from_json(&doc, "x", &json!(3.5)), Ok(PropValue::Number { value, .. }) if (value - 3.5).abs() < 1e-9));
    assert!(matches!(from_json(&doc, "blend", &json!("multiply")), Ok(PropValue::Choice { selected: 2, .. })));
    assert!(from_json(&doc, "x", &json!("wide")).is_err());
    assert!(from_json(&doc, "nope", &json!(1)).is_err());
    // Document, layer and keyframe properties are reachable whatever is selected.
    assert!(from_json(&doc, "doc.width", &json!(300)).is_ok());
    assert!(from_json(&doc, "layer.name", &json!("Ink")).is_ok());
}

#[test]
fn shape_fill_stroke_alpha_width_style_cap_join_and_scale() {
    let mut doc = shape_doc();
    for id in [
        "fill",
        "fill_alpha",
        "stroke",
        "stroke_alpha",
        "stroke_width",
        "stroke_style",
        "stroke_cap",
        "stroke_join",
        "stroke_scale",
        "stroke_hinting",
        "drawing_object",
    ] {
        assert!(rows(&doc).iter().any(|i| i == id), "{id}");
    }
    set(&mut doc, "fill", json!("#00ff00"));
    set(&mut doc, "fill_alpha", json!(50));
    set(&mut doc, "stroke", json!("#112233"));
    set(&mut doc, "stroke_alpha", json!(25));
    set(&mut doc, "stroke_width", json!(7.5));
    set(&mut doc, "stroke_style", json!("Dashed"));
    set(&mut doc, "stroke_cap", json!("square"));
    set(&mut doc, "stroke_join", json!("bevel"));
    set(&mut doc, "stroke_scale", json!("none"));
    set(&mut doc, "stroke_hinting", json!(true));
    set(&mut doc, "drawing_object", json!(true));
    let Element::Shape(s) = selected(&doc) else { panic!("no shape") };
    assert!(matches!(s.shape.fills.first(), Some(FillStyle::Solid(c)) if c.g == 255 && (127..=128).contains(&c.a)));
    let Some(StrokeStyle::Other(stroke)) = s.shape.strokes.first() else { panic!("expected a dashed stroke") };
    assert_eq!(stroke.kind, StrokeKind::Dashed);
    assert!(stroke.attrs.iter().any(|(k, _)| k == "dash1"));
    assert!((stroke.base.weight - 7.5).abs() < 1e-9);
    assert!(matches!(stroke.base.fill, FillStyle::Solid(c) if c.r == 0x11 && (63..=65).contains(&c.a)));
    assert_eq!(stroke.base.caps, anim_display::Cap::Square);
    assert_eq!(stroke.base.joints, anim_display::Join::Bevel);
    assert_eq!(stroke.base.scale_mode, anim_display::StrokeScale::None);
    assert!(stroke.base.pixel_hinting && s.is_drawing_object);
    // Back to solid keeps the weight; a miter join exposes its limit.
    set(&mut doc, "stroke_style", json!("Solid"));
    set(&mut doc, "stroke_join", json!("miter"));
    set(&mut doc, "stroke_miter", json!(8));
    let Element::Shape(s) = selected(&doc) else { panic!("no shape") };
    assert!(matches!(s.shape.strokes.first(), Some(StrokeStyle::Solid(x)) if (x.weight - 7.5).abs() < 1e-9 && (x.miter_limit - 8.0).abs() < 1e-9));
    assert!(apply(&mut doc, "stroke_width", &PropValue::number(-1.0)).is_err());
    assert!(apply(&mut doc, "stroke_cap", &PropValue::choice(9, &["x"])).is_err());
}

#[test]
fn gradient_stops_are_editable_and_a_colour_makes_the_fill_solid() {
    let gradient = anim_shape::styles::GradientFill {
        stops: vec![anim_display::GradientStop { offset: 0.0, color: RED }, anim_display::GradientStop { offset: 1.0, color: Rgba8::BLACK }],
        ..anim_shape::styles::GradientFill::default()
    };
    let mut doc = doc_with(Element::Shape(anim_scene::ShapeElement::new(EdgeShape::new(vec![FillStyle::LinearGradient(gradient)], vec![]))));
    let ids = rows(&doc);
    assert!(ids.iter().any(|i| i == "fill_type") && ids.iter().any(|i| i == "fill_stop1"));
    set(&mut doc, "fill_stop1", json!("#0000ff"));
    let Element::Shape(s) = selected(&doc) else { panic!("no shape") };
    assert!(matches!(s.shape.fills.first(), Some(FillStyle::LinearGradient(g)) if g.stops.get(1).is_some_and(|x| x.color.b == 255)));
    set(&mut doc, "fill", json!("#ff00ff"));
    let Element::Shape(s) = selected(&doc) else { panic!("no shape") };
    assert!(matches!(s.shape.fills.first(), Some(FillStyle::Solid(c)) if c.r == 255 && c.b == 255));
}

#[test]
fn instance_type_looping_advanced_colour_and_visibility() {
    let mut doc = doc_with_instance();
    for id in ["symbol", "name", "kind", "visible", "cache_as_bitmap", "loop", "first_frame", "effect", "blend", "filter.add"] {
        assert!(rows(&doc).iter().any(|i| i == id), "{id}");
    }
    set(&mut doc, "kind", json!("Movie Clip"));
    assert!(!rows(&doc).iter().any(|i| i == "loop"), "only graphics loop");
    set(&mut doc, "kind", json!("Graphic"));
    set(&mut doc, "loop", json!("Play Once"));
    set(&mut doc, "first_frame", json!(3));
    set(&mut doc, "last_frame", json!(9));
    set(&mut doc, "visible", json!(false));
    set(&mut doc, "cache_as_bitmap", json!(true));
    set(&mut doc, "effect", json!("Advanced"));
    set(&mut doc, "adv_g", json!(50));
    set(&mut doc, "adv_b_off", json!(51));
    let Element::SymbolInstance(i) = selected(&doc) else { panic!("no instance") };
    assert_eq!(i.symbol_type, Some(SymbolKind::Graphic));
    assert_eq!((i.first_frame, i.last_frame), (2, Some(8)));
    assert!(!i.visible && i.cache_as_bitmap);
    let ColorEffect::Advanced(t) = i.color_effect else { panic!("expected the advanced effect") };
    assert!((t.mul[1] - 0.5).abs() < 1e-6 && (t.add[2] - 0.2).abs() < 1e-6);
    assert_eq!(i.color, t);
    set(&mut doc, "effect", json!("Tint"));
    set(&mut doc, "tint", json!("#00ff00"));
    set(&mut doc, "tint_amount", json!(30));
    set(&mut doc, "effect", json!("Brightness"));
    set(&mut doc, "brightness", json!(-20));
    let Element::SymbolInstance(i) = selected(&doc) else { panic!("no instance") };
    assert!(matches!(i.color_effect, ColorEffect::Brightness(b) if (b + 0.2).abs() < 1e-6));
    assert!(apply(&mut doc, "adv_r", &PropValue::number(10.0)).is_err(), "not the advanced style any more");
}

#[test]
fn filters_are_listed_with_parameters_and_editable() {
    let mut doc = doc_with_instance();
    set(&mut doc, "filter.add", json!("Blur"));
    set(&mut doc, "filter.add", json!("Drop Shadow"));
    set(&mut doc, "filter.add", json!("Glow"));
    let ids = rows(&doc);
    for id in [
        "filter.0.blur_x",
        "filter.0.quality",
        "filter.1.angle",
        "filter.1.distance",
        "filter.1.hide_object",
        "filter.2.inner",
        "filter.2.strength",
        "filter.2.remove",
    ] {
        assert!(ids.iter().any(|i| i == id), "{id} in {ids:?}");
    }
    set(&mut doc, "filter.0.blur_x", json!(12));
    set(&mut doc, "filter.0.quality", json!("High"));
    set(&mut doc, "filter.1.angle", json!(90));
    set(&mut doc, "filter.1.color", json!("#336699"));
    set(&mut doc, "filter.2.strength", json!(250));
    set(&mut doc, "filter.2.inner", json!(true));
    let Element::SymbolInstance(i) = selected(&doc) else { panic!("no instance") };
    assert!(
        matches!(i.filters.first(), Some(anim_scene::InstanceFilter::Mapped { filter: anim_display::Filter::Blur { x, quality: 3, .. }, .. }) if (x - 12.0).abs() < 1e-9)
    );
    assert!(
        matches!(i.filters.get(1), Some(anim_scene::InstanceFilter::Mapped { filter: anim_display::Filter::DropShadow { angle_deg, color, .. }, .. }) if (angle_deg - 90.0).abs() < 1e-9 && color.b == 0x99)
    );
    assert!(
        matches!(i.filters.get(2), Some(anim_scene::InstanceFilter::Mapped { filter: anim_display::Filter::Glow { strength, inner: true, .. }, .. }) if (strength - 2.5).abs() < 1e-9)
    );
    assert!(apply(&mut doc, "filter.0.blur_x", &PropValue::number(999.0)).is_err());
    assert!(apply(&mut doc, "filter.9.blur_x", &PropValue::number(1.0)).is_err());
    assert!(apply(&mut doc, "filter.0.angle", &PropValue::number(1.0)).is_err(), "a blur has no angle");
    set(&mut doc, "filter.1.remove", json!(true));
    let Element::SymbolInstance(i) = selected(&doc) else { panic!("no instance") };
    assert_eq!(i.filters.len(), 2);
    assert!(matches!(i.filters.get(1), Some(anim_scene::InstanceFilter::Mapped { filter: anim_display::Filter::Glow { .. }, .. })));
}

#[test]
fn a_symbol_can_be_swapped_but_not_into_itself() {
    let mut doc = doc_with_instance();
    doc.editor.document.add_symbol("other", SymbolKind::MovieClip).unwrap();
    let names: Vec<String> = doc.editor.document.library.symbols().map(|s| s.name.clone()).collect();
    let other = names.iter().position(|n| n == "other").unwrap();
    apply(&mut doc, "symbol", &PropValue::choice(other, &["box", "other"])).unwrap();
    let Element::SymbolInstance(i) = selected(&doc) else { panic!("no instance") };
    assert_eq!(i.library_item_name, "other");
    assert!(apply(&mut doc, "symbol", &PropValue::choice(9, &["x"])).is_err());
    // Inside "box", placing "box" would make it contain itself.
    doc.editor.state.timeline = TimelineId::Symbol("box".to_owned());
    edit::add_symbol_instance(&mut doc.editor.document, &doc.editor.state.timeline.clone(), 0, 0, "other", Affine::IDENTITY).unwrap();
    doc.editor.state.selection = Selection::one(ElementRef::top_level(0, 0, 0));
    let box_index = names.iter().position(|n| n == "box").unwrap();
    assert!(apply(&mut doc, "symbol", &PropValue::choice(box_index, &["box", "other"])).is_err());
}

#[test]
fn text_character_and_paragraph_properties() {
    let mut doc = text_doc("שלום world");
    for id in [
        "text_type",
        "orientation",
        "auto_expand",
        "text",
        "font",
        "size",
        "text_color",
        "text_alpha",
        "bold",
        "italic",
        "letter_spacing",
        "align",
        "direction",
        "line_spacing",
        "indent",
        "margin_left",
        "margin_right",
    ] {
        assert!(rows(&doc).iter().any(|i| i == id), "{id}");
    }
    set(&mut doc, "direction", json!("Right to left"));
    set(&mut doc, "align", json!("Center"));
    set(&mut doc, "font", json!("Noto Sans Hebrew"));
    set(&mut doc, "size", json!(36));
    set(&mut doc, "text_color", json!("#336699"));
    set(&mut doc, "text_alpha", json!(80));
    set(&mut doc, "bold", json!(true));
    set(&mut doc, "letter_spacing", json!(1.5));
    set(&mut doc, "line_spacing", json!(6));
    set(&mut doc, "indent", json!(4));
    set(&mut doc, "margin_left", json!(8));
    set(&mut doc, "text_type", json!("Dynamic"));
    set(&mut doc, "name", json!("title_txt"));
    set(&mut doc, "orientation", json!("Vertical, right to left"));
    set(&mut doc, "auto_expand", json!(true));
    set(&mut doc, "text", json!("עברית"));
    let Element::Text(t) = selected(&doc) else { panic!("no text") };
    let run = t.text_runs.first().unwrap();
    assert_eq!(run.direction, TextDirection::RightToLeft);
    assert_eq!(run.alignment, TextAlign::Center);
    assert_eq!((run.face.as_str(), run.characters.as_str()), ("Noto Sans Hebrew", "עברית"));
    assert!((run.size - 36.0).abs() < 1e-6 && (run.alpha - 0.8).abs() < 1e-6 && (run.letter_spacing - 1.5).abs() < 1e-6);
    assert!(run.bold && run.fill_color.b == 0x99);
    assert_eq!(run.extra.attr("lineSpacing"), Some("6"));
    assert_eq!(run.extra.attr("indent"), Some("4"));
    assert_eq!(run.extra.attr("leftMargin"), Some("8"));
    assert_eq!(run.extra.attr("rightMargin"), None);
    assert_eq!((t.kind, t.instance_name.as_deref(), t.auto_expand), (TextKind::Dynamic, Some("title_txt"), true));
    assert_eq!(t.orientation, anim_scene::TextOrientation::VerticalRightToLeft);
    // Zero removes the attribute again, so untouched files stay byte-identical.
    set(&mut doc, "line_spacing", json!(0));
    let Element::Text(t) = selected(&doc) else { panic!("no text") };
    assert_eq!(t.text_runs.first().unwrap().extra.attr("lineSpacing"), None);
    assert!(apply(&mut doc, "size", &PropValue::number(0.0)).is_err());
    assert!(apply(&mut doc, "margin_left", &PropValue::number(-3.0)).is_err());
}

#[test]
fn bitmap_instances_swap_source_and_edit_the_library_item() {
    let mut doc = AniDoc::default();
    for name in ["a.png", "b.png"] {
        let item = anim_scene::BitmapItem::new(name, name);
        doc.editor.document.library.push(anim_scene::LibraryItem::Bitmap(item)).unwrap();
    }
    edit::add_element(&mut doc.editor.document, &SCENE, 0, 0, Element::Bitmap(anim_scene::BitmapInstance::new("a.png", Affine::IDENTITY))).unwrap();
    doc.editor.state.selection = Selection::one(ElementRef::top_level(0, 0, 0));
    for id in ["source", "bitmap.smoothing", "bitmap.compression"] {
        assert!(rows(&doc).iter().any(|i| i == id), "{id}");
    }
    set(&mut doc, "bitmap.smoothing", json!(false));
    set(&mut doc, "bitmap.compression", json!("Photo (JPEG)"));
    set(&mut doc, "bitmap.quality", json!(70));
    let item = doc.editor.document.library.find_bitmap("a.png").unwrap();
    assert!(!item.smoothing && item.compression == BitmapCompression::Photo && item.quality == Some(70));
    set(&mut doc, "source", json!("b.png"));
    let Element::Bitmap(b) = selected(&doc) else { panic!("no bitmap") };
    assert_eq!(b.library_item_name, "b.png");
}

#[test]
fn groups_move_and_report_their_members() {
    let inner = Element::Shape(anim_scene::ShapeElement::new(EdgeShape::new(vec![FillStyle::Solid(RED)], vec![])));
    let mut doc = doc_with(Element::Group(anim_scene::Group::new(vec![inner])));
    assert!(rows(&doc).iter().any(|i| i == "members"));
    apply(&mut doc, "x", &PropValue::number(40.0)).unwrap();
    let Element::Group(g) = selected(&doc) else { panic!("no group") };
    assert!((g.matrix.translation().x - 40.0).abs() < 1e-9);
}

#[test]
fn layer_properties() {
    let mut doc = AniDoc::default();
    set(&mut doc, "layer.name", json!("Ink"));
    set(&mut doc, "layer.visible", json!(false));
    set(&mut doc, "layer.locked", json!(true));
    set(&mut doc, "layer.outline", json!(true));
    set(&mut doc, "layer.color", json!("#00ff00"));
    let layer = doc.editor.document.scenes.first().and_then(|t| t.layers.first()).unwrap();
    assert_eq!(layer.name, "Ink");
    assert!(!layer.visible && layer.locked && layer.outline);
    assert_eq!(layer.color.g, 255);
    assert!(apply(&mut doc, "layer.name", &PropValue::Text(String::new())).is_err(), "a layer needs a name");
    assert!(apply(&mut doc, "layer.nonsense", &PropValue::Bool(true)).is_err());
}

#[test]
fn frame_label_and_tween_options() {
    let mut doc = doc_with_instance();
    doc.editor.state.selection = Selection::default();
    edit::insert_keyframe(&mut doc.editor.document, &SCENE, 0, 9).unwrap();
    set(&mut doc, "frame.label", json!("intro"));
    set(&mut doc, "frame.label_type", json!("Anchor"));
    let k = doc.editor.document.scenes[0].layers[0].keyframe_at(0).unwrap();
    assert_eq!((k.label.as_deref(), k.label_type), (Some("intro"), Some(LabelType::Anchor)));
    // A tween needs a type first; then its rows appear.
    assert!(apply(&mut doc, "tween.ease", &PropValue::number(10.0)).is_err());
    set(&mut doc, "tween.type", json!("Classic"));
    assert_eq!(doc.editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().tween_type, TweenType::Classic);
    for id in ["tween.preset", "tween.ease", "tween.rotate", "tween.scale", "tween.snap", "tween.orient", "tween.sync"] {
        assert!(rows(&doc).iter().any(|i| i == id), "{id}");
    }
    set(&mut doc, "tween.ease", json!(-40));
    set(&mut doc, "tween.rotate", json!("Clockwise"));
    set(&mut doc, "tween.rotate_times", json!(2));
    set(&mut doc, "tween.scale", json!(false));
    set(&mut doc, "tween.orient", json!(true));
    set(&mut doc, "tween.preset", json!("bounceOut"));
    let k = doc.editor.document.scenes[0].layers[0].keyframe_at(0).unwrap();
    assert_eq!(k.acceleration, 0, "a preset replaces the slider");
    assert!(matches!(k.ease_for(anim_scene::timeline::EaseTarget::All), Some(anim_scene::timeline::EaseCurve::Named(n)) if n == "bounceOut"));
    assert_eq!(k.motion_tween_rotate, anim_scene::timeline::TweenRotate::Clockwise);
    assert!(k.motion_tween_rotate_times.is_some_and(|t| (t - 2.0).abs() < 1e-9));
    assert!(!k.motion_tween_scale && k.motion_tween_orient_to_path);
    assert!(!rows(&doc).iter().any(|i| i == "tween.ease"), "the slider is hidden behind a preset");
    set(&mut doc, "tween.preset", json!("Classic (slider)"));
    set(&mut doc, "tween.ease", json!(60));
    assert_eq!(doc.editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().acceleration, 60);
    assert!(apply(&mut doc, "tween.ease", &PropValue::number(101.0)).is_err());
    set(&mut doc, "tween.type", json!("None"));
    assert_eq!(doc.editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().tween_type, TweenType::None);
    // A shape cannot be classic-tweened; the error says why.
    doc.editor.document.scenes[0].layers[0].frames[0].elements = vec![Element::Shape(anim_scene::ShapeElement::default())];
    assert!(apply(&mut doc, "tween.type", &PropValue::choice(1, &["None", "Classic", "Shape"])).is_err());
}

fn component_doc() -> AniDoc {
    let mut doc = doc_with_instance();
    if let Some(Element::SymbolInstance(i)) = element_mut(&mut doc.editor.document, &SCENE, &ElementRef::top_level(0, 0, 0)) {
        i.component =
            Some(anim_scene::ComponentInstance::from_params(&[("label", "Click", "String"), ("enabled", "true", "Boolean"), ("size", "3", "Number")]));
    }
    doc
}

#[test]
fn component_parameters_are_rows_and_are_written_back_to_the_parameter_xml() {
    let mut doc = component_doc();
    let ids = rows(&doc);
    for id in ["param.label", "param.enabled", "param.size"] {
        assert!(ids.iter().any(|r| r == id), "{ids:?}");
    }
    let by_id = |doc: &AniDoc, id: &str| describe(doc).into_iter().find(|p| p.id == id).map(|p| p.value);
    assert_eq!(by_id(&doc, "param.enabled"), Some(PropValue::Bool(true)));
    set(&mut doc, "param.label", json!("Go & stop"));
    set(&mut doc, "param.enabled", json!(false));
    set(&mut doc, "param.size", json!(12));
    let Element::SymbolInstance(i) = selected(&doc) else { panic!("not an instance") };
    let component = i.component.as_ref().unwrap();
    assert_eq!(component.param("label").as_deref(), Some("Go & stop"));
    assert_eq!(component.param("enabled").as_deref(), Some("false"));
    assert_eq!(component.param("size").as_deref(), Some("12"));
    assert!(component.parameters_xml.contains("Go &amp; stop"), "{}", component.parameters_xml);
    assert!(apply(&mut doc, "param.missing", &PropValue::Text("x".to_owned())).is_err());
}

fn primitive_doc(oval: bool) -> AniDoc {
    use anim_scene::{OvalPrimitive, Primitive, PrimitiveKind, RectPrimitive, ShapeElement};
    let kind = if oval {
        PrimitiveKind::Oval(OvalPrimitive { width: 80.0, height: 60.0, start_angle: 0.0, end_angle: 0.0, inner_radius: 0.0, close_path: true })
    } else {
        PrimitiveKind::Rectangle(RectPrimitive { width: 80.0, height: 60.0, radii: [0.0; 4], lock: false })
    };
    let primitive = Primitive { kind, fill: Some(FillStyle::Solid(RED)), stroke: Some(StrokeStyle::plain(2.0, Rgba8::BLACK)) };
    doc_with(Element::Shape(ShapeElement::from_primitive(primitive, Affine::translate((10.0, 10.0)))))
}

#[test]
fn a_width_profile_turns_the_stroke_into_a_pointed_outline() {
    let mut doc = AniDoc::default();
    let mut path = anim_core::BezPath::new();
    path.move_to((0.0, 50.0));
    path.line_to((200.0, 50.0));
    let shape = EdgeShape::from_paths(&[anim_shape::PathInput::new(path, None, Some(StrokeStyle::plain(12.0, RED)))]);
    edit::add_element(&mut doc.editor.document, &SCENE, 0, 0, Element::Shape(anim_scene::ShapeElement::new(shape))).unwrap();
    doc.editor.state.selection = Selection::one(ElementRef::top_level(0, 0, 0));
    assert!(rows(&doc).iter().any(|i| i == "stroke_profile"));
    // Uniform is what the stroke already is.
    set(&mut doc, "stroke_profile", json!("Uniform"));
    let Element::Shape(s) = selected(&doc) else { panic!("not a shape") };
    assert_eq!(s.shape.strokes.len(), 1);
    set(&mut doc, "stroke_profile", json!("Width Profile 1"));
    let Element::Shape(s) = selected(&doc) else { panic!("not a shape") };
    assert!(s.shape.strokes.is_empty(), "the stroke became a fill");
    assert!(s.shape.fill_at(anim_core::Point::new(100.0, 54.0)).is_some());
    assert!(s.shape.fill_at(anim_core::Point::new(3.0, 54.0)).is_none(), "pointed at the start");
}

#[test]
fn a_rectangle_primitive_shows_and_edits_its_parameters() {
    let mut doc = primitive_doc(false);
    let ids = rows(&doc);
    for id in ["prim_width", "prim_height", "prim_radius_tl", "prim_radius_br", "prim_lock"] {
        assert!(ids.iter().any(|i| i == id), "{id} in {ids:?}");
    }
    set(&mut doc, "prim_radius_tl", json!(20.0));
    set(&mut doc, "prim_width", json!(120.0));
    let Element::Shape(s) = selected(&doc) else { panic!("not a shape") };
    let b = s.shape.bounds().unwrap();
    assert!((b.width() - 120.0).abs() < 0.1, "{b:?}");
    assert!(s.shape.fill_at(anim_core::Point::new(1.0, 1.0)).is_none(), "the rounded corner is cut away");
    // Locking the radii copies the first one to all corners.
    set(&mut doc, "prim_lock", json!(true));
    set(&mut doc, "prim_radius_br", json!(5.0));
    let Element::Shape(s) = selected(&doc) else { panic!("not a shape") };
    let Some(anim_scene::PrimitiveKind::Rectangle(r)) = s.primitive.as_ref().map(|p| &p.kind) else { panic!("not a rectangle") };
    assert_eq!(r.radii, [5.0; 4]);
    // A fill change reaches the primitive, so the next parameter edit keeps the colour.
    set(&mut doc, "fill", json!("#00ff00"));
    set(&mut doc, "prim_height", json!(90.0));
    let Element::Shape(s) = selected(&doc) else { panic!("not a shape") };
    assert_eq!(s.shape.fills.first(), Some(&FillStyle::Solid(Rgba8 { r: 0, g: 255, b: 0, a: 255 })));
}

#[test]
fn an_oval_primitive_edits_its_arc() {
    let mut doc = primitive_doc(true);
    set(&mut doc, "prim_end", json!(90.0));
    set(&mut doc, "prim_inner", json!(50.0));
    let Element::Shape(s) = selected(&doc) else { panic!("not a shape") };
    let Some(anim_scene::PrimitiveKind::Oval(o)) = s.primitive.as_ref().map(|p| &p.kind) else { panic!("not an oval") };
    assert_eq!((o.end_angle, o.inner_radius), (90.0, 50.0));
    assert!(s.shape.bounds().unwrap().width() < 80.0, "a quarter ring is narrower than the whole oval");
}

#[test]
fn every_row_the_panel_shows_can_be_set_to_its_own_value() {
    // A panel edit that commits the value a row already holds must never fail, whatever the selection.
    let mut docs = vec![AniDoc::default(), doc_with_instance(), component_doc(), shape_doc(), text_doc("abc"), primitive_doc(false), primitive_doc(true)];
    let mut with_filters = doc_with_instance();
    set(&mut with_filters, "filter.add", json!("Drop Shadow"));
    set(&mut with_filters, "effect", json!("Advanced"));
    docs.push(with_filters);
    for mut doc in docs {
        for prop in describe(&doc) {
            if prop.enabled && prop.id != "filter.add" && !prop.id.ends_with(".remove") {
                apply(&mut doc, &prop.id, &prop.value).unwrap_or_else(|e| panic!("{}: {e}", prop.id));
            }
        }
    }
}
