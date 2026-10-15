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
