//! `<armature>` inside a `<DOMLayer>` and `<pose>` inside a `<DOMFrame>`: the bones of an inverse-kinematics layer
//! and the pose a keyframe sets.
//!
//! ```xml
//! <DOMLayer name="Armature_1" ...>
//!   <armature name="Armature_1" kind="symbol">
//!     <Bone name="upper" x="0" y="0" length="100" angle="0" minAngle="-45" maxAngle="45" element="0"/>
//!     <Bone name="lower" parent="0" x="100" y="0" length="80" angle="0" element="1"/>
//!   </armature>
//!   <frames>
//!     <DOMFrame index="0" duration="12" tweenType="motion"><pose x="0" y="0" angles="0 30"/> ... </DOMFrame>
//! ```
//!
//! Angles are degrees in the file (radians in the model); `x`/`y` of a `Bone` is its head at rest, `angle` its rest
//! direction (clockwise on the stage), `minAngle`/`maxAngle` the allowed pose rotation relative to rest, `parent` an
//! earlier bone's index, `element` the index of the element of the layer's first keyframe it moves. In a `pose`,
//! `angles` holds one rotation per bone (space separated) and `x`/`y` translate the whole armature.

use anim_core::{Point, Vec2};
use anim_scene::armature::MAX_BONES;
use anim_scene::{Armature, ArmatureBone, ArmatureKind, Extra, Pose};
use roxmltree::Node;

use crate::xml::{Attrs, Ctx, element_children};

/// Most numbers read from one `angles` list.
const MAX_ANGLES: usize = MAX_BONES;

/// Reads an `<armature>`.
pub fn read_armature(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> Armature {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("name").unwrap_or_default();
    let kind = attrs.take_with(ctx, "kind", ArmatureKind::from_xfl_name).unwrap_or_default();
    let mut armature = Armature::new(name, kind);
    for c in element_children(node) {
        if c.tag_name().name() != "Bone" {
            ctx.keep_child(&mut armature.extra, "armature", c);
            continue;
        }
        if armature.bones.len() >= MAX_BONES {
            ctx.warn("armature: too many bones, the rest are dropped");
            break;
        }
        let index = armature.bones.len();
        let bone = read_bone(ctx, c, index);
        armature.bones.push(bone);
    }
    attrs.rest(ctx, &mut armature.extra);
    armature
}

fn read_bone(ctx: &mut Ctx<'_>, node: Node<'_, '_>, index: usize) -> ArmatureBone {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("name").unwrap_or_default();
    let x = attrs.f64(ctx, "x").unwrap_or(0.0);
    let y = attrs.f64(ctx, "y").unwrap_or(0.0);
    let length = attrs.f64(ctx, "length").filter(|l| l.is_finite() && *l >= 0.0).unwrap_or(0.0);
    let angle = attrs.f64(ctx, "angle").unwrap_or(0.0).to_radians();
    let min = attrs.f64(ctx, "minAngle");
    let max = attrs.f64(ctx, "maxAngle");
    let element = attrs.u32(ctx, "element").and_then(|e| usize::try_from(e).ok());
    let mut parent = None;
    if let Some(p) = attrs.u32(ctx, "parent") {
        match usize::try_from(p) {
            Ok(p) if p < index => parent = Some(p),
            _ => ctx.warn(format!("armature bone {index}: parent {p} is not an earlier bone; the bone becomes a root")),
        }
    }
    let limits = match (min, max) {
        (None, None) => None,
        (lo, hi) => Some((lo.unwrap_or(-180.0).to_radians(), hi.unwrap_or(180.0).to_radians())),
    };
    let mut bone = ArmatureBone {
        name,
        parent,
        head: Point::new(finite(x), finite(y)),
        length,
        angle: finite(angle),
        limits: limits.filter(|(lo, hi)| lo.is_finite() && hi.is_finite()),
        element,
        extra: Extra::new(),
    };
    for c in element_children(node) {
        ctx.keep_child(&mut bone.extra, "Bone", c);
    }
    attrs.rest(ctx, &mut bone.extra);
    bone
}

const fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// Reads a `<pose>`.
pub fn read_pose(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> Pose {
    let mut attrs = Attrs::new(node);
    let x = attrs.f64(ctx, "x").map_or(0.0, finite);
    let y = attrs.f64(ctx, "y").map_or(0.0, finite);
    let list = attrs.string("angles").unwrap_or_default();
    let mut angles = Vec::new();
    for word in list.split_whitespace().take(MAX_ANGLES) {
        match crate::xml::parse_f64(word) {
            Some(deg) if deg.is_finite() => angles.push(deg.to_radians()),
            _ => {
                ctx.warn(format!("pose: '{word}' is not a number; read as 0"));
                angles.push(0.0);
            }
        }
    }
    attrs.rest_lost(ctx);
    Pose { offset: Vec2::new(x, y), angles }
}
