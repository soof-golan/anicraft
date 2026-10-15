//! `<DOMTimeline>`, `<DOMLayer>` and `<DOMFrame>`.

use anim_scene::{
    EaseCurve, EaseTarget, EnvelopePoint, Extra, Frame, FrameSound, KEY_MODE_NORMAL, LAYER_COLORS, LabelType, Layer, LayerKind, SoundLoop, SoundSync, Timeline,
    TweenEase, TweenRotate, TweenType,
};
use roxmltree::Node;

use crate::element::read_elements;
use crate::xml::{Attrs, Ctx, child, element_children, read_point_element, text_of};

/// Most layers read from one timeline.
pub const MAX_LAYERS: usize = 10_000;
/// Most keyframes read from one layer.
pub const MAX_FRAMES: usize = 100_000;

/// Reads a `<DOMTimeline>`.
pub fn read_timeline(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> Timeline {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("name").unwrap_or_default();
    let mut timeline = Timeline::new(name);
    timeline.current_frame = attrs.u32(ctx, "currentFrame").unwrap_or(0);
    timeline.layer_depth_enabled = attrs.bool(ctx, "layerDepthEnabled");
    for c in element_children(node) {
        if c.tag_name().name() == "layers" {
            for layer in element_children(c) {
                if layer.tag_name().name() != "DOMLayer" {
                    ctx.keep_child(&mut timeline.extra, "layers", layer);
                    continue;
                }
                if timeline.layers.len() >= MAX_LAYERS {
                    ctx.warn("timeline: too many layers, the rest are dropped");
                    break;
                }
                let index = timeline.layers.len();
                timeline.layers.push(read_layer(ctx, layer, index));
            }
        } else {
            ctx.keep_child(&mut timeline.extra, "DOMTimeline", c);
        }
    }
    attrs.rest(ctx, &mut timeline.extra);
    timeline
}

fn read_layer(ctx: &mut Ctx<'_>, node: Node<'_, '_>, index: usize) -> Layer {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("name").unwrap_or_default();
    let color = attrs.color(ctx, "color").unwrap_or_else(|| LAYER_COLORS.first().copied().unwrap_or(anim_core::Rgba8::BLACK));
    let mut layer = Layer::new(name, color);
    layer.kind = attrs.take_with(ctx, "layerType", LayerKind::from_xfl_name).unwrap_or(LayerKind::Normal);
    layer.locked = attrs.bool(ctx, "locked").unwrap_or(false);
    layer.visible = attrs.bool(ctx, "visible").unwrap_or(true);
    layer.outline = attrs.bool(ctx, "outline").unwrap_or(false);
    // Animate writes `autoNamed="false"` for layers the user named; a missing attribute means the name is generated.
    layer.autoname = attrs.bool(ctx, "autoNamed").unwrap_or(true);
    if let Some(parent) = attrs.u32(ctx, "parentLayerIndex") {
        match usize::try_from(parent) {
            Ok(p) if p < index => layer.parent_layer_index = Some(p),
            _ => {
                ctx.warn(format!("layer {index}: parentLayerIndex {parent} is not an earlier layer; kept as a plain attribute"));
                layer.extra.attrs.push(("parentLayerIndex".to_owned(), parent.to_string()));
            }
        }
    }
    for c in element_children(node) {
        if c.tag_name().name() == "frames" {
            read_frames(ctx, c, &mut layer);
        } else {
            ctx.keep_child(&mut layer.extra, "DOMLayer", c);
        }
    }
    attrs.rest(ctx, &mut layer.extra);
    layer
}

fn read_frames(ctx: &mut Ctx<'_>, node: Node<'_, '_>, layer: &mut Layer) {
    let mut next_index = 0u32;
    for c in element_children(node) {
        if c.tag_name().name() != "DOMFrame" {
            ctx.keep_child(&mut layer.extra, "frames", c);
            continue;
        }
        if layer.frames.len() >= MAX_FRAMES {
            ctx.warn(format!("layer {}: too many keyframes, the rest are dropped", layer.name));
            break;
        }
        let frame = read_frame(ctx, c, next_index);
        next_index = frame.end();
        layer.frames.push(frame);
    }
    let sorted = layer.frames.windows(2).all(|w| w.first().zip(w.get(1)).is_none_or(|(a, b)| a.index <= b.index));
    if !sorted {
        ctx.warn(format!("layer {}: keyframes are not in order; sorted", layer.name));
        layer.sort_frames();
    }
}

fn read_frame(ctx: &mut Ctx<'_>, node: Node<'_, '_>, default_index: u32) -> Frame {
    let mut attrs = Attrs::new(node);
    let index = attrs.u32(ctx, "index").unwrap_or(default_index);
    let duration = attrs.u32(ctx, "duration").unwrap_or(1);
    let mut frame = Frame::new(index, duration, Vec::new());
    frame.key_mode = attrs.u32(ctx, "keyMode").unwrap_or(KEY_MODE_NORMAL);
    frame.tween_type = attrs.take_with(ctx, "tweenType", TweenType::from_xfl_name).unwrap_or(TweenType::None);
    frame.motion_tween_rotate = attrs.take_with(ctx, "motionTweenRotate", TweenRotate::from_xfl_name).unwrap_or(TweenRotate::Auto);
    frame.motion_tween_rotate_times = attrs.f64(ctx, "motionTweenRotateTimes");
    frame.motion_tween_scale = attrs.bool(ctx, "motionTweenScale").unwrap_or(true);
    frame.motion_tween_snap = attrs.bool(ctx, "motionTweenSnap").unwrap_or(false);
    frame.motion_tween_orient_to_path = attrs.bool(ctx, "motionTweenOrientToPath").unwrap_or(false);
    frame.motion_tween_sync = attrs.bool(ctx, "motionTweenSync").unwrap_or(false);
    frame.acceleration = attrs.i32(ctx, "acceleration").unwrap_or(0).clamp(-100, 100);
    frame.ease_method_name = attrs.string("easeMethodName");
    // Derived from the presence of `<CustomEase>` children; the writer sets it again.
    let _ = attrs.take("hasCustomEase");
    frame.label = attrs.string("name");
    frame.label_type = attrs.take_with(ctx, "labelType", LabelType::from_xfl_name);
    let sound_name = attrs.string("soundName");
    let sound_sync = attrs.take_with(ctx, "soundSync", SoundSync::from_xfl_name);
    let sound_loop_mode = attrs.string("soundLoopMode");
    let repeat_count = attrs.u32(ctx, "repeatCount");
    let sound_effect = attrs.string("soundEffect");
    let mut sound_extra = Extra::new();
    for key in ["inPoint44", "outPoint44", "soundZoomLevel"] {
        if let Some(v) = attrs.string(key) {
            sound_extra.attrs.push((key.to_owned(), v));
        }
    }
    let mut envelope = Vec::new();
    for c in element_children(node) {
        match c.tag_name().name() {
            "elements" => frame.elements = read_elements(ctx, c, 0),
            "tweens" => read_tweens(ctx, c, &mut frame),
            "MorphShape" => frame.morph_shape_xml = Some(ctx.raw(c)),
            "motionObjectXML" => frame.motion_object_xml = Some(ctx.raw(c)),
            "SoundEnvelope" => envelope = read_envelope(ctx, c),
            "Actionscript" => read_actionscript(ctx, c, &mut frame),
            _ => ctx.keep_child(&mut frame.extra, "DOMFrame", c),
        }
    }
    if let Some(name) = sound_name {
        let loop_mode = match (sound_loop_mode.as_deref(), repeat_count) {
            (Some("loop"), _) => SoundLoop::Forever,
            (_, Some(n)) if n > 1 => SoundLoop::Repeat(n),
            _ => SoundLoop::Once,
        };
        frame.sound = Some(FrameSound { name, sync: sound_sync.unwrap_or_default(), loop_mode, effect: sound_effect, envelope, extra: sound_extra });
    } else {
        // Sound attributes without a sound name cannot form a `FrameSound`; keep them where they were.
        for (key, value) in [
            ("soundSync", sound_sync.map(|s| s.xfl_name().to_owned())),
            ("soundLoopMode", sound_loop_mode),
            ("repeatCount", repeat_count.map(|n| n.to_string())),
            ("soundEffect", sound_effect),
        ] {
            if let Some(v) = value {
                frame.extra.attrs.push((key.to_owned(), v));
            }
        }
        frame.extra.attrs.extend(sound_extra.attrs);
    }
    attrs.rest(ctx, &mut frame.extra);
    frame
}

fn read_tweens(ctx: &mut Ctx<'_>, node: Node<'_, '_>, frame: &mut Frame) {
    for c in element_children(node) {
        match c.tag_name().name() {
            "Ease" => {
                let mut attrs = Attrs::new(c);
                let target = attrs.take_with(ctx, "target", EaseTarget::from_xfl_name).unwrap_or(EaseTarget::All);
                let method = attrs.string("method").unwrap_or_default();
                attrs.rest_lost(ctx);
                frame.eases.push(TweenEase { target, curve: EaseCurve::Named(method) });
            }
            "CustomEase" => {
                let mut attrs = Attrs::new(c);
                let target = attrs.take_with(ctx, "target", EaseTarget::from_xfl_name).unwrap_or(EaseTarget::All);
                attrs.rest_lost(ctx);
                let points: Vec<[f64; 2]> = element_children(c)
                    .filter(|p| p.tag_name().name() == "Point")
                    .take(1_000)
                    .map(|p| {
                        let point = read_point_element(ctx, p);
                        [point.x, point.y]
                    })
                    .collect();
                frame.eases.push(TweenEase { target, curve: EaseCurve::Custom(points) });
            }
            _ => ctx.keep_child(&mut frame.extra, "tweens", c),
        }
    }
}

fn read_envelope(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> Vec<EnvelopePoint> {
    let mut out = Vec::new();
    for c in element_children(node).take(10_000) {
        if c.tag_name().name() != "SoundEnvelopePoint" {
            ctx.lost_attr("SoundEnvelope", c.tag_name().name());
            continue;
        }
        let mut attrs = Attrs::new(c);
        let mark44 = attrs.u32(ctx, "mark44").unwrap_or(0);
        let level0 = attrs.u32(ctx, "level0").unwrap_or(32768);
        let level1 = attrs.u32(ctx, "level1").unwrap_or(32768);
        attrs.rest_lost(ctx);
        out.push(EnvelopePoint { mark44, level0, level1 });
    }
    out
}

fn read_actionscript(ctx: &mut Ctx<'_>, node: Node<'_, '_>, frame: &mut Frame) {
    let simple = element_children(node).count() == 1 && child(node, "script").is_some() && node.attributes().len() == 0 && frame.actionscript.is_none();
    if simple && let Some(script) = child(node, "script") {
        frame.actionscript = Some(text_of(script));
    } else {
        ctx.keep_child(&mut frame.extra, "DOMFrame", node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Sink;

    fn read(text: &str) -> (Timeline, Sink) {
        let doc = roxmltree::Document::parse(text).unwrap();
        let mut sink = Sink::default();
        let timeline = {
            let mut ctx = Ctx::new(text, "t.xml", &mut sink);
            read_timeline(&mut ctx, doc.root_element())
        };
        (timeline, sink)
    }

    #[test]
    fn layers_frames_and_layer_flags() {
        let (t, sink) = read(
            r##"<DOMTimeline name="Scene 1" currentFrame="4" layerDepthEnabled="true"><layers>
            <DOMLayer name="Folder" color="#112233" layerType="folder" autoNamed="false"><frames><DOMFrame index="0" keyMode="9728"><elements/></DOMFrame></frames></DOMLayer>
            <DOMLayer name="Child" color="#00FF00" parentLayerIndex="0" visible="false" locked="true" current="true"><frames>
              <DOMFrame index="0" duration="5" keyMode="9728"><elements/></DOMFrame>
              <DOMFrame index="5" keyMode="22017" tweenType="motion" motionTweenSnap="true" acceleration="-40" name="go" labelType="name" rigPropagationMatrix="{}"><elements/></DOMFrame></frames></DOMLayer>
            </layers></DOMTimeline>"##,
        );
        assert_eq!(t.name, "Scene 1");
        assert_eq!(t.current_frame, 4);
        assert_eq!(t.layer_depth_enabled, Some(true));
        assert_eq!(t.layers.len(), 2);
        let folder = t.layers.first().unwrap();
        assert_eq!(folder.kind, LayerKind::Folder);
        assert!(!folder.autoname);
        let child = t.layers.get(1).unwrap();
        assert_eq!(child.parent_layer_index, Some(0));
        assert!(!child.visible && child.locked && child.autoname);
        assert_eq!(child.frames.len(), 2);
        let f = child.frames.get(1).unwrap();
        assert_eq!((f.index, f.duration), (5, 1));
        assert_eq!(f.tween_type, TweenType::Classic);
        assert!(f.motion_tween_snap);
        assert_eq!(f.acceleration, -40);
        assert_eq!(f.label.as_deref(), Some("go"));
        assert_eq!(f.label_type, Some(LabelType::Name));
        assert_eq!(f.extra.attr("rigPropagationMatrix"), Some("{}"));
        assert_eq!(sink.unknowns.ui_state, 1);
        assert_eq!(t.frame_count(), 6);
    }

    #[test]
    fn bad_parent_layer_index_is_kept_as_attribute() {
        let (t, sink) = read(r#"<DOMTimeline><layers><DOMLayer name="a" parentLayerIndex="3"/></layers></DOMTimeline>"#);
        let l = t.layers.first().unwrap();
        assert_eq!(l.parent_layer_index, None);
        assert_eq!(l.extra.attr("parentLayerIndex"), Some("3"));
        assert!(!sink.warnings.is_empty());
    }

    #[test]
    fn tweens_sounds_scripts_and_morph_shapes() {
        let (t, _) = read(
            r#"<DOMTimeline><layers><DOMLayer name="a"><frames>
            <DOMFrame index="0" duration="10" tweenType="shape" keyMode="17922" hasCustomEase="true" soundName="s.wav" soundSync="stream" soundLoopMode="loop">
              <tweens><Ease target="position" method="cubicIn"/><CustomEase target="all"><Point/><Point x="0.25" y="0.5"/><Point x="1" y="1"/></CustomEase></tweens>
              <SoundEnvelope><SoundEnvelopePoint mark44="100" level0="10" level1="20"/></SoundEnvelope>
              <MorphShape><morphSegments/></MorphShape>
              <Actionscript><script><![CDATA[stop();]]></script></Actionscript>
              <elements/></DOMFrame></frames></DOMLayer></layers></DOMTimeline>"#,
        );
        let f = t.layers.first().unwrap().frames.first().unwrap();
        assert_eq!(f.tween_type, TweenType::Shape);
        assert_eq!(f.eases.len(), 2);
        assert!(matches!(f.eases.get(1), Some(TweenEase { target: EaseTarget::All, curve: EaseCurve::Custom(p) }) if p.len() == 3));
        let s = f.sound.as_ref().unwrap();
        assert_eq!((s.name.as_str(), s.sync, s.loop_mode), ("s.wav", SoundSync::Stream, SoundLoop::Forever));
        assert_eq!(s.envelope, vec![EnvelopePoint { mark44: 100, level0: 10, level1: 20 }]);
        assert_eq!(f.morph_shape_xml.as_deref(), Some("<MorphShape><morphSegments/></MorphShape>"));
        assert_eq!(f.actionscript.as_deref(), Some("stop();"));
        assert!(f.extra.is_empty());
    }

    #[test]
    fn unsorted_and_overflowing_frames_do_not_panic() {
        let (t, sink) = read(
            r#"<DOMTimeline><layers><DOMLayer name="a"><frames><DOMFrame index="9" duration="4294967295"/><DOMFrame index="2"/></frames></DOMLayer></layers></DOMTimeline>"#,
        );
        let l = t.layers.first().unwrap();
        assert_eq!(l.frames.first().map(|f| f.index), Some(2));
        assert!(!sink.warnings.is_empty());
        assert_eq!(l.frame_count(), u32::MAX);
    }
}
