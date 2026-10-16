//! What the toolbox, the tool options, the Colour panel and the Swatches panel ask for.

use anicraft_tools::swatches::{TableFormat, load_table, save_table};
use anim_scene::Element;
use anim_shape::FillStyle;
use serde_json::json;

use super::AniApp;
use crate::toolui::{ColourEdit, ToolUiEvent};

impl AniApp {
    pub(super) fn toolui_event(&mut self, event: ToolUiEvent) {
        match event {
            ToolUiEvent::Tool(id) => self.select_tool_by_id(&id),
            ToolUiEvent::Command { id, params } => {
                self.run(id, &params);
            }
            ToolUiEvent::Colour(edit) => self.colour_edit(edit),
            ToolUiEvent::SwatchLoad => self.swatches_load(),
            ToolUiEvent::SwatchSave(format) => self.swatches_save(format),
        }
    }

    /// Applies a Colour panel edit to the toolbox and the selected shapes. The edits of one drag share an undo step;
    /// with no shape selected only the toolbox changes, which is not an undo step.
    fn colour_edit(&mut self, edit: ColourEdit) {
        if edit.fill.is_some() || edit.stroke.is_some() {
            let has_shape = {
                let editor = &self.host.session().doc().editor;
                editor.with_context(|ctx| ctx.selection.items.iter().any(|r| matches!(ctx.resolve(r).map(|x| x.element), Some(Element::Shape(_)))))
            };
            let result = if has_shape {
                let key = format!("colour:{}", self.prop_gesture);
                self.host.session_mut().edit("Change Colour", Some(&key), |doc| {
                    anicraft_tools::colour::apply(&mut doc.editor, edit.fill.as_ref(), edit.stroke.as_ref(), true)
                })
            } else {
                anicraft_tools::colour::apply(&mut self.host.session_mut().doc_mut().editor, edit.fill.as_ref(), edit.stroke.as_ref(), true)
            };
            if let Err(e) = result {
                if has_shape {
                    self.host.session_mut().undo();
                }
                self.status(e);
            }
        }
        if edit.commit {
            self.prop_gesture = self.prop_gesture.saturating_add(1);
        }
    }

    fn swatches_load(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("Load Colors").add_filter("Colour tables", &["aco", "act"]).pick_file() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(format) = TableFormat::from_name(&name) else {
            self.status(format!("{name} is not a .aco or .act file"));
            return;
        };
        let loaded = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| load_table(format, &bytes).map_err(|e| e.to_string()));
        match loaded {
            Ok(fills) => {
                let swatches: Vec<&FillStyle> = fills.iter().collect();
                if self.run("swatches.set", &json!({ "swatches": swatches, "append": false })) {
                    self.status(format!("loaded {} colours from {name}", fills.len()));
                }
            }
            Err(e) => self.status(format!("{name}: {e}")),
        }
    }

    fn swatches_save(&mut self, format: TableFormat) {
        let (ext, label) = match format {
            TableFormat::Aco => ("aco", "Photoshop colour swatches"),
            TableFormat::Act => ("act", "Photoshop colour table"),
        };
        let Some(path) = rfd::FileDialog::new().set_title("Save Colors").add_filter(label, &[ext]).set_file_name(format!("swatches.{ext}")).save_file() else {
            return;
        };
        let swatches = anicraft_tools::swatches::read(self.host.session().doc().document());
        match std::fs::write(&path, save_table(format, &swatches)) {
            Ok(()) => self.status(format!("saved {}", path.display())),
            Err(e) => self.status(format!("{}: {e}", path.display())),
        }
    }
}
