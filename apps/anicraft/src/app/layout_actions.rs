//! Turns the events of the layout UI (Align and Transform panels, the grid, guides and snapping dialogs, the
//! magnification items) into commands and view changes.

use anim_core::{Point, Rect, num};
use serde_json::json;

use super::AniApp;
use crate::layout_ui::{DialogKind, LayoutDialog, LayoutEvent};

impl AniApp {
    /// One event of the layout UI.
    pub(super) fn layout_event(&mut self, event: LayoutEvent) {
        match event {
            LayoutEvent::Command { id, params } => {
                self.run(id, &params);
            }
            LayoutEvent::Magnify(percent) => self.magnify(num::f64_to_f32(f64::from(percent)) / 100.0),
            LayoutEvent::ShowFrame => self.ui.stage.fit_request = true,
            LayoutEvent::ShowAll => self.show_all(),
            LayoutEvent::TogglePasteboard => {
                self.ui.arrange.pasteboard = !self.ui.arrange.pasteboard;
                let state = if self.ui.arrange.pasteboard { "shown" } else { "clipped" };
                self.status(format!("Content outside the stage is {state}"));
            }
            LayoutEvent::ToggleToStage => self.ui.arrange.to_stage = !self.ui.arrange.to_stage,
            LayoutEvent::TogglePanel(id) => self.toggle_layout_panel(id),
            LayoutEvent::Open(kind) => self.open_layout_dialog(kind),
            LayoutEvent::Close => self.ui.arrange.dialog = None,
        }
    }

    pub(super) fn open_layout_dialog(&mut self, kind: DialogKind) {
        let doc = self.host.session().doc();
        let aids = anicraft_tools::guides::read(doc.document());
        self.ui.arrange.dialog = Some(LayoutDialog::open(kind, &aids, &doc.editor.state.settings));
    }

    fn toggle_layout_panel(&mut self, id: u16) {
        let panel = anim_ui::PanelId::Custom(id);
        if self.ui.layout.contains(&panel) {
            self.ui.layout.apply_with_permissions(anim_ui::dock::docking::Action::Close { panel }, |p| p.permissions()).ok();
        } else if let Err(e) = anim_ui::dock::open_panel(&mut self.ui.layout, panel) {
            self.status(format!("cannot open {}: {e}", panel.title()));
        }
    }

    /// Zooms the stage to a magnification (1.0 is 100%) about its centre.
    fn magnify(&mut self, zoom: f32) {
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, self.stage_canvas().size());
        self.ui.stage.view = self.ui.stage.view.with_zoom_centered(canvas, zoom.clamp(0.02, 64.0));
    }

    /// View > Magnification > Show All: fits the stage together with everything on the pasteboard in the current
    /// frame.
    fn show_all(&mut self) {
        let doc = self.host.session().doc();
        let mut bounds = Rect::new(0.0, 0.0, doc.document().width, doc.document().height);
        let state = &doc.editor.state;
        if let Some(timeline) = doc.document().timeline(&state.timeline) {
            for layer in &timeline.layers {
                let Some(keyframe) = layer.keyframe_at(state.frame) else { continue };
                for element in &keyframe.elements {
                    if let Some(r) = doc.editor.with_context(|ctx| ctx.bounds_in_parent(element)) {
                        bounds = bounds.union(r);
                    }
                }
            }
        }
        let size = egui::vec2(num::f64_to_f32(bounds.width()), num::f64_to_f32(bounds.height()));
        if size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        let canvas = self.stage_canvas().size();
        let fit = anim_ui::StageView::fit(canvas, size, 24.0);
        let center = egui::vec2(num::f64_to_f32(bounds.center().x), num::f64_to_f32(bounds.center().y));
        self.ui.stage.view = anim_ui::StageView { zoom: fit.zoom, pan: canvas / 2.0 - center * fit.zoom };
        self.ui.stage.fit_request = false;
    }

    /// A right click on the stage: selects the object under the pointer when it is not selected, then opens the menu.
    pub(super) fn open_stage_menu(&mut self, at: Point) {
        let hit = self.host.session().doc().editor.with_context(|ctx| anicraft_tools::pick(ctx, at));
        let selected = hit.as_ref().is_some_and(|h| self.host.session().doc().editor.state.selection.contains(&h.element));
        if hit.is_some() && !selected {
            self.run("select.at", &json!({ "x": at.x, "y": at.y }));
        }
        self.ui.arrange.context = Some(crate::layout_ui::ContextMenu { at: None });
    }

    /// Opens the layout dialogs and panels `--show` names, so a screenshot can capture them.
    pub(super) fn layout_shot(&mut self, what: &str) {
        match what {
            "layout" => {
                // Three objects of different sizes, selected, with a guide and the grid on.
                for (x, y, w, h, fill) in [(60.0, 50.0, 70.0, 40.0, "#e53935"), (190.0, 120.0, 120.0, 90.0, "#1e88e5"), (360.0, 70.0, 50.0, 60.0, "#43a047")] {
                    self.run("draw.rectangle", &json!({ "x": x, "y": y, "width": w, "height": h, "fill": fill, "stroke": "none", "object_drawing": true }));
                }
                self.run("select.all", &json!({}));
                self.run("guides.add", &json!({ "axis": "vertical", "position": 275.0 }));
                self.run("guides.add", &json!({ "axis": "horizontal", "position": 100.0 }));
                self.run("grid.set", &json!({ "visible": true, "spacing": 25.0 }));
            }
            "align" => {
                self.run("layout.align", &json!({ "edge": "bottom" }));
                self.run("layout.space", &json!({ "axis": "horizontal" }));
            }
            "grid-dialog" => self.open_layout_dialog(DialogKind::Grid),
            "guides-dialog" => self.open_layout_dialog(DialogKind::Guides),
            "snap-dialog" => self.open_layout_dialog(DialogKind::Snapping),
            "context-menu" => self.ui.arrange.context = Some(crate::layout_ui::ContextMenu { at: Some(egui::pos2(380.0, 260.0)) }),
            "no-pasteboard" => self.ui.arrange.pasteboard = false,
            "show-all" => self.show_all(),
            _ => {}
        }
    }
}
