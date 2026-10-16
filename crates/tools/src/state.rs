//! Editor state that is not part of the document: selection, tool options, the editing position and the stage view.

use anim_core::{Point, Vec2};
use anim_scene::TimelineId;
use serde::{Deserialize, Serialize};

use crate::selection::{ElementRef, Selection};
use crate::settings::ToolSettings;
use crate::tool::ToolId;

/// Smallest and largest zoom the view allows.
pub const MIN_ZOOM: f64 = 0.02;
pub const MAX_ZOOM: f64 = 256.0;

/// The stage view: `screen = stage * zoom + offset` (points).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub zoom: f64,
    pub offset: Vec2,
    /// Size of the stage widget in points, used to fit rectangles.
    pub viewport: Vec2,
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: 1.0, offset: Vec2::ZERO, viewport: Vec2::new(1000.0, 700.0) }
    }
}

impl View {
    /// Screen position of a stage point.
    #[must_use]
    pub const fn to_screen(&self, p: Point) -> Point {
        Point::new(p.x.mul_add(self.zoom, self.offset.x), p.y.mul_add(self.zoom, self.offset.y))
    }

    /// Stage position of a screen point.
    #[must_use]
    pub fn to_stage(&self, p: Point) -> Point {
        Point::new((p.x - self.offset.x) / self.zoom, (p.y - self.offset.y) / self.zoom)
    }

    /// Zooms by `factor` keeping the stage point `center` where it is on screen.
    pub fn zoom_at(&mut self, center: Point, factor: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let screen = self.to_screen(center);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.offset = Vec2::new(center.x.mul_add(-self.zoom, screen.x), center.y.mul_add(-self.zoom, screen.y));
    }

    /// Zooms so that `rect` fills the viewport, centred.
    pub fn zoom_to_rect(&mut self, rect: anim_core::Rect) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        self.zoom = (self.viewport.x / rect.width()).min(self.viewport.y / rect.height()).clamp(MIN_ZOOM, MAX_ZOOM);
        let c = rect.center();
        self.offset = Vec2::new(c.x.mul_add(-self.zoom, self.viewport.x / 2.0), c.y.mul_add(-self.zoom, self.viewport.y / 2.0));
    }
}

/// Everything about the editing session except the document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorState {
    pub selection: Selection,
    pub settings: ToolSettings,
    /// The timeline being edited.
    pub timeline: TimelineId,
    /// Timelines to return to when leaving a symbol, with the layer and frame to restore.
    pub timeline_stack: Vec<(TimelineId, usize, u32)>,
    pub layer: usize,
    pub frame: u32,
    pub view: View,
    pub active_tool: ToolId,
    /// The text field being edited.
    pub text_editing: Option<ElementRef>,
    /// Edit Multiple Frames: the frames (first and last, 0-based, inclusive) whose keyframes the tools can pick and
    /// edit together. `None`: only the keyframes under the playhead.
    #[serde(default)]
    pub edit_range: Option<(u32, u32)>,
    /// Auto keyframe: changing a held frame (one inside a span) first makes it a keyframe, so the change does not
    /// reach the rest of the span. Off by default for scripted use; the window turns it on.
    #[serde(default)]
    pub auto_keyframe: bool,
    /// Edit in Place: the symbol being edited is shown in the context of the timeline it was entered from, with this
    /// matrix (symbol space to that timeline's stage space) and the rest of the artwork dimmed. `None`: the symbol
    /// is edited on its own stage.
    #[serde(default)]
    pub in_place: Option<anim_core::Affine>,
    /// The `in_place` of each timeline in `timeline_stack`, to restore when leaving the symbol.
    #[serde(default)]
    pub in_place_stack: Vec<Option<anim_core::Affine>>,
}

impl EditorState {
    /// A fresh session editing the first scene.
    #[must_use]
    pub fn new() -> Self {
        Self {
            selection: Selection::empty(),
            settings: ToolSettings::default(),
            timeline: TimelineId::Scene(0),
            timeline_stack: Vec::new(),
            layer: 0,
            frame: 0,
            view: View::default(),
            active_tool: ToolId::Selection,
            text_editing: None,
            edit_range: None,
            auto_keyframe: false,
            in_place: None,
            in_place_stack: Vec::new(),
        }
    }

    /// Enters the symbol `name` from the current timeline. With `in_place` the symbol stays in the context of the
    /// timeline it was entered from, positioned by `to_parent` (symbol space to the current timeline's stage space);
    /// without it the symbol gets a stage of its own.
    pub fn enter_symbol(&mut self, name: &str, in_place: Option<anim_core::Affine>) {
        self.timeline_stack.push((self.timeline.clone(), self.layer, self.frame));
        self.in_place_stack.push(self.in_place);
        self.in_place = in_place.map(|m| self.in_place.unwrap_or(anim_core::Affine::IDENTITY) * m);
        self.timeline = TimelineId::Symbol(name.to_owned());
        self.layer = 0;
        self.frame = 0;
        self.selection = Selection::empty();
        self.edit_range = None;
    }

    /// Leaves the symbol being edited for the timeline it was entered from. False when there is none.
    pub fn leave_symbol(&mut self) -> bool {
        let Some((timeline, layer, frame)) = self.timeline_stack.pop() else { return false };
        self.timeline = timeline;
        self.layer = layer;
        self.frame = frame;
        self.in_place = self.in_place_stack.pop().flatten();
        self.selection = Selection::empty();
        self.edit_range = None;
        true
    }

    /// Jumps straight to `depth` levels below the first timeline of the stack (the breadcrumb), leaving the symbols
    /// above that level. `depth` 0 is the outermost timeline.
    pub fn leave_to_depth(&mut self, depth: usize) {
        while self.timeline_stack.len() > depth {
            if !self.leave_symbol() {
                break;
            }
        }
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_keeps_the_anchor_fixed_on_screen() {
        let mut v = View::default();
        let anchor = Point::new(120.0, 80.0);
        let before = v.to_screen(anchor);
        v.zoom_at(anchor, 2.0);
        assert!((v.zoom - 2.0).abs() < 1.0e-12);
        let after = v.to_screen(anchor);
        assert!(before.distance(after) < 1.0e-9);
        assert!(v.to_stage(after).distance(anchor) < 1.0e-9);
    }

    #[test]
    fn zoom_to_rect_centres_the_rectangle() {
        let mut v = View::default();
        v.zoom_to_rect(anim_core::Rect::new(0.0, 0.0, 500.0, 350.0));
        assert!((v.zoom - 2.0).abs() < 1.0e-12);
        let c = v.to_screen(Point::new(250.0, 175.0));
        assert!(c.distance(Point::new(500.0, 350.0)) < 1.0e-9);
    }
}
