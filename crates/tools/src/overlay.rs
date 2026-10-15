//! Overlay primitives and cursor hints that tools hand to the stage view.
//!
//! The vocabulary mirrors `anim_ui::stage::Overlay` (rectangles, lines, polylines, bezier nodes with handles,
//! marquees, markers, crosshairs) so the application converts one to the other mechanically, but it carries no
//! colours: a [`Role`] says what a primitive is for and the theme picks the colour. All geometry is in stage
//! pixels; line widths and marker sizes are in screen points and stay constant while zooming.

use anim_core::{BezPath, Point, Rect};
use serde::{Deserialize, Serialize};

use crate::selection::ElementRef;

/// What an overlay primitive means; the theme maps it to a colour and style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Outline of a selected object.
    Selection,
    /// The result a gesture would produce if released now.
    Preview,
    /// Something under the pointer.
    Hover,
    /// A control the user can drag.
    Handle,
    /// The transformation point.
    Pivot,
    /// Construction geometry (pen rubber band, handle lines).
    Guide,
}

/// Marker shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Marker {
    Square,
    Circle,
    Diamond,
}

/// One anchor of a bezier path with its optional handles (absolute positions).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub pos: Point,
    pub control_in: Option<Point>,
    pub control_out: Option<Point>,
    pub selected: bool,
}

/// A thing drawn over the stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "overlay", rename_all = "snake_case")]
pub enum Overlay {
    Rect {
        rect: Rect,
        role: Role,
    },
    Line {
        a: Point,
        b: Point,
        role: Role,
    },
    Polyline {
        points: Vec<Point>,
        closed: bool,
        role: Role,
    },
    /// A finished bezier path; with `width` it is drawn as a stroke of that many stage pixels, otherwise as an
    /// outline (and filled when `filled`).
    Path {
        path: BezPath,
        role: Role,
        width: Option<f64>,
        filled: bool,
    },
    /// Anchors and handles of a path being edited.
    Nodes {
        nodes: Vec<Node>,
        closed: bool,
        role: Role,
    },
    Marker {
        pos: Point,
        marker: Marker,
        role: Role,
        filled: bool,
    },
    /// The rubber-band rectangle of a marquee selection.
    Marquee {
        rect: Rect,
    },
    Crosshair {
        pos: Point,
        role: Role,
    },
    /// A text caret or selection inside a text field; the host asks its text layout for the geometry.
    TextCursor {
        target: ElementRef,
        offset: usize,
        anchor: usize,
    },
}

/// Which cursor the stage should show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cursor {
    #[default]
    Default,
    /// Over a selectable object.
    Move,
    /// Over an edge that can be bent.
    Bend,
    /// Over a corner point.
    Corner,
    /// Over a path point.
    Point,
    Crosshair,
    Pen,
    PenClose,
    Pencil,
    Brush,
    Bucket,
    InkBottle,
    Eraser,
    Eyedropper,
    Text,
    Hand,
    Grabbing,
    ZoomIn,
    ZoomOut,
    /// Resize handle; the angle is where the handle sits, as a compass point.
    Resize(Compass),
    Rotate,
    Skew,
    Pivot,
}

/// The eight compass directions of resize handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compass {
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
    Nw,
}
