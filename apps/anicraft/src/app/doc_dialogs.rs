//! The dialogs of the document features: New Document, Save as Template, Save Changes, Revert, Recover, Document
//! Settings and the Find and Replace window.

use anicraft_cli::mcp::docfile::{self, Anchor, CATEGORIES, DocType, Units};
use anicraft_cli::mcp::find::{Kind, Scope, TextIn};
use egui::{Context, RichText, Ui, vec2};

use super::AniApp;
use crate::docs::{BUILTIN_CATEGORY_ORDER, DocDialog, NewDocState, NewTab, SettingsState, TemplateChoice};

fn size_row(ui: &mut Ui, state: &mut NewDocState) {
    let units = state.units;
    let mut w = units.from_pixels(state.width);
    let mut h = units.from_pixels(state.height);
    let (max, speed) = (units.from_pixels(docfile::MAX_STAGE), units.from_pixels(1.0));
    ui.label("Width");
    if ui.add(egui::DragValue::new(&mut w).speed(speed).range(speed..=max).suffix(format!(" {}", units.suffix()))).changed() {
        state.width = units.to_pixels(w);
        state.preset = None;
    }
    ui.end_row();
    ui.label("Height");
    if ui.add(egui::DragValue::new(&mut h).speed(speed).range(speed..=max).suffix(format!(" {}", units.suffix()))).changed() {
        state.height = units.to_pixels(h);
        state.preset = None;
    }
    ui.end_row();
}

fn units_combo(ui: &mut Ui, id: &str, units: &mut Units) {
    egui::ComboBox::from_id_salt(id).selected_text(units.label()).show_ui(ui, |ui| {
        for u in Units::ALL {
            ui.selectable_value(units, u, u.label());
        }
    });
}

fn type_combo(ui: &mut Ui, id: &str, kind: &mut DocType) {
    egui::ComboBox::from_id_salt(id).width(190.0).selected_text(kind.label()).show_ui(ui, |ui| {
        for t in DocType::ALL {
            ui.selectable_value(kind, t, t.label());
        }
    });
}

/// The New Document dialog's contents; true when the person pressed Create.
fn new_document_ui(ui: &mut Ui, state: &mut NewDocState, user: &[crate::templates::UserTemplate], deleted: &mut Option<std::path::PathBuf>) -> (bool, bool) {
    let (mut create, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, NewTab::Presets, RichText::new("Presets").strong());
        ui.selectable_value(&mut state.tab, NewTab::Templates, RichText::new("Templates").strong());
    });
    ui.separator();
    ui.horizontal_top(|ui| {
        ui.allocate_ui(vec2(520.0, 360.0), |ui| {
            ui.set_min_size(vec2(520.0, 360.0));
            match state.tab {
                NewTab::Presets => {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_min_width(150.0);
                            for (i, category) in CATEGORIES.iter().enumerate() {
                                if ui.selectable_label(state.category == i, category.name).clicked() {
                                    state.category = i;
                                    if let Some(first) = category.presets.first() {
                                        state.choose(i, first.id);
                                    }
                                }
                            }
                        });
                        ui.separator();
                        egui::ScrollArea::vertical().id_salt("presets").max_height(340.0).show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_min_width(330.0);
                                if let Some(category) = CATEGORIES.get(state.category) {
                                    for preset in category.presets {
                                        let label = format!("{}\n{} x {} px, {} fps", preset.name, preset.width, preset.height, preset.fps);
                                        let selected = state.preset == Some(preset.id);
                                        if ui.add_sized([320.0, 40.0], egui::Button::selectable(selected, label)).clicked() {
                                            state.choose(state.category, preset.id);
                                        }
                                    }
                                }
                            });
                        });
                    });
                }
                NewTab::Templates => {
                    egui::ScrollArea::vertical().id_salt("templates").max_height(340.0).show(ui, |ui| {
                        for category in BUILTIN_CATEGORY_ORDER {
                            let items: Vec<_> = docfile::BUILTIN_TEMPLATES.iter().filter(|t| t.category == category).collect();
                            if items.is_empty() {
                                continue;
                            }
                            ui.label(RichText::new(category).strong());
                            for t in items {
                                let choice = TemplateChoice::Builtin(t.id);
                                if ui.selectable_label(state.template.as_ref() == Some(&choice), t.name).on_hover_text(t.description).clicked() {
                                    state.template = Some(choice);
                                }
                            }
                        }
                        let mut last = "";
                        for t in user {
                            if t.category != last {
                                ui.add_space(4.0);
                                ui.label(RichText::new(crate::text(&t.category)).strong());
                                last = &t.category;
                            }
                            ui.horizontal(|ui| {
                                let choice = TemplateChoice::User(t.path.clone());
                                if ui
                                    .selectable_label(state.template.as_ref() == Some(&choice), crate::text(&t.name))
                                    .on_hover_text(t.path.display().to_string())
                                    .clicked()
                                {
                                    state.template = Some(choice);
                                }
                                if ui.small_button("Delete").clicked() {
                                    *deleted = Some(t.path.clone());
                                }
                            });
                        }
                        if user.is_empty() {
                            ui.add_space(4.0);
                            ui.label(RichText::new("Your own templates appear here after File > Save as Template.").weak());
                        }
                    });
                }
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.set_min_width(260.0);
            ui.label(RichText::new("Details").strong());
            match state.tab {
                NewTab::Presets => {
                    egui::Grid::new("new-doc-details").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("Platform");
                        type_combo(ui, "new-type", &mut state.kind);
                        ui.end_row();
                        size_row(ui, state);
                        ui.label("Units");
                        units_combo(ui, "new-units", &mut state.units);
                        ui.end_row();
                        ui.label("Frame rate");
                        if ui.add(egui::DragValue::new(&mut state.fps).range(0.01..=1000.0).suffix(" fps")).changed() {
                            state.preset = None;
                        }
                        ui.end_row();
                        ui.label("Stage colour");
                        ui.color_edit_button_srgb(&mut state.background);
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new(state.kind.description()).weak());
                }
                NewTab::Templates => {
                    let text = match &state.template {
                        Some(TemplateChoice::Builtin(id)) => docfile::BUILTIN_TEMPLATES.iter().find(|t| t.id == *id).map_or("", |t| t.description).to_owned(),
                        Some(TemplateChoice::User(path)) => format!("Your template {}", path.display()),
                        None => "Choose a template to start from. The new document is untitled, so saving never changes the template.".to_owned(),
                    };
                    ui.label(RichText::new(crate::text(&text)).weak());
                }
            }
            if let Some(error) = &state.error {
                ui.colored_label(egui::Color32::from_rgb(0xe0, 0x50, 0x50), crate::text(error));
            }
        });
    });
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Create").clicked() {
            create = true;
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    (create, cancel)
}

impl AniApp {
    /// Draws the dialogs of this module; returns nothing, the answers are applied at once.
    pub(super) fn doc_dialogs(&mut self, ctx: &Context) {
        self.find_window(ctx);
        let Some(mut dialog) = self.ui.docs.dialog.take() else { return };
        let mut keep = true;
        let title = match &dialog {
            DocDialog::New(_) => "New Document",
            DocDialog::SaveTemplate { .. } => "Save as Template",
            DocDialog::SaveChanges { .. } => "Save Changes",
            DocDialog::Revert => "Revert",
            DocDialog::Recover(_) => "Recover Documents",
        };
        egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| match &mut dialog {
            DocDialog::New(state) => {
                let user = self.ui.docs.templates.clone();
                let mut deleted = None;
                let (create, cancel) = new_document_ui(ui, state, &user, &mut deleted);
                if let Some(path) = deleted {
                    self.doc_action(ctx, crate::docs::DocAction::DeleteTemplate(path));
                }
                if create {
                    match self.create_document(state) {
                        Ok(()) => keep = false,
                        Err(e) => state.error = Some(e),
                    }
                }
                if cancel {
                    keep = false;
                }
            }
            DocDialog::SaveTemplate { name, category, error } => {
                egui::Grid::new("save-template").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(name);
                    ui.end_row();
                    ui.label("Category");
                    ui.text_edit_singleline(category);
                    ui.end_row();
                });
                let mut categories: Vec<&str> = self.ui.docs.templates.iter().map(|t| t.category.as_str()).collect();
                categories.dedup();
                if !categories.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Existing:");
                        for c in categories {
                            if ui.small_button(crate::text(c)).clicked() {
                                c.clone_into(category);
                            }
                        }
                    });
                }
                if let Some(e) = error {
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x50, 0x50), crate::text(e));
                }
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        match self.save_template(name, category) {
                            Ok(()) => keep = false,
                            Err(e) => *error = Some(e),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
            DocDialog::SaveChanges { close, dirty, after } => {
                let Some(&asking) = dirty.first() else {
                    keep = false;
                    return;
                };
                let name = self.tab_infos().into_iter().find(|t| t.id == asking).map_or_else(|| "this document".to_owned(), |t| t.title);
                ui.label(crate::text(&format!("Do you want to save the changes you made to {name}?")));
                ui.label(RichText::new("Your changes will be lost if you do not save them.").weak());
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.activate_tab(asking);
                        self.save(false);
                        if !self.host.session().is_dirty() {
                            dirty.remove(0);
                        }
                    }
                    if ui.button("Don't Save").clicked() {
                        dirty.remove(0);
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
                if keep && dirty.is_empty() {
                    let (close, after) = (std::mem::take(close), *after);
                    keep = false;
                    self.close_tabs(&close, after);
                }
            }
            DocDialog::Revert => {
                let name = self.tab_infos().into_iter().find(|t| t.active).map_or_else(|| "this document".to_owned(), |t| t.title);
                ui.label(crate::text(&format!("Revert to the last saved version of {name}? Your changes will be lost.")));
                ui.horizontal(|ui| {
                    if ui.button("Revert").clicked() {
                        self.revert_now();
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
            DocDialog::Recover(orphans) => {
                ui.label("The program did not close normally last time. These documents had changes that were not saved:");
                for o in orphans.iter() {
                    let from = o.note.original.as_deref().unwrap_or("not saved yet");
                    ui.label(RichText::new(crate::text(&format!("{}  ({from})", o.note.title))).monospace());
                }
                ui.horizontal(|ui| {
                    if ui.button("Recover All").clicked() {
                        for o in std::mem::take(orphans) {
                            if let Err(e) = self.recover(&o) {
                                self.ui.docs.log(crate::docs::Level::Error, format!("could not recover {}: {e}", o.note.title));
                            }
                        }
                        keep = false;
                    }
                    if ui.button("Discard All").clicked() {
                        if let Some(recovery) = &self.docs.recovery {
                            for o in orphans.iter() {
                                recovery.remove(&o.id);
                            }
                        }
                        keep = false;
                    }
                    if ui.button("Decide Later").clicked() {
                        keep = false;
                    }
                });
            }
        });
        if keep {
            self.ui.docs.dialog = Some(dialog);
        }
    }

    fn find_window(&mut self, ctx: &Context) {
        if !self.ui.docs.find.open {
            return;
        }
        let mut open = true;
        let mut action: Option<FindAction> = None;
        let mut library = Vec::new();
        let kind = self.ui.docs.find.query.kind;
        if matches!(kind, Kind::Symbol | Kind::Bitmap | Kind::Sound | Kind::Video) {
            library = self.library_names_for(kind);
        }
        let hits: Vec<String> = self
            .ui
            .docs
            .find
            .hits
            .iter()
            .map(|h| {
                let place = match &h.timeline {
                    anim_scene::TimelineId::Scene(i) => format!("scene {}", i.saturating_add(1)),
                    anim_scene::TimelineId::Symbol(n) => format!("symbol {n}"),
                };
                format!("{}: {}   [{place}, layer {}, frame {}]", h.what, h.context, h.layer.saturating_add(1), h.frame.saturating_add(1))
            })
            .collect();
        let current = self.ui.docs.find.current;
        let mut clicked_hit: Option<usize> = None;
        let find = &mut self.ui.docs.find;
        egui::Window::new("Find and Replace").open(&mut open).collapsible(false).resizable(false).default_pos([60.0, 80.0]).show(ctx, |ui| {
            egui::Grid::new("find-grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Search in");
                let in_scene = matches!(find.query.scope, Scope::Scene(_));
                egui::ComboBox::from_id_salt("find-scope").selected_text(if in_scene { "Current Scene" } else { "Current Document" }).show_ui(ui, |ui| {
                    if ui.selectable_label(!in_scene, "Current Document").clicked() {
                        find.query.scope = Scope::Document;
                    }
                    if ui.selectable_label(in_scene, "Current Scene").clicked() {
                        find.query.scope = Scope::Scene(0);
                    }
                });
                ui.end_row();
                ui.label("For");
                egui::ComboBox::from_id_salt("find-kind").selected_text(find.query.kind.label()).show_ui(ui, |ui| {
                    for k in Kind::ALL {
                        if ui.selectable_value(&mut find.query.kind, k, k.label()).clicked() {
                            find.hits.clear();
                            find.current = None;
                            find.searched = false;
                        }
                    }
                });
                ui.end_row();
                match find.query.kind {
                    Kind::Text | Kind::Font => {
                        ui.label(if find.query.kind == Kind::Text { "Text" } else { "Font" });
                        let response = ui.add(egui::TextEdit::singleline(&mut find.query.find).desired_width(260.0));
                        if find.focus {
                            response.request_focus();
                            find.focus = false;
                        }
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            action = Some(FindAction::Next);
                        }
                        ui.end_row();
                        ui.label("Replace with");
                        ui.add(egui::TextEdit::singleline(&mut find.query.replace).desired_width(260.0));
                        ui.end_row();
                    }
                    Kind::Color => {
                        ui.label("Colour");
                        ui.color_edit_button_srgb(&mut find.color_find);
                        ui.end_row();
                        ui.label("Replace with");
                        ui.color_edit_button_srgb(&mut find.color_replace);
                        ui.end_row();
                    }
                    Kind::Symbol | Kind::Bitmap | Kind::Sound | Kind::Video => {
                        ui.label("Item");
                        egui::ComboBox::from_id_salt("find-item").width(260.0).selected_text(crate::text(&find.query.find)).show_ui(ui, |ui| {
                            for name in &library {
                                ui.selectable_value(&mut find.query.find, name.clone(), crate::text(name));
                            }
                        });
                        ui.end_row();
                        ui.label("Replace with");
                        egui::ComboBox::from_id_salt("find-item-with").width(260.0).selected_text(crate::text(&find.query.replace)).show_ui(ui, |ui| {
                            for name in &library {
                                ui.selectable_value(&mut find.query.replace, name.clone(), crate::text(name));
                            }
                        });
                        ui.end_row();
                    }
                }
            });
            if find.query.kind == Kind::Text {
                ui.horizontal_wrapped(|ui| {
                    for place in TextIn::ALL {
                        let mut on = find.query.text_in.contains(&place);
                        if ui.checkbox(&mut on, place.label()).changed() {
                            find.query.text_in.retain(|p| *p != place);
                            if on {
                                find.query.text_in.push(place);
                            }
                        }
                    }
                });
            }
            if matches!(find.query.kind, Kind::Text | Kind::Font) {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut find.query.match_case, "Match case");
                    ui.checkbox(&mut find.query.whole_word, "Whole word");
                    ui.checkbox(&mut find.query.regex, "Regular expression");
                });
            }
            ui.horizontal(|ui| {
                if ui.button("Find Next").clicked() {
                    action = Some(FindAction::Next);
                }
                if ui.button("Find All").clicked() {
                    action = Some(FindAction::All);
                }
                if ui.add_enabled(find.current.is_some(), egui::Button::new("Replace")).clicked() {
                    action = Some(FindAction::Replace);
                }
                if ui.button("Replace All").clicked() {
                    action = Some(FindAction::ReplaceAll);
                }
            });
            if !find.message.is_empty() {
                ui.label(RichText::new(crate::text(&find.message)).weak());
            }
            if !hits.is_empty() {
                ui.separator();
                egui::ScrollArea::vertical().max_height(180.0).id_salt("find-hits").show(ui, |ui| {
                    ui.set_min_width(420.0);
                    for (i, text) in hits.iter().enumerate() {
                        if ui.selectable_label(current == Some(i), crate::text(text)).clicked() {
                            clicked_hit = Some(i);
                        }
                    }
                });
            }
        });
        if !open {
            self.ui.docs.find.open = false;
        }
        if let Some(i) = clicked_hit {
            self.show_hit(i);
        }
        match action {
            Some(FindAction::Next) => self.find_next(),
            Some(FindAction::All) => self.find_all(),
            Some(FindAction::Replace) => self.find_replace_one(),
            Some(FindAction::ReplaceAll) => self.find_replace_all(),
            None => {}
        }
    }
}

#[derive(Clone, Copy)]
enum FindAction {
    Next,
    All,
    Replace,
    ReplaceAll,
}

/// The contents of the Document Settings dialog. Returns true when the stage size should be matched to the contents.
pub fn settings_ui(ui: &mut Ui, state: &mut SettingsState, document: &anim_scene::Document, confirmed: &mut bool, keep: &mut bool) {
    let units = state.units;
    let (max, speed) = (units.from_pixels(docfile::MAX_STAGE), units.from_pixels(1.0));
    egui::Grid::new("doc-settings").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Platform");
        type_combo(ui, "settings-type", &mut state.kind);
        ui.end_row();
        ui.label("Ruler units");
        units_combo(ui, "settings-units", &mut state.units);
        ui.end_row();
        ui.label("Width");
        let mut w = units.from_pixels(state.width);
        if ui.add(egui::DragValue::new(&mut w).speed(speed).range(speed..=max).suffix(format!(" {}", units.suffix()))).changed() {
            state.width = units.to_pixels(w);
        }
        ui.end_row();
        ui.label("Height");
        let mut h = units.from_pixels(state.height);
        if ui.add(egui::DragValue::new(&mut h).speed(speed).range(speed..=max).suffix(format!(" {}", units.suffix()))).changed() {
            state.height = units.to_pixels(h);
        }
        ui.end_row();
        ui.label("Match");
        ui.horizontal(|ui| {
            if ui.button("Default").on_hover_text("550 x 400 px").clicked() {
                state.width = 550.0;
                state.height = 400.0;
            }
            if ui.add_enabled(docfile::content_bounds(document).is_some(), egui::Button::new("Contents")).on_hover_text("The size of all the artwork").clicked()
                && let Some(b) = docfile::content_bounds(document)
            {
                state.width = b.width().ceil().max(1.0);
                state.height = b.height().ceil().max(1.0);
            }
        });
        ui.end_row();
        ui.label("Scale content");
        ui.checkbox(&mut state.scale_content, "Scale content with stage");
        ui.end_row();
        ui.label("Anchor");
        egui::Grid::new("anchor-grid").spacing([2.0, 2.0]).show(ui, |ui| {
            for (i, anchor) in Anchor::GRID.into_iter().enumerate() {
                let selected = state.anchor == anchor;
                if ui
                    .add_sized([24.0, 24.0], egui::Button::selectable(selected, if selected { "o" } else { "" }))
                    .on_hover_text(anchor.id().replace('_', " "))
                    .clicked()
                {
                    state.anchor = anchor;
                }
                if i % 3 == 2 {
                    ui.end_row();
                }
            }
        });
        ui.end_row();
        ui.label("Frame rate");
        ui.add(egui::DragValue::new(&mut state.fps).range(0.01..=1000.0).suffix(" fps"));
        ui.end_row();
        ui.label("Stage colour");
        ui.color_edit_button_srgb(&mut state.background);
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.label(RichText::new(state.kind.description()).weak());
    ui.horizontal(|ui| {
        if ui.button("OK").clicked() {
            *confirmed = true;
        }
        if ui.button("Cancel").clicked() {
            *keep = false;
        }
    });
}
