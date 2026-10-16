//! The Lasso tool: select the part of the artwork inside a free-form loop, a polygon, or by colour with the wand.

use anim_core::Point;

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerEvent, PointerKind, ToolEvent};
use crate::overlay::{Cursor, Overlay, Role};
use crate::selection::Selection;
use crate::settings::LassoMode;
use crate::tool::{Tool, ToolId, ToolOutput};

/// Distance in screen points within which a click on the first corner closes a polygon.
const CLOSE_RADIUS: f64 = 6.0;
/// Free-form points closer than this many stage pixels to the previous one are dropped.
const MIN_STEP: f64 = 1.0;

/// What the current gesture is building.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Building {
    #[default]
    Nothing,
    /// Dragging a free-form loop.
    Freeform,
    /// Placing polygon corners.
    Polygon,
}

/// Selects with a loop, a polygon or the magic wand.
#[derive(Debug, Clone, Default)]
pub struct LassoTool {
    building: Building,
    points: Vec<Point>,
    hover: Option<Point>,
    additive: bool,
}

impl LassoTool {
    fn finish(&mut self) -> ToolOutput {
        let building = std::mem::take(&mut self.building);
        let points = std::mem::take(&mut self.points);
        let additive = std::mem::take(&mut self.additive);
        self.hover = None;
        if points.len() >= 3 {
            return ToolOutput::with(Cursor::Crosshair, vec![ToolCommand::LassoSelect { polygon: points, additive }]);
        }
        // A click that made no loop selects nothing.
        if building == Building::Freeform && !additive {
            return ToolOutput::with(Cursor::Crosshair, vec![ToolCommand::SetSelection { selection: Selection::default() }]);
        }
        ToolOutput::cursor(Cursor::Crosshair)
    }

    fn down(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let mode = ctx.settings.lasso.mode;
        if mode == LassoMode::MagicWand {
            let command = ToolCommand::WandSelect { point: p.pos, threshold: ctx.settings.lasso.threshold, additive: p.modifiers.shift };
            return ToolOutput::with(Cursor::Crosshair, vec![command]);
        }
        let polygon = mode == LassoMode::Polygon || p.modifiers.alt || self.building == Building::Polygon;
        if polygon {
            if self.building == Building::Polygon {
                let near_first = self.points.len() >= 3 && self.points.first().is_some_and(|f| f.distance(p.pos) <= CLOSE_RADIUS / ctx.zoom.max(1.0e-6));
                if p.click_count >= 2 || near_first {
                    return self.finish();
                }
            } else {
                self.building = Building::Polygon;
                self.additive = p.modifiers.shift;
            }
            self.points.push(p.pos);
            return ToolOutput::cursor(Cursor::Crosshair);
        }
        self.building = Building::Freeform;
        self.additive = p.modifiers.shift;
        self.points = vec![p.pos];
        ToolOutput::cursor(Cursor::Crosshair)
    }
}

impl Tool for LassoTool {
    fn id(&self) -> ToolId {
        ToolId::Lasso
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => self.down(p, ctx),
                PointerKind::Move => {
                    if self.building == Building::Freeform && self.points.last().is_none_or(|l| l.distance(p.pos) >= MIN_STEP) {
                        self.points.push(p.pos);
                    }
                    self.hover = Some(p.pos);
                    ToolOutput::cursor(Cursor::Crosshair)
                }
                PointerKind::Up => {
                    if self.building == Building::Freeform {
                        return self.finish();
                    }
                    ToolOutput::cursor(Cursor::Crosshair)
                }
                PointerKind::Hover => {
                    self.hover = Some(p.pos);
                    ToolOutput::cursor(Cursor::Crosshair)
                }
            },
            ToolEvent::Key(k) if k.pressed => match k.key {
                Key::Enter if self.building == Building::Polygon => self.finish(),
                Key::Escape => {
                    self.cancel();
                    ToolOutput::cursor(Cursor::Crosshair)
                }
                Key::Backspace | Key::Delete if self.building == Building::Polygon => {
                    self.points.pop();
                    ToolOutput::cursor(Cursor::Crosshair)
                }
                _ => ToolOutput::cursor(Cursor::Crosshair),
            },
            ToolEvent::Cancel => {
                self.cancel();
                ToolOutput::cursor(Cursor::Crosshair)
            }
            _ => ToolOutput::cursor(Cursor::Crosshair),
        }
    }

    fn overlay(&self, _ctx: &ToolContext<'_>) -> Vec<Overlay> {
        match self.building {
            Building::Nothing => Vec::new(),
            Building::Freeform => vec![Overlay::Polyline { points: self.points.clone(), closed: true, role: Role::Preview }],
            Building::Polygon => {
                let mut points = self.points.clone();
                points.extend(self.hover);
                vec![Overlay::Polyline { points, closed: false, role: Role::Preview }]
            }
        }
    }

    fn cancel(&mut self) {
        self.building = Building::Nothing;
        self.points.clear();
        self.hover = None;
        self.additive = false;
    }

    fn is_busy(&self) -> bool {
        self.building != Building::Nothing
    }

    fn captures_keys(&self) -> bool {
        self.building == Building::Polygon
    }
}
