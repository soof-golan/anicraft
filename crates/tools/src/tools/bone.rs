//! The Bone tool: drag out bones on a layer's artwork, then pose them with inverse kinematics.
//!
//! * Drag on empty stage (or on a symbol) to start an armature; the bone runs from the press to the release.
//! * Press the tip of a bone and drag to add a child bone there.
//! * Drag the middle of a bone to pose it: the bones from the root down to it turn so that its tip follows the
//!   pointer (FABRIK), within the joints' limits. Dragging a root bone's head moves the whole armature.
//!
//! The Selection tool poses an armature the same way when the press lands on a bone of the current layer.
//! Posing writes the pose of the keyframe under the playhead, making it a pose keyframe first when needed.

use anim_core::{Affine, Point, Vec2};
use anim_scene::{Armature, ArmatureKind, Element, Layer, Pose};
use anim_timeline::armature::pose_at;

use crate::armature::{Grab, grab_at, solve_pose, translated};
use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerEvent, PointerKind, ToolEvent};
use crate::overlay::{Cursor, Marker, Overlay, Role};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Shortest bone (stage pixels) a drag still makes.
const MIN_BONE: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum PoseMode {
    /// Move the tip of the bone with IK.
    Ik { bone: usize, tip: Point },
    /// Move the whole armature.
    Translate,
}

#[derive(Debug, Clone, Default)]
enum State {
    #[default]
    Idle,
    Creating {
        layer: usize,
        frame: u32,
        parent: Option<usize>,
        head: Point,
        current: Point,
    },
    Posing {
        layer: usize,
        frame: u32,
        start: Point,
        base: Pose,
        mode: PoseMode,
        last: Option<Pose>,
    },
}

/// Draws bones and poses armatures.
#[derive(Debug, Clone, Default)]
pub struct BoneTool {
    state: State,
    /// Only pose; never start a bone (the Selection tool routes presses on bones here).
    pose_only: bool,
    hover: Option<Grab>,
}

/// The armature of the current layer and its pose at the playhead.
fn current<'a>(ctx: &ToolContext<'a>) -> Option<(&'a Layer, &'a Armature, Pose)> {
    let layer = ctx.layer_ref(ctx.layer)?;
    let armature = layer.armature.as_ref()?;
    let pose = pose_at(layer, ctx.frame)?;
    Some((layer, armature, pose))
}

/// True when a press at `p` lands on a bone of the current layer's armature.
#[must_use]
pub fn bone_under(ctx: &ToolContext<'_>, p: Point) -> bool {
    ctx.layer_editable(ctx.layer) && current(ctx).is_some_and(|(_, a, pose)| grab_at(a, &pose, p, ctx.tolerance()).is_some())
}

/// The motion that takes the rest geometry the new bone hangs from to where it is shown: the parent's motion, or
/// the whole-armature offset.
fn parent_motion(armature: Option<&Armature>, pose: &Pose, parent: Option<usize>) -> Affine {
    parent.and_then(|p| armature.and_then(|a| a.motions(pose).get(p).copied())).unwrap_or_else(|| Affine::translate(pose.offset))
}

/// The element of the first keyframe a new bone should move, from its end points (rest coordinates): a symbol
/// instance that no bone moves yet and whose bounds hold the middle of the bone (a bone drawn along a limb moves
/// that limb), else the tail, else the head.
fn element_for(ctx: &ToolContext<'_>, layer: &Layer, armature: Option<&Armature>, head: Point, tail: Point) -> Option<usize> {
    let rest = layer.rest_keyframe()?;
    let taken = |i: usize| armature.is_some_and(|a| a.bones.iter().any(|b| b.element == Some(i)));
    let holds = |p: Point| {
        rest.elements.iter().enumerate().rev().find_map(|(i, e)| {
            let ok = matches!(e, Element::SymbolInstance(_)) && !taken(i) && ctx.bounds_in_parent(e).is_some_and(|b| b.contains(p));
            ok.then_some(i)
        })
    };
    holds(head.midpoint(tail)).or_else(|| holds(tail)).or_else(|| holds(head))
}

/// What kind a new armature on `layer` is: shape bones when the artwork is vector shapes only.
fn kind_for(layer: &Layer) -> ArmatureKind {
    let Some(rest) = layer.rest_keyframe() else { return ArmatureKind::Symbol };
    let symbols = rest.elements.iter().any(|e| matches!(e, Element::SymbolInstance(_)));
    let shapes = rest.elements.iter().any(|e| matches!(e, Element::Shape(_)));
    if shapes && !symbols { ArmatureKind::Shape } else { ArmatureKind::Symbol }
}

impl BoneTool {
    /// Makes the tool only pose (see the module docs).
    pub const fn set_pose_only(&mut self, pose_only: bool) {
        self.pose_only = pose_only;
    }

    fn down(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let layer_index = ctx.layer;
        if !ctx.layer_editable(layer_index) {
            return ToolOutput::none();
        }
        let Some(layer) = ctx.layer_ref(layer_index) else { return ToolOutput::none() };
        if let Some((_, armature, pose)) = current(ctx)
            && let Some(grab) = grab_at(armature, &pose, p.pos, ctx.tolerance())
        {
            let segments = armature.posed_segments(&pose);
            let tip_of = |bone: usize| segments.get(bone).map_or(p.pos, |s| s.1);
            let mode = match grab {
                Grab::Tip(bone) if !self.pose_only => {
                    self.state = State::Creating { layer: layer_index, frame: ctx.frame, parent: Some(bone), head: tip_of(bone), current: p.pos };
                    return ToolOutput::cursor(Cursor::Point);
                }
                Grab::Tip(bone) | Grab::Body(bone) => PoseMode::Ik { bone, tip: tip_of(bone) },
                Grab::RootHead(_) => PoseMode::Translate,
            };
            self.state = State::Posing { layer: layer_index, frame: ctx.frame, start: p.pos, base: pose, mode, last: None };
            return ToolOutput::cursor(Cursor::Grabbing);
        }
        if self.pose_only || layer.frames.is_empty() {
            return ToolOutput::none();
        }
        self.state = State::Creating { layer: layer_index, frame: ctx.frame, parent: None, head: p.pos, current: p.pos };
        ToolOutput::cursor(Cursor::Crosshair)
    }

    fn drag(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match &mut self.state {
            State::Idle => ToolOutput::none(),
            State::Creating { current, head, .. } => {
                *current = constrained(*head, p);
                ToolOutput::cursor(Cursor::Crosshair)
            }
            State::Posing { layer, frame, start, base, mode, last } => {
                let Some(armature) = ctx.layer_ref(*layer).and_then(|l| l.armature.as_ref()) else { return ToolOutput::none() };
                let delta = p.pos - *start;
                let pose = match *mode {
                    PoseMode::Ik { bone, tip } => solve_pose(armature, base, bone, tip + delta),
                    PoseMode::Translate => translated(base, delta),
                };
                if last.as_ref() == Some(&pose) {
                    return ToolOutput::cursor(Cursor::Grabbing);
                }
                *last = Some(pose.clone());
                ToolOutput::with(Cursor::Grabbing, vec![ToolCommand::SetPose { layer: *layer, frame: *frame, pose }])
            }
        }
    }

    fn up(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let state = std::mem::take(&mut self.state);
        match state {
            State::Idle => ToolOutput::none(),
            State::Posing { .. } => ToolOutput::cursor(Cursor::Move),
            State::Creating { layer, frame, parent, head, .. } => {
                let tail = constrained(head, p);
                if head.distance(tail) < MIN_BONE.max(ctx.drag_threshold()) {
                    return ToolOutput::cursor(Cursor::Crosshair);
                }
                let Some(l) = ctx.layer_ref(layer) else { return ToolOutput::none() };
                let pose = pose_at(l, frame).unwrap_or_default();
                let to_rest = parent_motion(l.armature.as_ref(), &pose, parent).inverse();
                let element = element_for(ctx, l, l.armature.as_ref(), to_rest * head, to_rest * tail);
                let kind = l.armature.as_ref().map_or_else(|| kind_for(l), |a| a.kind);
                let command = ToolCommand::AddBone { layer, frame, parent, head, tail, element, kind, name: None };
                ToolOutput::with(Cursor::Crosshair, vec![command])
            }
        }
    }
}

/// The end of a bone dragged from `head`; shift snaps the direction to multiples of 45 degrees.
fn constrained(head: Point, p: &PointerEvent) -> Point {
    if !p.modifiers.shift {
        return p.pos;
    }
    let d = p.pos - head;
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (d.y.atan2(d.x) / step).round() * step;
    head + Vec2::from_angle(angle) * d.hypot()
}

impl Tool for BoneTool {
    fn id(&self) -> ToolId {
        ToolId::Bone
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => self.down(p, ctx),
                PointerKind::Move => self.drag(p, ctx),
                PointerKind::Up => self.up(p, ctx),
                PointerKind::Hover => {
                    self.hover = current(ctx).and_then(|(_, a, pose)| grab_at(a, &pose, p.pos, ctx.tolerance()));
                    match self.hover {
                        Some(Grab::Tip(_)) if !self.pose_only => ToolOutput::cursor(Cursor::Point),
                        Some(_) => ToolOutput::cursor(Cursor::Move),
                        None => ToolOutput::cursor(Cursor::Crosshair),
                    }
                }
            },
            ToolEvent::Key(k) if k.pressed && k.key == Key::Escape => {
                self.cancel();
                ToolOutput::none()
            }
            ToolEvent::Cancel => {
                self.cancel();
                ToolOutput::none()
            }
            _ => ToolOutput::none(),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        let mut out = bones_overlay(ctx);
        if let State::Creating { head, current, .. } = &self.state {
            out.push(bone_shape(*head, *current, 1.0 / ctx.zoom.max(1.0e-6), Role::Preview));
        }
        out
    }

    fn cancel(&mut self) {
        self.state = State::Idle;
    }

    fn is_busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }
}

/// The outline of one bone: a long diamond from `head` to `tip`, `unit` stage pixels per screen point.
fn bone_shape(head: Point, tip: Point, unit: f64, role: Role) -> Overlay {
    let d = tip - head;
    let len = d.hypot();
    if len < 1.0e-9 {
        return Overlay::Marker { pos: head, marker: Marker::Circle, role, filled: false };
    }
    let dir = d / len;
    let normal = Vec2::new(-dir.y, dir.x);
    let width = (len * 0.15).min(5.0 * unit);
    let shoulder = head + dir * (len * 0.2);
    Overlay::Polyline { points: vec![head, shoulder + normal * width, tip, shoulder - normal * width], closed: true, role }
}

/// The bones of the current layer's armature in the pose at the playhead, with a handle on every tip.
#[must_use]
pub fn bones_overlay(ctx: &ToolContext<'_>) -> Vec<Overlay> {
    let Some((_, armature, pose)) = current(ctx) else { return Vec::new() };
    let unit = 1.0 / ctx.zoom.max(1.0e-6);
    let mut out = Vec::new();
    for (i, (head, tip)) in armature.posed_segments(&pose).into_iter().enumerate() {
        out.push(bone_shape(head, tip, unit, Role::Guide));
        out.push(Overlay::Marker { pos: tip, marker: Marker::Circle, role: Role::Handle, filled: true });
        if armature.bones.get(i).is_some_and(|b| b.parent.is_none()) {
            out.push(Overlay::Marker { pos: head, marker: Marker::Diamond, role: Role::Pivot, filled: true });
        }
    }
    out
}
