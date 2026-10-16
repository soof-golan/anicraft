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

/// What the Actions panel shows: the keyframe whose script it edits.
#[derive(Debug, Clone, Default)]
pub struct ActionsView {
    /// The layer name, the layer, the playhead (0-based) and the keyframe that holds the script; `None` when the
    /// current layer has no keyframe at or before the playhead.
    pub target: Option<ActionsTarget>,
    /// Whether a test movie is running.
    pub testing: bool,
}

/// The keyframe the Actions panel is on.
#[derive(Debug, Clone)]
pub struct ActionsTarget {
    pub layer_name: String,
    pub layer: usize,
    /// 0-based index of the keyframe.
    pub keyframe: u32,
    /// The script stored on the document.
    pub script: String,
}

/// The colour of a token kind in the script editor.
fn token_colour(kind: anim_script::TokenKind, dark: bool) -> Color32 {
    use anim_script::TokenKind as K;
    match (kind, dark) {
        (K::Keyword, true) => Color32::from_rgb(0xc5, 0x86, 0xc0),
        (K::Keyword, false) => Color32::from_rgb(0xaf, 0x00, 0xdb),
        (K::Builtin, true) => Color32::from_rgb(0x4e, 0xc9, 0xb0),
        (K::Builtin, false) => Color32::from_rgb(0x26, 0x7f, 0x99),
        (K::Number, true) => Color32::from_rgb(0xb5, 0xce, 0xa8),
        (K::Number, false) => Color32::from_rgb(0x09, 0x86, 0x58),
        (K::String, true) => Color32::from_rgb(0xce, 0x91, 0x78),
        (K::String, false) => Color32::from_rgb(0xa3, 0x15, 0x15),
        (K::Comment, true) => Color32::from_rgb(0x6a, 0x99, 0x55),
        (K::Comment, false) => Color32::from_rgb(0x00, 0x80, 0x00),
        (K::Error, _) => Color32::from_rgb(0xf4, 0x47, 0x47),
        (K::Whitespace | K::Ident | K::Punct, true) => Color32::from_rgb(0xd4, 0xd4, 0xd4),
        (K::Whitespace | K::Ident | K::Punct, false) => Color32::from_rgb(0x1f, 0x1f, 0x1f),
    }
}

/// The text of a script as a syntax highlighted layout job.
#[must_use]
pub fn highlight(text: &str, font: egui::FontId, dark: bool, wrap: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for token in anim_script::tokenize(text) {
        job.append(token.text(text), 0.0, egui::TextFormat::simple(font.clone(), token_colour(token.kind, dark)));
    }
    job.wrap.max_width = wrap;
    job
}

/// The Actions panel: the script of the selected keyframe, with syntax highlighting.
///
/// Edits are stored on the document when the editor loses focus, so one typing session is one undo step.
pub fn actions_panel(ui: &mut Ui, view: &ActionsView, state: &mut DocsUi) -> Vec<DocAction> {
    let mut out = Vec::new();
    let Some(target) = &view.target else {
        ui.label(RichText::new("Select a layer with a keyframe to write its script.").weak());
        return out;
    };
    let key = (target.layer, target.keyframe);
    let editor = &mut state.actions;
    if editor.key != Some(key) || (!editor.dirty && editor.text != target.script) {
        editor.text.clone_from(&target.script);
        editor.key = Some(key);
        editor.dirty = false;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(crate::text(&format!("{} : frame {}", target.layer_name, target.keyframe.saturating_add(1)))).strong());
        let label = if view.testing { "End Test Movie" } else { "Test Movie" };
        if ui.button(label).on_hover_text("Run the frame scripts in the preview runtime (Ctrl+Enter)").clicked() {
            out.push(DocAction::TestMovie);
        }
    });
    ui.separator();
    let dark = ui.visuals().dark_mode;
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let diagnostics = if editor.text.trim().is_empty() { Vec::new() } else { anim_script::parse(&editor.text).diagnostics };
    let syntax = crate::docs::script_syntax_error(&editor.text);
    let notes = f32::from(u8::try_from(diagnostics.len().min(4)).unwrap_or(4));
    let height = notes.mul_add(-22.0, ui.available_height() - 24.0).max(60.0);
    egui::ScrollArea::vertical().id_salt("actions-editor").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            let job = highlight(text.as_str(), font.clone(), dark, wrap);
            ui.ctx().fonts_mut(|f| f.layout_job(job))
        };
        let response = ui.add_sized(
            [ui.available_width(), ui.available_height().max(height)],
            egui::TextEdit::multiline(&mut editor.text).code_editor().hint_text("// ActionScript for this keyframe").lock_focus(true).layouter(&mut layouter),
        );
        if response.changed() {
            editor.dirty = true;
        }
        if response.lost_focus() && editor.dirty {
            editor.dirty = false;
            out.push(DocAction::SetScript { layer: target.layer, frame: target.keyframe, script: editor.text.clone() });
        }
    });
    if let Some(message) = syntax {
        ui.label(RichText::new(crate::text(&message)).color(level_colour(Level::Error, ui)));
    }
    for d in diagnostics.iter().take(4) {
        ui.label(RichText::new(crate::text(&format!("line {}: {}", d.line, d.message))).color(level_colour(Level::Warning, ui)));
    }
    out
}
