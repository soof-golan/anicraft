//! Widgets of the document features: the tab strip, the History, Output and Compiler Errors panels. Each takes a view
//! model and returns what happened; none changes the document.

use egui::{Color32, RichText, Ui};

use crate::docs::{DocAction, DocsUi, Level, TabEvent, TabInfo};

/// The strip of open documents under the menu bar.
pub fn tab_strip(ui: &mut Ui, tabs: &[TabInfo]) -> Vec<TabEvent> {
    let mut out = Vec::new();
    egui::ScrollArea::horizontal().id_salt("doc-tabs").auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal(|ui| {
            for tab in tabs {
                let title = format!("{}{}", tab.title, if tab.dirty { " *" } else { "" });
                let text = if tab.active { RichText::new(crate::text(&title)).strong() } else { RichText::new(crate::text(&title)) };
                let response = ui.selectable_label(tab.active, text).on_hover_text(crate::text(&tab.tooltip));
                if response.clicked() {
                    out.push(TabEvent::Select(tab.id));
                }
                if response.middle_clicked() {
                    out.push(TabEvent::Close(tab.id));
                }
                if ui.small_button("x").on_hover_text("Close").clicked() {
                    out.push(TabEvent::Close(tab.id));
                }
                ui.add_space(6.0);
            }
            if ui.small_button("+").on_hover_text("New document").clicked() {
                out.push(TabEvent::New);
            }
        });
    });
    out
}

/// What the History panel draws: step labels from the oldest, how many are applied, and which can be replayed.
#[derive(Debug, Clone, Default)]
pub struct HistoryView {
    pub labels: Vec<String>,
    pub applied: usize,
    pub replayable: Vec<bool>,
}

/// The History panel: every undo step, the current one marked, steps that were undone dimmed.
pub fn history_panel(ui: &mut Ui, view: &HistoryView, state: &mut DocsUi) -> Vec<DocAction> {
    let mut out = Vec::new();
    let can_replay = state.history_selected.is_some_and(|(a, b)| (a..=b).any(|i| view.replayable.get(i).copied().unwrap_or(false)));
    ui.horizontal(|ui| {
        if ui.add_enabled(can_replay, egui::Button::new("Replay Steps")).on_hover_text("Run the selected commands again on the document as it is now").clicked()
            && let Some((from, to)) = state.history_selected
        {
            out.push(DocAction::HistoryReplay { from, to });
        }
        if ui
            .add_enabled(can_replay, egui::Button::new("Save as Command..."))
            .on_hover_text("Keep the selected steps as a command in the Commands menu")
            .clicked()
        {
            state.save_command_open = true;
        }
    });
    if state.save_command_open {
        ui.horizontal(|ui| {
            ui.label("Name");
            let response = ui.text_edit_singleline(&mut state.save_command_name);
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Save").clicked() || enter)
                && let Some((from, to)) = state.history_selected
            {
                out.push(DocAction::SaveSteps { from, to, name: state.save_command_name.clone() });
                state.save_command_open = false;
            }
            if ui.button("Cancel").clicked() {
                state.save_command_open = false;
            }
        });
    }
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let row = |ui: &mut Ui, text: &str, selected: bool, current: bool, dim: bool| {
            let mut rich = RichText::new(crate::text(text));
            if dim {
                rich = rich.weak();
            }
            if current {
                rich = rich.strong();
            }
            ui.selectable_label(selected, rich)
        };
        // Row 0 is the document as it was opened; row n is the state after step n.
        if row(ui, "Original document", false, view.applied == 0, false).clicked() {
            out.push(DocAction::HistoryJump(0));
            state.history_selected = None;
        }
        for (i, label) in view.labels.iter().enumerate() {
            let selected = state.history_selected.is_some_and(|(a, b)| (a..=b).contains(&i));
            let applied = i < view.applied;
            let replay_mark = if view.replayable.get(i).copied().unwrap_or(false) { "" } else { "  (gesture)" };
            let response = row(ui, &format!("{}. {label}{replay_mark}", i.saturating_add(1)), selected, i.saturating_add(1) == view.applied, !applied);
            if response.clicked() {
                let extend = ui.input(|input| input.modifiers.shift);
                state.history_selected = match (extend, state.history_selected) {
                    (true, Some((a, _))) => Some((a.min(i), a.max(i))),
                    _ => Some((i, i)),
                };
                out.push(DocAction::HistoryJump(i.saturating_add(1)));
            }
        }
    });
    out
}

fn level_colour(level: Level, ui: &Ui) -> Color32 {
    match level {
        Level::Info => ui.visuals().text_color(),
        Level::Warning => Color32::from_rgb(0xd0, 0x90, 0x20),
        Level::Error => Color32::from_rgb(0xe0, 0x50, 0x50),
    }
}

/// The Output panel: messages from opening, saving, exporting and autosaving.
pub fn output_panel(ui: &mut Ui, state: &mut DocsUi) -> Vec<DocAction> {
    ui.horizontal(|ui| {
        if ui.button("Clear").clicked() {
            state.output.clear();
        }
        if ui.button("Copy").clicked() {
            let text: Vec<&str> = state.output.iter().map(|l| l.text.as_str()).collect();
            ui.ctx().copy_text(text.join("\n"));
        }
        ui.separator();
        ui.label("Show");
        let shown = match state.output_min {
            None | Some(Level::Info) => "Everything",
            Some(Level::Warning) => "Warnings and errors",
            Some(Level::Error) => "Errors only",
        };
        egui::ComboBox::from_id_salt("output-filter").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut state.output_min, None, "Everything");
            ui.selectable_value(&mut state.output_min, Some(Level::Warning), "Warnings and errors");
            ui.selectable_value(&mut state.output_min, Some(Level::Error), "Errors only");
        });
    });
    ui.separator();
    let min = state.output_min.unwrap_or(Level::Info);
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        let mut any = false;
        for line in state.output.iter().filter(|l| l.level >= min) {
            any = true;
            ui.label(RichText::new(crate::text(&line.text)).monospace().color(level_colour(line.level, ui)));
        }
        if !any {
            ui.label(RichText::new("Nothing has been reported yet.").weak());
        }
    });
    Vec::new()
}

/// The Compiler Errors panel: problems in frame scripts and in the document's structure.
pub fn errors_panel(ui: &mut Ui, state: &DocsUi) -> Vec<DocAction> {
    let mut out = Vec::new();
    let errors = state.problems.iter().filter(|p| p.level == Level::Error).count();
    let warnings = state.problems.len().saturating_sub(errors);
    ui.label(RichText::new(format!("{errors} error(s), {warnings} warning(s)")).strong());
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if state.problems.is_empty() {
            ui.label(RichText::new("No problems in the document or in its frame scripts.").weak());
        }
        for (i, problem) in state.problems.iter().enumerate() {
            let tag = if problem.level == Level::Error { "Error" } else { "Warning" };
            let text = if problem.place.is_empty() { format!("{tag}: {}", problem.message) } else { format!("{tag}: {} ({})", problem.message, problem.place) };
            if ui
                .add(egui::Button::new(RichText::new(crate::text(&text)).color(level_colour(problem.level, ui))).frame(false))
                .on_hover_text("Go to this place")
                .clicked()
            {
                out.push(DocAction::GotoProblem(i));
            }
        }
    });
    out
}
