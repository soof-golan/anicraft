//! Armature editing shared by the Bone tool, the command executor and the MCP façade: adding bones, posing with
//! inverse kinematics and picking bones.
//!
//! The maths of chains lives in `anim-rig` (FABRIK, the analytic two-bone solution and angle limits); this module maps
//! an [`Armature`] and a [`Pose`] onto a rig [`Chain`] and back.

use anim_core::{Affine, Point, Vec2};
use anim_rig::bones::normalize_angle;
use anim_rig::{AngleLimits, Bone as RigBone, Chain, IkOptions};
use anim_scene::{Armature, ArmatureBone, ArmatureKind, Layer, Pose};
use anim_timeline::armature::pose_at;
use thiserror::Error;

/// Why an armature edit was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArmatureError {
    #[error("the layer has no keyframe to hold the artwork")]
    NoArtwork,
    #[error("the layer has no armature")]
    NoArmature,
    #[error("no such bone: {0}")]
    NoSuchBone(usize),
    #[error("the bone is too short or the numbers are not finite")]
    BadBone,
    #[error("an armature may hold at most {0} bones")]
    TooMany(usize),
    #[error("element {0} does not exist or is already moved by another bone")]
    BadElement(usize),
}

/// Bone to add, in stage coordinates of the pose shown at `frame`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewBone {
    pub parent: Option<usize>,
    /// For a child the head snaps to the parent's tip.
    pub head: Point,
    pub tail: Point,
    /// Element of the layer's first keyframe the bone moves.
    pub element: Option<usize>,
    pub name: Option<String>,
    /// Kind of a new armature; ignored when the layer already has one.
    pub kind: ArmatureKind,
}

/// The rest-pose bone for a bone dragged out on the stage while the armature is in `pose`.
fn rest_bone(armature: &Armature, pose: &Pose, spec: &NewBone) -> Result<ArmatureBone, ArmatureError> {
    let motions = armature.motions(pose);
    let to_rest: Affine = match spec.parent {
        Some(p) => motions.get(p).copied().ok_or(ArmatureError::NoSuchBone(p))?.inverse(),
        None => Affine::translate(pose.offset).inverse(),
    };
    let tail = to_rest * spec.tail;
    let head = match spec.parent {
        Some(p) => armature.bones.get(p).map(ArmatureBone::tip).ok_or(ArmatureError::NoSuchBone(p))?,
        None => to_rest * spec.head,
    };
    let name = spec.name.clone().unwrap_or_else(|| format!("bone{}", armature.bones.len().saturating_add(1)));
    let bone = ArmatureBone::between(name, spec.parent, head, tail);
    if !bone.length.is_finite() || bone.length < 1.0e-6 || !bone.angle.is_finite() || !head.x.is_finite() || !head.y.is_finite() {
        return Err(ArmatureError::BadBone);
    }
    Ok(bone)
}

/// Adds a bone to `layer`, creating the armature when the layer has none. Returns the new bone's index.
///
/// # Errors
/// [`ArmatureError`] when the layer has no artwork keyframe, the parent or element do not exist, the bone has no
/// length, or the armature is full.
pub fn add_bone(layer: &mut Layer, frame: u32, spec: &NewBone) -> Result<usize, ArmatureError> {
    if layer.frames.is_empty() {
        return Err(ArmatureError::NoArtwork);
    }
    let pose = pose_at(layer, frame);
    let element_count = layer.rest_keyframe().map_or(0, |k| k.elements.len());
    let name = layer.name.clone();
    let armature = layer.armature.get_or_insert_with(|| Armature::new(name, spec.kind));
    let pose = pose.unwrap_or_else(|| armature.rest_pose());
    if let Some(e) = spec.element
        && (e >= element_count || armature.bones.iter().any(|b| b.element == Some(e)))
    {
        return Err(ArmatureError::BadElement(e));
    }
    let mut bone = rest_bone(armature, &pose, spec)?;
    bone.element = spec.element;
    let created_empty = armature.bones.is_empty();
    if let Some(index) = armature.push(bone) {
        return Ok(index);
    }
    if created_empty {
        layer.armature = None;
    }
    Err(ArmatureError::TooMany(anim_scene::armature::MAX_BONES))
}

/// Removes `bone` and its descendants from the layer's armature, dropping their entries from every pose. An
/// armature left without bones is removed (the poses with it).
///
/// # Errors
/// [`ArmatureError::NoArmature`] or [`ArmatureError::NoSuchBone`].
pub fn remove_bone(layer: &mut Layer, bone: usize) -> Result<(), ArmatureError> {
    let armature = layer.armature.as_mut().ok_or(ArmatureError::NoArmature)?;
    if bone >= armature.bones.len() {
        return Err(ArmatureError::NoSuchBone(bone));
    }
    let removed = armature.remove_subtree(bone);
    let empty = armature.bones.is_empty();
    for frame in &mut layer.frames {
        if let Some(pose) = &mut frame.pose {
            let mut i = 0usize;
            pose.angles.retain(|_| {
                let keep = !removed.contains(&i);
                i = i.saturating_add(1);
                keep
            });
        }
    }
    if empty {
        remove_armature(layer);
    }
    Ok(())
}

/// Turns an armature layer back into an ordinary layer showing its first keyframe's artwork at rest.
pub fn remove_armature(layer: &mut Layer) {
    layer.armature = None;
    for frame in &mut layer.frames {
        frame.pose = None;
    }
    // Pose keyframes hold no artwork of their own; drop them so the layer shows the rest artwork throughout.
    let first = layer.frames.first().map(|f| f.index);
    let end = layer.frame_count();
    layer.frames.retain(|f| Some(f.index) == first || !f.elements.is_empty());
    if let Some(f) = layer.frames.first_mut() {
        f.duration = end.saturating_sub(f.index).max(1);
        f.tween_type = anim_scene::TweenType::None;
    }
}

/// Sets the pose of the keyframe at `frame`, which must start there.
///
/// Clamps to the bones' limits. When the new pose
/// keyframe follows another pose keyframe that has no tween, that one starts a tween to it (armature layers tween by
/// default, as in Animate).
///
/// # Errors
/// [`ArmatureError::NoArmature`].
pub fn store_pose(layer: &mut Layer, frame: u32, pose: &Pose) -> Result<(), ArmatureError> {
    let armature = layer.armature.as_ref().ok_or(ArmatureError::NoArmature)?;
    let pose = armature.clamped(pose);
    let rest = armature.rest_pose();
    let first_index = layer.frames.first().map(|f| f.index).ok_or(ArmatureError::NoArtwork)?;
    let existed = layer.frames.iter().any(|f| f.index == frame && f.pose.is_some());
    if !existed
        && frame != first_index
        && let Some(prev) = layer.frames.iter_mut().rfind(|f| f.index < frame && f.pose.is_some())
    {
        if prev.tween_type == anim_scene::TweenType::None {
            prev.tween_type = anim_scene::TweenType::Classic;
            prev.key_mode = anim_scene::KEY_MODE_CLASSIC_TWEEN;
        }
    } else if !existed && frame != first_index && !layer.frames.iter().any(|f| f.index < frame && f.pose.is_some()) {
        // The first pose keyframe tweens from the rest pose, which the artwork keyframe stands for.
        if let Some(first) = layer.frames.first_mut() {
            first.pose = Some(rest);
            first.tween_type = anim_scene::TweenType::Classic;
            first.key_mode = anim_scene::KEY_MODE_CLASSIC_TWEEN;
        }
    }
    let target = layer.frames.iter_mut().find(|f| f.index == frame).ok_or(ArmatureError::NoArtwork)?;
    target.pose = Some(pose);
    Ok(())
}

fn rig_chain(armature: &Armature, ids: &[usize], base: &Pose) -> Option<(Chain, Vec<f64>)> {
    let first = armature.bones.get(*ids.first()?)?;
    let mut parent_rest = 0.0;
    let mut rest_rel = Vec::with_capacity(ids.len());
    let mut bones = Vec::with_capacity(ids.len());
    for id in ids {
        let b = armature.bones.get(*id)?;
        let rel = normalize_angle(b.angle - parent_rest);
        parent_rest = b.angle;
        rest_rel.push(rel);
        let limits = b.limits.map(|(lo, hi)| AngleLimits { min: rel + lo, max: rel + hi });
        bones.push(RigBone { length: b.length, angle: normalize_angle(rel + base.angle(*id)), limits });
    }
    Some((Chain::new(first.head + base.offset, bones), rest_rel))
}

/// The pose that moves the tip of `bone` towards `target` (stage pixels).
///
/// Rotates the bones from the root down to
/// it, starting from `base`. Bones off that chain keep their rotations and so follow their parents. Targets out of
/// reach stretch the chain towards them; limits are respected.
#[must_use]
pub fn solve_pose(armature: &Armature, base: &Pose, bone: usize, target: Point) -> Pose {
    let ids = armature.chain_to(bone);
    let Some((mut chain, rest_rel)) = rig_chain(armature, &ids, base) else { return base.clone() };
    chain.solve_ik(target, &IkOptions::default());
    let mut pose = base.clone();
    for ((id, solved), rel) in ids.iter().zip(&chain.bones).zip(&rest_rel) {
        let delta = normalize_angle(solved.angle - rel);
        if delta.is_finite() {
            pose.set_angle(*id, delta);
        }
    }
    armature.clamped(&pose)
}

/// What the pointer grabbed on an armature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    /// The tip of a bone: drag it with IK.
    Tip(usize),
    /// The head of a root bone: drag the whole armature.
    RootHead(usize),
    /// The middle of a bone: drag its tip with IK.
    Body(usize),
}

impl Grab {
    /// The bone whose tip IK moves (the root bone for a head grab).
    #[must_use]
    pub const fn bone(self) -> usize {
        match self {
            Self::Tip(b) | Self::RootHead(b) | Self::Body(b) => b,
        }
    }
}

/// The bone part within `tolerance` of `p` in `pose`, tips first, then root heads, then bodies.
#[must_use]
pub fn grab_at(armature: &Armature, pose: &Pose, p: Point, tolerance: f64) -> Option<Grab> {
    let segments = armature.posed_segments(pose);
    let nearest = |dist: &dyn Fn(usize, &(Point, Point)) -> f64| {
        segments.iter().enumerate().map(|(i, s)| (i, dist(i, s))).filter(|(_, d)| *d <= tolerance).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    };
    if let Some(i) = nearest(&|_, s| s.1.distance(p)) {
        return Some(Grab::Tip(i));
    }
    if let Some(i) = nearest(&|i, s| if armature.bones.get(i).is_some_and(|b| b.parent.is_none()) { s.0.distance(p) } else { f64::INFINITY }) {
        return Some(Grab::RootHead(i));
    }
    nearest(&|_, s| anim_scene::armature::segment_distance(s.0, s.1, p)).map(Grab::Body)
}

/// Moves every root joint by `delta` (stage pixels): the pose offset.
#[must_use]
pub fn translated(base: &Pose, delta: Vec2) -> Pose {
    Pose { offset: base.offset + delta, angles: base.angles.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_core::Rgba8;
    use anim_scene::{Element, Frame, SymbolInstance};

    fn layer_with_elements(n: usize) -> Layer {
        let mut l = Layer::new("rig", Rgba8::BLACK);
        let elements = (0..n).map(|i| Element::SymbolInstance(Box::new(SymbolInstance::new(format!("s{i}"), Affine::IDENTITY)))).collect();
        l.frames.push(Frame::new(0, 24, elements));
        l
    }

    fn spec(parent: Option<usize>, head: (f64, f64), tail: (f64, f64)) -> NewBone {
        NewBone { parent, head: Point::new(head.0, head.1), tail: Point::new(tail.0, tail.1), element: None, name: None, kind: ArmatureKind::Symbol }
    }

    fn two_bone_layer() -> Layer {
        let mut l = layer_with_elements(2);
        add_bone(&mut l, 0, &spec(None, (0.0, 0.0), (100.0, 0.0))).expect("root");
        add_bone(&mut l, 0, &spec(Some(0), (100.0, 0.0), (200.0, 0.0))).expect("child");
        l
    }

    #[test]
    fn bones_are_added_and_children_snap_to_the_parent_tip() {
        let mut l = layer_with_elements(1);
        assert_eq!(add_bone(&mut l, 0, &spec(None, (10.0, 10.0), (110.0, 10.0))), Ok(0));
        assert_eq!(add_bone(&mut l, 0, &spec(Some(0), (111.0, 12.0), (110.0, 90.0))), Ok(1));
        let a = l.armature.as_ref().expect("armature");
        assert!(a.bones[1].head.distance(Point::new(110.0, 10.0)) < 1.0e-9);
        assert!((a.bones[1].length - 80.0).abs() < 1.0e-9);
    }

    #[test]
    fn a_degenerate_bone_is_refused() {
        let mut l = layer_with_elements(1);
        assert_eq!(add_bone(&mut l, 0, &spec(None, (5.0, 5.0), (5.0, 5.0))), Err(ArmatureError::BadBone));
        assert!(l.armature.as_ref().is_none_or(|a| a.bones.is_empty()));
    }

    #[test]
    fn an_element_can_belong_to_one_bone_only() {
        let mut l = layer_with_elements(2);
        let mut s = spec(None, (0.0, 0.0), (50.0, 0.0));
        s.element = Some(1);
        assert_eq!(add_bone(&mut l, 0, &s), Ok(0));
        assert_eq!(add_bone(&mut l, 0, &s), Err(ArmatureError::BadElement(1)));
        s.element = Some(7);
        assert_eq!(add_bone(&mut l, 0, &s), Err(ArmatureError::BadElement(7)));
    }

    #[test]
    fn ik_puts_the_tip_on_a_reachable_target() {
        let l = two_bone_layer();
        let a = l.armature.as_ref().expect("armature");
        let pose = solve_pose(a, &a.rest_pose(), 1, Point::new(100.0, 100.0));
        let tip = a.posed_segments(&pose)[1].1;
        assert!(tip.distance(Point::new(100.0, 100.0)) < 1.0e-3, "{tip:?}");
        // The joint between the bones stays 100 px from the root.
        let joint = a.posed_segments(&pose)[0].1;
        assert!((joint.to_vec2().hypot() - 100.0).abs() < 1.0e-6);
    }

    #[test]
    fn ik_stretches_towards_an_unreachable_target() {
        let l = two_bone_layer();
        let a = l.armature.as_ref().expect("armature");
        let pose = solve_pose(a, &a.rest_pose(), 1, Point::new(0.0, 500.0));
        let tip = a.posed_segments(&pose)[1].1;
        assert!(tip.distance(Point::new(0.0, 200.0)) < 1.0e-3, "{tip:?}");
    }

    #[test]
    fn ik_respects_joint_limits() {
        let mut l = two_bone_layer();
        // The elbow may bend at most 20 degrees.
        l.armature.as_mut().expect("armature").bones[1].limits = Some(((-20.0_f64).to_radians(), 20.0_f64.to_radians()));
        let a = l.armature.as_ref().expect("armature");
        let pose = solve_pose(a, &a.rest_pose(), 1, Point::new(100.0, 100.0));
        assert!(pose.angle(1).abs() <= 20.0_f64.to_radians() + 1.0e-9, "{}", pose.angle(1));
        let tip = a.posed_segments(&pose)[1].1;
        assert!(tip.distance(Point::new(100.0, 100.0)) > 1.0, "the target is out of the limited reach");
    }

    #[test]
    fn the_bone_under_the_pointer_is_found() {
        let l = two_bone_layer();
        let a = l.armature.as_ref().expect("armature");
        let rest = a.rest_pose();
        assert_eq!(grab_at(a, &rest, Point::new(200.0, 2.0), 6.0), Some(Grab::Tip(1)));
        assert_eq!(grab_at(a, &rest, Point::new(1.0, 1.0), 6.0), Some(Grab::RootHead(0)));
        assert_eq!(grab_at(a, &rest, Point::new(150.0, 3.0), 6.0), Some(Grab::Body(1)));
        assert_eq!(grab_at(a, &rest, Point::new(150.0, 40.0), 6.0), None);
    }

    #[test]
    fn storing_poses_chains_tweens_and_clamps() {
        let mut l = two_bone_layer();
        l.armature.as_mut().expect("armature").bones[0].limits = Some((-1.0, 1.0));
        l.frames.push(Frame::blank(24, 1));
        let mut pose = Pose::rest(2);
        pose.set_angle(0, 3.0);
        store_pose(&mut l, 24, &pose).expect("store");
        assert!((l.frames[1].pose.as_ref().expect("pose").angle(0) - 1.0).abs() < 1.0e-12, "clamped");
        assert_eq!(l.frames[0].tween_type, anim_scene::TweenType::Classic, "tweens from rest");
        assert!(l.frames[0].pose.is_some());
        let mut pose2 = Pose::rest(2);
        pose2.set_angle(0, -0.5);
        l.frames.push(Frame::blank(40, 1));
        store_pose(&mut l, 40, &pose2).expect("store");
        assert_eq!(l.frames[1].tween_type, anim_scene::TweenType::Classic);
    }

    #[test]
    fn removing_a_bone_trims_the_poses() {
        let mut l = two_bone_layer();
        add_bone(&mut l, 0, &spec(Some(0), (100.0, 0.0), (100.0, 50.0))).expect("third");
        l.frames[0].pose = Some(Pose { offset: Vec2::ZERO, angles: vec![0.1, 0.2, 0.3] });
        remove_bone(&mut l, 1).expect("remove");
        assert_eq!(l.armature.as_ref().expect("armature").bones.len(), 2);
        assert_eq!(l.frames[0].pose.as_ref().expect("pose").angles, vec![0.1, 0.3]);
        remove_bone(&mut l, 0).expect("remove all");
        assert!(l.armature.is_none());
        assert!(l.frames[0].pose.is_none());
    }
}
