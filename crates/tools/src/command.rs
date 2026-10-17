//! What tools ask the editor to do.
//!
//! Tools never touch the document; they return these and the editor applies them (see [`crate::apply`] for the
//! reference implementation used by the app, the CLI and the MCP server).

use anim_core::{Affine, Matrix3D, Point, Rect, Vec2};
use anim_scene::{ArmatureKind, Element, Pose, Primitive};
use anim_shape::{EdgeShape, FillStyle, StrokeStyle};
use serde::{Deserialize, Serialize};

use crate::selection::{ElementRef, Selection};
use crate::tool::ToolId;

/// How the caret should move; the host resolves it with the text layout (visual order matters for right-to-left
/// text) and answers with a `SetCaret` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaretMotion {
    /// One position to the left on screen (backwards in left-to-right text, forwards in right-to-left text).
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    WordLeft,
    WordRight,
    DocumentStart,
    DocumentEnd,
}

/// One edit for the editor to apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ToolCommand {
    /// Replaces the selection.
    SetSelection { selection: Selection },
    /// Deletes the selected elements.
    DeleteSelection,
    /// Duplicates the selected elements in place; the copies become the selection.
    DuplicateSelection,
    /// Multiplies the matrices of the selected elements by `matrix` (in stage coordinates, applied after the
    /// element's own matrix).
    TransformSelection { matrix: Affine },
    /// Applies a 3D matrix to the selected symbol instances (the 3D Rotation and 3D Translation tools). The matrix is
    /// in stage coordinates (x right, y down, z away from the viewer); rotations in it are about the origin, so a
    /// rotation about a centre point is `translate(-c) * rotate * translate(c)`.
    Transform3DSelection { matrix: Matrix3D },
    /// Moves the centre point the 3D tools rotate about (stage coordinates); `None` returns it to the instance's
    /// own origin.
    SetCenterPoint3D { target: ElementRef, point: Option<Point> },
    /// Moves the transformation point of an element (stage coordinates).
    SetTransformationPoint { target: ElementRef, point: Point },
    /// Draws a shape on a layer. In merge mode (`object_drawing` false) the shape merges with the layer's other
    /// merge-mode shapes: its fills replace what lies underneath and cut strokes. In object drawing mode it becomes
    /// a separate drawing object.
    DrawShape { layer: usize, frame: u32, shape: EdgeShape, object_drawing: bool },
    /// Adds any element at the end of the keyframe covering `frame`.
    AddElement { layer: usize, frame: u32, element: Element },
    /// Replaces the geometry and styles of a shape element.
    ReplaceShape { target: ElementRef, shape: EdgeShape },
    /// Removes elements (for example shapes that were erased completely).
    DeleteElements { targets: Vec<ElementRef> },
    /// Selects what lies inside a lasso polygon (stage pixels, implicitly closed) on the unlocked layers at the
    /// playhead. Merge-mode shapes are cut along the polygon and the part inside becomes an element of its own.
    /// With `additive` the current selection is kept.
    LassoSelect { polygon: Vec<Point>, additive: bool },
    /// The magic wand: selects the fills whose colour is within `threshold` (per channel) of the fill at `point`.
    WandSelect { point: Point, threshold: f64, additive: bool },
    /// Replaces the parameters of a Rectangle or Oval Primitive and regenerates its shape.
    SetPrimitive { target: ElementRef, primitive: Primitive },
    /// Inserts text at a byte offset of the field's concatenated text (always on a character boundary).
    InsertText { target: ElementRef, offset: usize, text: String },
    /// Deletes the byte range `start..end` of the field's text.
    DeleteText { target: ElementRef, start: usize, end: usize },
    /// Sets the width of a text field's box and whether the box grows with its text (dragging the box handle makes the
    /// width fixed, double-clicking it makes the box auto-grow again).
    SetTextBox { target: ElementRef, width: f64, auto_expand: bool },
    /// Moves the caret as the text layout sees fit.
    CaretMotion { motion: CaretMotion, extend: bool },
    /// A text field was entered for editing.
    BeginTextEdit { target: ElementRef },
    /// Editing of the text field ended.
    EndTextEdit,
    /// Enters a group: picking is limited to its members.
    EnterGroup { target: ElementRef },
    /// Switches the editing timeline to a symbol instance's symbol.
    EnterSymbol { target: ElementRef },
    /// Leaves the group or symbol being edited.
    ExitScope,
    /// Loads a fill and/or stroke into the toolbox (eyedropper).
    SetToolStyle { fill: Option<FillStyle>, stroke: Option<StrokeStyle> },
    /// Adds a bone to the armature of `layer` (creating the armature of the given kind when there is none). `head`
    /// and `tail` are stage pixels in the pose shown at `frame`; a child's head snaps to its parent's tip. The bone
    /// moves `element` of the layer's first keyframe, when given.
    AddBone { layer: usize, frame: u32, parent: Option<usize>, head: Point, tail: Point, element: Option<usize>, kind: ArmatureKind, name: Option<String> },
    /// Sets the armature pose of the keyframe at `frame`, making `frame` a pose keyframe when it is not one yet.
    SetPose { layer: usize, frame: u32, pose: Pose },
    /// Limits how far a bone may rotate away from its rest angle (radians), or removes the limits.
    SetBoneLimits { layer: usize, bone: usize, limits: Option<(f64, f64)> },
    /// Renames a bone.
    RenameBone { layer: usize, bone: usize, name: String },
    /// Removes a bone and the bones below it.
    RemoveBone { layer: usize, bone: usize },
    /// Removes the armature of a layer; the layer shows its artwork at rest again.
    RemoveArmature { layer: usize },
    /// Switches the active tool.
    ActivateTool { tool: ToolId },
    /// Pans the stage view by a screen-space delta (points).
    PanScreen { delta: Vec2 },
    /// Zooms the stage view about a stage point.
    ZoomAt { center: Point, factor: f64 },
    /// Zooms the stage view to show a stage rectangle.
    ZoomToRect { rect: Rect },
}

impl ToolCommand {
    /// True for commands that change the document (and so make an undo step); false for view, tool and selection
    /// state.
    #[must_use]
    pub const fn edits_document(&self) -> bool {
        matches!(
            self,
            Self::DeleteSelection
                | Self::DuplicateSelection
                | Self::TransformSelection { .. }
                | Self::Transform3DSelection { .. }
                | Self::SetCenterPoint3D { .. }
                | Self::SetTransformationPoint { .. }
                | Self::DrawShape { .. }
                | Self::AddElement { .. }
                | Self::ReplaceShape { .. }
                | Self::DeleteElements { .. }
                | Self::LassoSelect { .. }
                | Self::WandSelect { .. }
                | Self::SetPrimitive { .. }
                | Self::InsertText { .. }
                | Self::DeleteText { .. }
                | Self::SetTextBox { .. }
                | Self::AddBone { .. }
                | Self::SetPose { .. }
                | Self::SetBoneLimits { .. }
                | Self::RenameBone { .. }
                | Self::RemoveBone { .. }
                | Self::RemoveArmature { .. }
        )
    }
}
