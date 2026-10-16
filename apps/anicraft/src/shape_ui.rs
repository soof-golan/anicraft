//! The dialogs of the shape commands: Optimize Curves, Expand Fill, Soften Fill Edges, Scale and Rotate, Trace Bitmap
//! and Paste Special.
//!
//! A dialog holds only plain numbers and flags (so it can be compared and cloned with the rest of the window state);
//! [`ShapeDialog::command`] turns the answer into the registered command it stands for, and the application runs it
//! like any menu item, so the dialogs add nothing the MCP interface cannot do.

use egui::Ui;
use serde_json::{Value, json};

/// A dialog of a shape command, with the values being edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeDialog {
    /// Modify > Shape > Optimize: smoothing strength 0 to 100 and the number of passes.
    Optimize { amount: u32, passes: u32 },
    /// Modify > Shape > Expand Fill: distance in pixels and the direction.
    ExpandFill { distance: u32, inset: bool },
    /// Modify > Shape > Soften Fill Edges: distance, number of steps and the direction.
    SoftenEdges { distance: u32, steps: u32, inset: bool },
    /// Modify > Transform > Scale and Rotate: percent and degrees.
    ScaleRotate { scale: u32, degrees: i32 },
    /// Modify > Bitmap > Trace Bitmap.
    TraceBitmap { threshold: u32, min_area: u32, curve: usize, corner: usize },
    /// Edit > Paste Special: what to paste as and where.
    PasteSpecial { as_bitmap: bool, position: usize },
}

/// Curve fit choices of Trace Bitmap, as the command names them.
const CURVE_FIT: [(&str, &str); 6] =
    [("pixels", "Pixels"), ("very_tight", "Very Tight"), ("tight", "Tight"), ("normal", "Normal"), ("smooth", "Smooth"), ("very_smooth", "Very Smooth")];
/// Corner threshold choices of Trace Bitmap.
const CORNER: [(&str, &str); 3] = [("many", "Many corners"), ("normal", "Normal"), ("few", "Few corners")];
/// Where Paste Special puts the objects.
const POSITIONS: [(&str, &str); 3] = [("offset", "Slightly offset"), ("in_place", "In place"), ("center", "Centre of the stage")];

impl ShapeDialog {
    /// Optimize Curves with Animate's usual strength.
    #[must_use]
    pub const fn optimize() -> Self {
        Self::Optimize { amount: 50, passes: 1 }
    }

    /// Expand Fill with Animate's defaults (4 px, expand).
    #[must_use]
    pub const fn expand_fill() -> Self {
        Self::ExpandFill { distance: 4, inset: false }
    }

    /// Soften Fill Edges with Animate's defaults (10 px, 5 steps, expand).
    #[must_use]
    pub const fn soften_edges() -> Self {
        Self::SoftenEdges { distance: 10, steps: 5, inset: false }
    }

    /// Scale and Rotate at 100 percent and no turn.
    #[must_use]
    pub const fn scale_rotate() -> Self {
        Self::ScaleRotate { scale: 100, degrees: 0 }
    }

    /// Trace Bitmap with Animate's defaults (threshold 100, minimum area 8, normal curves and corners).
    #[must_use]
    pub const fn trace_bitmap() -> Self {
        Self::TraceBitmap { threshold: 100, min_area: 8, curve: 3, corner: 1 }
    }

    /// Paste Special with the plain objects slightly offset.
    #[must_use]
    pub const fn paste_special() -> Self {
        Self::PasteSpecial { as_bitmap: false, position: 0 }
    }

    /// The window title.
    #[must_use]
    pub const fn title(&self) -> &'static str {
        match self {
            Self::Optimize { .. } => "Optimize Curves",
            Self::ExpandFill { .. } => "Expand Fill",
            Self::SoftenEdges { .. } => "Soften Fill Edges",
            Self::ScaleRotate { .. } => "Scale and Rotate",
            Self::TraceBitmap { .. } => "Trace Bitmap",
            Self::PasteSpecial { .. } => "Paste Special",
        }
    }

    /// The fields of the dialog.
    pub fn ui(&mut self, ui: &mut Ui) {
        match self {
            Self::Optimize { amount, passes } => {
                egui::Grid::new("optimize-curves").num_columns(2).show(ui, |ui| {
                    ui.label("Smoothing");
                    ui.add(egui::Slider::new(amount, 0..=100).suffix(" %"));
                    ui.end_row();
                    ui.label("Passes");
                    ui.add(egui::DragValue::new(passes).range(1..=10));
                    ui.end_row();
                });
                ui.label("Fewer anchor points make a smaller file and smoother curves.");
            }
            Self::ExpandFill { distance, inset } => {
                egui::Grid::new("expand-fill").num_columns(2).show(ui, |ui| {
                    ui.label("Distance");
                    ui.add(egui::DragValue::new(distance).range(1..=1000).suffix(" px"));
                    ui.end_row();
                    ui.label("Direction");
                    ui.horizontal(|ui| {
                        ui.radio_value(inset, false, "Expand");
                        ui.radio_value(inset, true, "Inset");
                    });
                    ui.end_row();
                });
            }
            Self::SoftenEdges { distance, steps, inset } => {
                egui::Grid::new("soften-edges").num_columns(2).show(ui, |ui| {
                    ui.label("Distance");
                    ui.add(egui::DragValue::new(distance).range(1..=1000).suffix(" px"));
                    ui.end_row();
                    ui.label("Number of steps");
                    ui.add(egui::DragValue::new(steps).range(1..=64));
                    ui.end_row();
                    ui.label("Direction");
                    ui.horizontal(|ui| {
                        ui.radio_value(inset, false, "Expand");
                        ui.radio_value(inset, true, "Inset");
                    });
                    ui.end_row();
                });
            }
            Self::ScaleRotate { scale, degrees } => {
                egui::Grid::new("scale-rotate").num_columns(2).show(ui, |ui| {
                    ui.label("Scale");
                    ui.add(egui::DragValue::new(scale).range(1..=10_000).suffix(" %"));
                    ui.end_row();
                    ui.label("Rotate");
                    ui.add(egui::DragValue::new(degrees).range(-360..=360).suffix(" degrees"));
                    ui.end_row();
                });
            }
            Self::TraceBitmap { threshold, min_area, curve, corner } => {
                egui::Grid::new("trace-bitmap").num_columns(2).show(ui, |ui| {
                    ui.label("Color threshold");
                    ui.add(egui::DragValue::new(threshold).range(1..=500));
                    ui.end_row();
                    ui.label("Minimum area");
                    ui.add(egui::DragValue::new(min_area).range(1..=1000).suffix(" pixels"));
                    ui.end_row();
                    ui.label("Curve fit");
                    choice(ui, "trace-curve", curve, &CURVE_FIT);
                    ui.end_row();
                    ui.label("Corner threshold");
                    choice(ui, "trace-corner", corner, &CORNER);
                    ui.end_row();
                });
            }
            Self::PasteSpecial { as_bitmap, position } => {
                egui::Grid::new("paste-special").num_columns(2).show(ui, |ui| {
                    ui.label("Paste as");
                    ui.horizontal(|ui| {
                        ui.radio_value(as_bitmap, false, "Objects");
                        ui.radio_value(as_bitmap, true, "Bitmap");
                    });
                    ui.end_row();
                    ui.label("Position");
                    choice(ui, "paste-position", position, &POSITIONS);
                    ui.end_row();
                });
            }
        }
    }

    /// The command and parameters this answer stands for. Paste Special has none: the clipboard is the window's.
    #[must_use]
    pub fn command(&self) -> Option<(&'static str, Value)> {
        match *self {
            Self::Optimize { amount, passes } => Some(("modify.shape_curves", json!({ "op": "optimize", "amount": amount, "passes": passes }))),
            Self::ExpandFill { distance, inset } => Some(("modify.expand_fill", json!({ "distance": distance, "inset": inset }))),
            Self::SoftenEdges { distance, steps, inset } => Some(("modify.soften_fill_edges", json!({ "distance": distance, "steps": steps, "inset": inset }))),
            Self::ScaleRotate { scale, degrees } => Some(("modify.transform", json!({ "op": "scale_rotate", "scale": scale, "degrees": degrees }))),
            Self::TraceBitmap { threshold, min_area, curve, corner } => Some((
                "modify.trace_bitmap",
                json!({
                    "color_threshold": threshold,
                    "minimum_area": min_area,
                    "curve_fit": CURVE_FIT.get(curve).map_or("normal", |c| c.0),
                    "corner_threshold": CORNER.get(corner).map_or("normal", |c| c.0),
                }),
            )),
            Self::PasteSpecial { .. } => None,
        }
    }

    /// For Paste Special: the position name and whether to paste a bitmap.
    #[must_use]
    pub fn paste_choice(&self) -> Option<(&'static str, bool)> {
        match *self {
            Self::PasteSpecial { as_bitmap, position } => Some((POSITIONS.get(position).map_or("offset", |p| p.0), as_bitmap)),
            _ => None,
        }
    }
}

/// A drop-down over `(name, label)` pairs, editing the index.
fn choice(ui: &mut Ui, id: &str, index: &mut usize, options: &[(&str, &str)]) {
    let current = options.get(*index).map_or("", |o| o.1);
    egui::ComboBox::from_id_salt(id).selected_text(current).show_ui(ui, |ui| {
        for (i, (_, label)) in options.iter().enumerate() {
            ui.selectable_value(index, i, *label);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dialog_names_a_command_with_valid_parameters() {
        for dialog in
            [ShapeDialog::optimize(), ShapeDialog::expand_fill(), ShapeDialog::soften_edges(), ShapeDialog::scale_rotate(), ShapeDialog::trace_bitmap()]
        {
            let (id, params) = dialog.command().expect("a command");
            assert!(id.starts_with("modify."), "{id}");
            assert!(params.is_object());
        }
        assert!(ShapeDialog::paste_special().command().is_none());
        assert_eq!(ShapeDialog::paste_special().paste_choice(), Some(("offset", false)));
    }

    #[test]
    fn trace_choices_map_to_the_names_the_command_takes() {
        let (_, params) = ShapeDialog::TraceBitmap { threshold: 50, min_area: 4, curve: 0, corner: 2 }.command().expect("a command");
        assert_eq!(params["curve_fit"], "pixels");
        assert_eq!(params["corner_threshold"], "few");
        assert_eq!(params["color_threshold"], 50);
    }
}
