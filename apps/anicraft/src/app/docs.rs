//! Document tabs, Save All, Revert, Save and Compact, templates, autosave and recovery, the history log and Find and
//! Replace: everything the File and Edit menus do with whole documents.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anicraft_cli::mcp::AniDoc;
use anicraft_cli::mcp::docfile::{self, PlayOptions};
use anicraft_cli::mcp::find::{self, Hit, Kind};
use anicraft_cli::project::Project;
use anim_session::Session;
use anim_ui::PanelId;
use egui::Context;
use serde_json::{Value, json};

/// A document waiting for its recovery copy: id, content, path, title and the (edits, position, counter) stamp.
type AutosaveJob = (u64, AniDoc, Option<PathBuf>, String, (usize, usize, u64));

use super::AniApp;
use crate::docs::{
    After, DOC_PANELS, DocAction, DocDialog, DocView, ERRORS_PANEL, HISTORY_PANEL, Level, NewDocState, NewTab, Parked, PlayToggle, SettingsState, Step,
    TabEvent, TabInfo, TemplateChoice, tab_title,
};
use crate::menus::MenuAction;
use crate::state::Dialog;

/// How often the autosave check looks at the clock.
const AUTOSAVE_POLL: Duration = Duration::from_secs(5);

impl AniApp {
    // ---- start-up ---------------------------------------------------------------------------------------------

    /// Called once after the window state is built: offers crashed documents back and lists the templates.
    pub(crate) fn docs_start(&mut self) {
        self.refresh_templates();
        self.refresh_commands();
        self.ui.recent = self.docs.prefs.get().common.recent.to_vec();
        if let Some(recovery) = &self.docs.recovery {
            recovery.heartbeat();
            let orphans = recovery.orphans();
            recovery.tidy();
            if !orphans.is_empty() && self.ui.docs.dialog.is_none() {
                self.ui.docs.log(Level::Warning, format!("{} document(s) were left by a session that did not close normally", orphans.len()));
                self.ui.docs.dialog = Some(DocDialog::Recover(orphans));
            }
        }
    }

    pub(crate) fn refresh_commands(&mut self) {
        self.ui.docs.commands = self.docs.prefs.commands_dir().map(|d| crate::macros::list(&d)).unwrap_or_default();
    }

    pub(crate) fn refresh_templates(&mut self) {
        self.ui.docs.templates = self.docs.prefs.templates_dir().map(|d| crate::templates::list(&d)).unwrap_or_default();
    }

    // ---- per frame --------------------------------------------------------------------------------------------

    /// Per-frame bookkeeping: the history log, autosave, the recovery heartbeat, settings, mute and the problems list.
    pub(crate) fn docs_frame(&mut self, ctx: &Context) {
        let history = self.host.session().history();
        let (undo, redo) = (history.undo_len(), history.redo_len());
        self.docs.steps.sync(undo, redo);
        self.docs.prefs.tick();
        let now = Instant::now();
        if now.duration_since(self.docs.last_heartbeat) >= crate::recovery::HEARTBEAT {
            self.docs.last_heartbeat = now;
            if let Some(recovery) = &self.docs.recovery {
                recovery.heartbeat();
            }
        }
        let minutes = self.docs.prefs.get().common.autosave_minutes;
        if minutes > 0 && now.duration_since(self.docs.last_autosave) >= Duration::from_secs(u64::from(minutes).saturating_mul(60)) {
            self.docs.last_autosave = now;
            self.autosave_all();
        }
        self.apply_mute();
        if self.ui.docs.problems_for.is_none() || self.problems_stale() {
            if self.ui.layout.contains(&ERRORS_PANEL) {
                self.refresh_problems();
            } else {
                self.ui.docs.problems_for = None;
            }
        }
        ctx.request_repaint_after(AUTOSAVE_POLL);
    }

    fn problems_stamp(&self) -> (u64, usize, usize) {
        let history = self.host.session().history();
        (self.edits, history.undo_len(), history.redo_len())
    }

    fn problems_stale(&self) -> bool {
        self.ui.docs.problems_for != Some(self.problems_stamp())
    }

    pub(crate) fn refresh_problems(&mut self) {
        self.ui.docs.problems = crate::docs::problems(self.host.session().doc().document());
        self.ui.docs.problems_for = Some(self.problems_stamp());
    }

    fn apply_mute(&mut self) {
        let muted = self.play_options().mute_sounds;
        if muted != self.docs.muted_applied {
            self.docs.muted_applied = muted;
            if muted {
                self.audio.pause();
            } else if self.ui.playback.is_some() {
                // Unmuted while playing: start again from here so the soundtrack catches up.
                let frame = self.host.session().doc().editor.state.frame;
                self.start_playback(frame);
            }
        }
    }

    /// The Control menu toggles of the active document.
    pub(crate) fn play_options(&self) -> PlayOptions {
        docfile::play_options(self.host.session().doc().document())
    }

    // ---- menu actions -----------------------------------------------------------------------------------------

    pub(super) fn doc_action(&mut self, ctx: &Context, action: DocAction) {
        match action {
            DocAction::NewDialog => self.open_new_dialog(),
            DocAction::NewFrom(choice) => self.new_from_template(&choice),
            DocAction::SaveAll => self.save_all(),
            DocAction::Revert => self.ask_revert(),
            DocAction::SaveCompact => self.save_and_compact(),
            DocAction::SaveTemplate => self.open_save_template(),
            DocAction::CloseTab => {
                let id = self.docs.tabs.active;
                self.request_close(vec![id], After::CloseTabs);
            }
            DocAction::CloseAll => {
                let all = self.docs.tabs.order.clone();
                self.request_close(all, After::CloseTabs);
            }
            DocAction::SelectTab(id) => self.activate_tab(id),
            DocAction::NextTab => {
                if let Some(to) = self.docs.tabs.neighbour(self.docs.tabs.active, false) {
                    self.activate_tab(to);
                }
            }
            DocAction::PrevTab => {
                if let Some(to) = self.docs.tabs.neighbour(self.docs.tabs.active, true) {
                    self.activate_tab(to);
                }
            }
            DocAction::Quit => {
                let all = self.docs.tabs.order.clone();
                self.request_close(all, After::Quit);
            }
            DocAction::Find => {
                self.ui.docs.find.open = true;
                self.ui.docs.find.focus = true;
            }
            DocAction::FindAgain => self.find_again(),
            DocAction::SetAutosave(minutes) => {
                self.docs.prefs.update(|p| p.common.autosave_minutes = minutes);
                self.status(if minutes == 0 { "autosave is off".to_owned() } else { format!("autosave every {minutes} minute(s)") });
            }
            DocAction::Play(toggle) => self.toggle_play_option(toggle),
            DocAction::ClearRecent => {
                self.docs.prefs.update(|p| p.common.recent.clear());
                self.ui.recent.clear();
            }
            DocAction::TogglePanel(panel) => self.toggle_panel(panel),
            DocAction::DeleteTemplate(path) => {
                match crate::templates::delete(&path) {
                    Ok(()) => self.status("template deleted"),
                    Err(e) => self.status(e),
                }
                self.refresh_templates();
            }
            DocAction::HistoryJump(applied) => self.history_jump(applied),
            DocAction::HistoryReplay { from, to } => self.history_replay(from, to),
            DocAction::SaveSteps { from, to, name } => {
                let steps = self.recorded_steps(from, to);
                let saved = self
                    .docs
                    .prefs
                    .commands_dir()
                    .ok_or_else(|| "this system has no folder for commands".to_owned())
                    .and_then(|d| crate::macros::save(&d, &name, &steps));
                match saved {
                    Ok(()) => {
                        self.status(format!("saved command {name:?}"));
                        self.refresh_commands();
                    }
                    Err(e) => self.status(e),
                }
            }
            DocAction::RunCommand(name) => {
                let steps = self
                    .docs
                    .prefs
                    .commands_dir()
                    .ok_or_else(|| "this system has no folder for commands".to_owned())
                    .and_then(|d| crate::macros::load(&d, &name));
                match steps {
                    Ok(steps) => {
                        let mut done = 0_usize;
                        for step in &steps {
                            if !self.run(&step.id, &step.params) {
                                break;
                            }
                            done = done.saturating_add(1);
                        }
                        self.status(format!("ran {name:?}: {done} of {} step(s)", steps.len()));
                    }
                    Err(e) => self.status(e),
                }
            }
            DocAction::DeleteCommand(name) => {
                if let Some(dir) = self.docs.prefs.commands_dir() {
                    match crate::macros::delete(&dir, &name) {
                        Ok(()) => self.status(format!("deleted command {name:?}")),
                        Err(e) => self.status(e),
                    }
                }
                self.refresh_commands();
            }
            DocAction::GotoProblem(index) => {
                if let Some(p) = self.ui.docs.problems.get(index).cloned() {
                    self.goto_place(p.timeline, p.layer, p.frame);
                }
            }
            DocAction::OpenSettings => {
                self.ui.docs.settings = Some(SettingsState::of(self.host.session().doc().document()));
                self.ui.dialog = Some(Dialog::DocumentSettings);
            }
        }
        let _ = ctx;
    }

    fn toggle_panel(&mut self, panel: PanelId) {
        if self.ui.layout.contains(&panel) {
            self.ui.layout.apply_with_permissions(anim_ui::dock::docking::Action::Close { panel }, |p| p.permissions()).ok();
        } else if let Err(e) = self.open_doc_panel(panel) {
            self.status(format!("cannot open {}: {e}", crate::docs::panel_title(panel)));
        }
    }

    /// Opens one of the panels of this module beside the timeline (or the properties).
    pub(crate) fn open_doc_panel(&mut self, panel: PanelId) -> Result<(), anim_ui::dock::docking::DockError> {
        if !DOC_PANELS.contains(&panel) {
            return anim_ui::dock::open_panel(&mut self.ui.layout, panel);
        }
        let anchor = if panel == HISTORY_PANEL { PanelId::Properties } else { PanelId::Timeline };
        let anchor = [anchor, PanelId::Timeline, PanelId::Properties].into_iter().find(|a| self.ui.layout.contains(a));
        self.ui.layout.apply_with_permissions(anim_ui::dock::docking::Action::Open { panel, anchor }, |p| p.permissions())
    }

    fn toggle_play_option(&mut self, toggle: PlayToggle) {
        let mut options = self.play_options();
        let (flag, name) = match toggle {
            PlayToggle::MuteSounds => (&mut options.mute_sounds, "mute_sounds"),
            PlayToggle::SimpleButtons => (&mut options.simple_buttons, "simple_buttons"),
            PlayToggle::FrameActions => (&mut options.frame_actions, "frame_actions"),
        };
        *flag = !*flag;
        let on = *flag;
        self.run("control.play_options", &json!({ name: on }));
    }

    // ---- tabs -------------------------------------------------------------------------------------------------

    /// What the tab strip shows.
    pub(crate) fn tab_infos(&self) -> Vec<TabInfo> {
        self.docs
            .tabs
            .order
            .iter()
            .map(|&id| {
                let (session, active) = if id == self.docs.tabs.active {
                    (self.host.session(), true)
                } else if let Some(p) = self.docs.tabs.parked(id) {
                    (&p.session, false)
                } else {
                    return TabInfo { id, title: "?".to_owned(), dirty: false, active: false, tooltip: String::new() };
                };
                let number = self.docs.untitled.get(&id).copied().unwrap_or(1);
                TabInfo {
                    id,
                    title: tab_title(session.path(), number),
                    dirty: session.is_dirty(),
                    active,
                    tooltip: session.path().map_or_else(|| "Not saved yet".to_owned(), |p| p.display().to_string()),
                }
            })
            .collect()
    }

    pub(super) fn tab_event(&mut self, event: TabEvent) {
        match event {
            TabEvent::Select(id) => self.activate_tab(id),
            TabEvent::Close(id) => self.request_close(vec![id], After::CloseTabs),
            TabEvent::New => self.open_new_dialog(),
        }
    }

    fn tab_is_dirty(&self, id: u64) -> bool {
        if id == self.docs.tabs.active { self.host.session().is_dirty() } else { self.docs.tabs.parked(id).is_some_and(|p| p.session.is_dirty()) }
    }

    /// Makes `id` the tab on screen; the one that was is parked with its view.
    pub(crate) fn activate_tab(&mut self, id: u64) {
        if id == self.docs.tabs.active {
            return;
        }
        let Some(at) = self.docs.tabs.parked.iter().position(|p| p.id == id) else { return };
        let mut incoming = self.docs.tabs.parked.swap_remove(at);
        self.stop_playback();
        self.ui.drag_base = None;
        let outgoing_view = DocView {
            stage: Some(self.ui.stage.view),
            guides: std::mem::take(&mut self.ui.stage.guides),
            timeline: std::mem::replace(&mut self.ui.timeline, incoming.view.timeline.clone()),
            steps: std::mem::take(&mut self.docs.steps),
        };
        std::mem::swap(self.host.session_mut(), &mut incoming.session);
        let previous = self.docs.tabs.active;
        self.docs.tabs.parked.push(Parked { id: previous, session: incoming.session, view: outgoing_view });
        self.docs.steps = incoming.view.steps;
        self.ui.stage.guides = incoming.view.guides;
        self.docs.tabs.active = id;
        self.reset_view_for_new_document();
        if let Some(view) = incoming.view.stage {
            self.ui.stage.view = view;
            self.ui.stage.fit_request = false;
        }
        self.edits = self.edits.saturating_add(1);
        self.ui.docs.problems_for = None;
        self.ui.docs.find.hits.clear();
        self.ui.docs.find.searched = false;
    }

    /// Opens `doc` in a new tab and shows it.
    pub(crate) fn open_tab(&mut self, doc: AniDoc, path: Option<PathBuf>) {
        let id = self.docs.tabs.allocate();
        if path.is_none() {
            self.docs.number_tab(id);
        }
        let mut session = Session::new(doc, crate::commands::registry());
        session.set_path(path);
        self.docs.tabs.parked.push(Parked { id, session, view: DocView::default() });
        self.docs.tabs.order.push(id);
        self.activate_tab(id);
        self.ui.stage.fit_request = true;
    }

    /// True while the active tab is the untouched untitled document the program started with.
    fn pristine_start(&self) -> bool {
        let session = self.host.session();
        self.docs.tabs.order.len() == 1 && session.path().is_none() && !session.is_dirty() && session.history().undo_len() == 0 && self.docs.tabs.active == 1
    }

    /// A document finished opening: show it in a tab (or, when the file is already open, switch to it).
    pub(crate) fn finish_open(&mut self, path: PathBuf, project: Project) {
        let warnings = project.loaded.warnings.len();
        let doc = AniDoc::from(project);
        self.remember(&path);
        if self.docs.reverting.as_deref() == Some(path.as_path()) {
            self.docs.reverting = None;
            let id = self.docs.tabs.active;
            self.replace_document(doc, Some(path.clone()));
            self.forget_recovery(id);
            self.status(format!("reverted to {}", path.display()));
            return;
        }
        let existing = self.docs.tabs.order.iter().copied().find(|id| {
            let session_path =
                if *id == self.docs.tabs.active { self.host.session().path() } else { self.docs.tabs.parked(*id).and_then(|p| p.session.path()) };
            session_path == Some(path.as_path())
        });
        if let Some(id) = existing {
            self.activate_tab(id);
            self.status(format!("{} is already open", path.display()));
            return;
        }
        if self.pristine_start() {
            self.replace_document(doc, Some(path.clone()));
        } else {
            self.open_tab(doc, Some(path.clone()));
        }
        if warnings > 0 {
            self.ui.docs.log(Level::Warning, format!("{}: {warnings} reader warning(s)", path.display()));
        }
        self.status(format!("opened {}{}", path.display(), if warnings > 0 { format!(" ({warnings} reader warnings)") } else { String::new() }));
    }

    // ---- closing ----------------------------------------------------------------------------------------------

    /// Closes `ids`, asking first about the ones with unsaved changes.
    pub(crate) fn request_close(&mut self, ids: Vec<u64>, after: After) {
        let dirty: Vec<u64> = ids.iter().copied().filter(|id| self.tab_is_dirty(*id)).collect();
        if dirty.is_empty() {
            self.close_tabs(&ids, after);
        } else {
            self.ui.docs.dialog = Some(DocDialog::SaveChanges { close: ids, dirty, after });
        }
    }

    /// Closes tabs without asking.
    pub(crate) fn close_tabs(&mut self, ids: &[u64], after: After) {
        if after == After::Quit {
            self.docs.quit_ok = true;
            self.closing = true;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        for &id in ids {
            self.close_one(id);
        }
    }

    fn forget_recovery(&mut self, id: u64) {
        if let Some(recovery) = &self.docs.recovery {
            recovery.remove(&recovery.id_for(id));
        }
        self.docs.autosaved.remove(&id);
    }

    fn close_one(&mut self, id: u64) {
        if !self.docs.tabs.order.contains(&id) {
            return;
        }
        self.forget_recovery(id);
        if self.docs.tabs.order.len() == 1 {
            // The last document: leave a fresh untitled one, as the window cannot be empty.
            self.docs.number_tab(id);
            self.replace_document(AniDoc::default(), None);
            self.docs.steps.clear();
            self.status("closed");
            return;
        }
        if id == self.docs.tabs.active {
            let to = self.docs.tabs.neighbour(id, false).filter(|t| *t != id).or_else(|| self.docs.tabs.order.iter().copied().find(|t| *t != id));
            let at = self.docs.tabs.order.iter().position(|t| *t == id).unwrap_or(0);
            let to = if at > 0 { self.docs.tabs.order.get(at.saturating_sub(1)).copied().or(to) } else { to };
            if let Some(to) = to {
                self.activate_tab(to);
            }
        }
        self.docs.tabs.parked.retain(|p| p.id != id);
        self.docs.tabs.order.retain(|t| *t != id);
        self.docs.untitled.remove(&id);
    }

    // ---- New Document and templates ---------------------------------------------------------------------------

    fn open_new_dialog(&mut self) {
        self.refresh_templates();
        self.ui.docs.dialog = Some(DocDialog::New(NewDocState::default()));
    }

    fn new_from_template(&mut self, choice: &TemplateChoice) {
        let made = match choice {
            TemplateChoice::Builtin(id) => docfile::builtin_template(id),
            TemplateChoice::User(path) => crate::templates::load(path),
        };
        match made {
            Ok(doc) => {
                self.open_tab(doc, None);
                self.status("new document from template");
            }
            Err(e) => self.ui.dialog = Some(Dialog::Message { title: "Could not make the document".to_owned(), text: e }),
        }
    }

    /// OK of the New Document dialog.
    pub(crate) fn create_document(&mut self, state: &NewDocState) -> Result<(), String> {
        let doc = match (&state.tab, &state.template) {
            (NewTab::Templates, Some(TemplateChoice::Builtin(id))) => docfile::builtin_template(id)?,
            (NewTab::Templates, Some(TemplateChoice::User(path))) => crate::templates::load(path)?,
            (NewTab::Templates, None) => return Err("choose a template".to_owned()),
            (NewTab::Presets, _) => docfile::new_document(&state.spec())?,
        };
        self.open_tab(doc, None);
        self.status("new document");
        Ok(())
    }

    fn open_save_template(&mut self) {
        self.ui.docs.dialog =
            Some(DocDialog::SaveTemplate { name: "My template".to_owned(), category: crate::templates::DEFAULT_CATEGORY.to_owned(), error: None });
    }

    /// OK of the Save as Template dialog.
    pub(crate) fn save_template(&mut self, name: &str, category: &str) -> Result<(), String> {
        let dir = self.docs.prefs.templates_dir().ok_or("this system has no folder for templates")?;
        let path = crate::templates::save(&dir, category, name, self.host.session().doc())?;
        self.refresh_templates();
        self.status(format!("saved template {}", path.display()));
        Ok(())
    }

    // ---- Save All, Revert, Save and Compact -------------------------------------------------------------------

    fn save_all(&mut self) {
        let original = self.docs.tabs.active;
        let mut saved = 0_usize;
        for id in self.docs.tabs.order.clone() {
            if self.tab_is_dirty(id) {
                self.activate_tab(id);
                self.save(false);
                if !self.host.session().is_dirty() {
                    saved = saved.saturating_add(1);
                }
            }
        }
        self.activate_tab(original);
        self.status(format!("saved {saved} document(s)"));
    }

    fn ask_revert(&mut self) {
        if self.host.session().path().is_none() {
            self.status("this document has not been saved; there is nothing to revert to");
            return;
        }
        self.ui.docs.dialog = Some(DocDialog::Revert);
    }

    /// Confirmed Revert: reads the file again into the active tab.
    pub(crate) fn revert_now(&mut self) {
        if let Some(path) = self.host.session().path().map(Path::to_path_buf) {
            self.docs.reverting = Some(path.clone());
            self.start_open(path);
        }
    }

    fn save_and_compact(&mut self) {
        let before = self.host.session().path().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());
        let freed = match self.host.session_mut().execute("document.compact", &json!({})) {
            Ok(v) => v.get("freed_bytes").and_then(Value::as_u64).unwrap_or(0),
            Err(e) => {
                self.status(format!("compact: {e}"));
                return;
            }
        };
        self.save(false);
        if self.host.session().is_dirty() {
            return;
        }
        let after = self.host.session().path().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());
        let size = match (before, after) {
            (Some(b), Some(a)) => format!("; the file went from {b} to {a} bytes"),
            _ => String::new(),
        };
        let note = format!("saved and compacted: {freed} bytes of unused data dropped{size}");
        self.ui.docs.log(Level::Info, note.clone());
        self.status(note);
    }

    /// Called after a successful save: the autosaved copy is no longer needed.
    pub(crate) fn saved_hook(&mut self) {
        let id = self.docs.tabs.active;
        self.forget_recovery(id);
        self.ui.docs.problems_for = None;
    }

    // ---- autosave ---------------------------------------------------------------------------------------------

    fn stamp_of(session: &Session<AniDoc>, edits: u64) -> (usize, usize, u64) {
        (session.history().undo_len(), session.history().redo_len(), edits)
    }

    /// Writes a recovery copy of every document with unsaved changes that changed since its last copy.
    pub(crate) fn autosave_all(&mut self) {
        let Some(recovery) = self.docs.recovery.clone() else { return };
        let mut todo: Vec<AutosaveJob> = Vec::new();
        let active = self.docs.tabs.active;
        for &id in &self.docs.tabs.order {
            let (session, edits) = if id == active {
                (self.host.session(), self.edits)
            } else if let Some(p) = self.docs.tabs.parked(id) {
                (&p.session, 0)
            } else {
                continue;
            };
            if !session.is_dirty() {
                continue;
            }
            let stamp = Self::stamp_of(session, edits);
            if self.docs.autosaved.get(&id) == Some(&stamp) {
                continue;
            }
            let number = self.docs.untitled.get(&id).copied().unwrap_or(1);
            todo.push((id, session.snapshot(), session.path().map(Path::to_path_buf), tab_title(session.path(), number), stamp));
        }
        for (id, doc, original, title, stamp) in todo {
            self.docs.autosaved.insert(id, stamp);
            let rid = recovery.id_for(id);
            let rec = recovery.clone();
            self.jobs.spawn("autosave", move |_ctx| {
                rec.write(&rid, &doc, original.as_deref(), &title).map_err(anim_session::JobError::Failed)?;
                Ok(crate::files::Loaded::Autosaved { title })
            });
        }
    }

    /// OK of the recovery dialog for one document: opens the copy as a tab that still has unsaved changes.
    pub(crate) fn recover(&mut self, orphan: &crate::recovery::Orphan) -> Result<(), String> {
        let doc = crate::templates::load(&orphan.fla)?;
        let original = orphan.note.original.as_ref().map(PathBuf::from);
        let titled = original.is_some();
        self.open_tab(doc, original);
        if !titled {
            let id = self.docs.tabs.active;
            self.docs.untitled.entry(id).or_insert(1);
        }
        // The copy is the only record of these changes, so the tab starts dirty.
        self.host.session_mut().edit("Recover", None, |_| {});
        self.ui.docs.log(Level::Info, format!("recovered {}", orphan.note.title));
        if let Some(recovery) = &self.docs.recovery {
            recovery.remove(&orphan.id);
        }
        Ok(())
    }

    // ---- history ----------------------------------------------------------------------------------------------

    /// Moves the document to the state with `want` steps applied (0 is the document as it was opened).
    fn history_jump(&mut self, want: usize) {
        let session = self.host.session_mut();
        let total = session.history().undo_len().saturating_add(session.history().redo_len());
        if want > total {
            return;
        }
        while session.history().undo_len() > want {
            if session.undo().is_none() {
                break;
            }
        }
        while session.history().undo_len() < want {
            if session.redo().is_none() {
                break;
            }
        }
        self.edits = self.edits.saturating_add(1);
    }

    /// The commands of the steps `from..=to` that have one.
    fn recorded_steps(&self, from: usize, to: usize) -> Vec<Step> {
        (from..=to).filter_map(|i| self.docs.steps.get(i).cloned()).collect()
    }

    /// Replays the commands of steps `from..=to` on the document as it is now.
    fn history_replay(&mut self, from: usize, to: usize) {
        let steps = self.recorded_steps(from, to);
        if steps.is_empty() {
            self.status("none of the selected steps can be replayed");
            return;
        }
        let mut done = 0_usize;
        for step in &steps {
            if self.run(&step.id, &step.params) {
                done = done.saturating_add(1);
            } else {
                break;
            }
        }
        self.status(format!("replayed {done} of {} step(s)", steps.len()));
    }

    // ---- Find and Replace -------------------------------------------------------------------------------------

    fn current_query(&self) -> find::Query {
        let f = &self.ui.docs.find;
        let mut q = f.query.clone();
        if q.kind == Kind::Color {
            q.find = format!("#{:02x}{:02x}{:02x}", f.color_find[0], f.color_find[1], f.color_find[2]);
            q.replace = format!("#{:02x}{:02x}{:02x}", f.color_replace[0], f.color_replace[1], f.color_replace[2]);
        }
        if let find::Scope::Scene(_) = q.scope {
            let doc = self.host.session().doc();
            if let anim_scene::TimelineId::Scene(i) = doc.editor.state.timeline {
                q.scope = find::Scope::Scene(i);
            } else {
                q.scope = find::Scope::Scene(doc.document().current_timeline);
            }
        }
        q
    }

    /// Find All: lists every hit.
    pub(crate) fn find_all(&mut self) {
        let query = self.current_query();
        let mut scratch = self.host.session().doc().document().clone();
        match find::find_all(&mut scratch, &query) {
            Ok(hits) => {
                let n = hits.len();
                self.ui.docs.find.message = if n == 0 { "Not found".to_owned() } else { format!("{n} found") };
                self.ui.docs.find.hits = hits;
                self.ui.docs.find.current = None;
            }
            Err(e) => {
                self.ui.docs.find.message = e.0;
                self.ui.docs.find.hits.clear();
            }
        }
        self.ui.docs.find.searched = true;
    }

    /// Find Next: the hit after the one shown, wrapping.
    pub(crate) fn find_next(&mut self) {
        self.find_all();
        let hits = self.ui.docs.find.hits.len();
        if hits == 0 {
            return;
        }
        let next = self.ui.docs.find.current.map_or(0, |c| c.saturating_add(1).checked_rem(hits).unwrap_or(0));
        self.show_hit(next);
    }

    fn find_again(&mut self) {
        if !self.ui.docs.find.searched && self.ui.docs.find.query.find.is_empty() && self.ui.docs.find.query.kind == Kind::Text {
            self.ui.docs.find.open = true;
            self.ui.docs.find.focus = true;
            return;
        }
        self.find_next();
    }

    /// Selects the hit and brings it into view.
    pub(crate) fn show_hit(&mut self, index: usize) {
        let Some(hit) = self.ui.docs.find.hits.get(index).cloned() else { return };
        self.ui.docs.find.current = Some(index);
        if hit.what == "library item name" {
            self.ui.library_selected = Some(hit.context.clone());
            return;
        }
        self.goto_hit(&hit);
    }

    fn goto_hit(&mut self, hit: &Hit) {
        self.goto_place(Some(hit.timeline.clone()), hit.layer, hit.frame);
        if let Some((first, rest)) = hit.path.split_first() {
            let mut r = anicraft_tools::ElementRef::top_level(hit.layer, hit.frame, *first);
            for i in rest {
                r = r.child(*i);
            }
            let doc = self.host.session_mut().doc_mut();
            doc.editor.state.selection = anicraft_tools::Selection::one(r);
        }
    }

    /// Opens a timeline at a layer and frame.
    pub(crate) fn goto_place(&mut self, timeline: Option<anim_scene::TimelineId>, layer: usize, frame: u32) {
        let doc = self.host.session_mut().doc_mut();
        if let Some(t) = timeline {
            doc.editor.state.timeline = t;
        }
        doc.editor.state.layer = layer;
        doc.editor.state.selection = anicraft_tools::Selection::default();
        self.goto_frame(frame);
    }

    /// Replace: changes the hit shown (or, when none is shown, finds the first), then moves on.
    pub(crate) fn find_replace_one(&mut self) {
        if self.ui.docs.find.current.is_none() {
            self.find_next();
            return;
        }
        let Some(index) = self.ui.docs.find.current else { return };
        let mut params = serde_json::to_value(self.current_query()).unwrap_or(Value::Null);
        if let Some(map) = params.as_object_mut() {
            map.insert("only".to_owned(), json!(index));
        }
        if self.run("find.replace", &params) {
            self.find_all();
            let n = self.ui.docs.find.hits.len();
            if n > 0 {
                self.show_hit(index.min(n.saturating_sub(1)));
            }
        }
    }

    /// Replace All.
    pub(crate) fn find_replace_all(&mut self) {
        let params = serde_json::to_value(self.current_query()).unwrap_or(Value::Null);
        let before = self.ui.docs.find.hits.len();
        let result = self.host.session_mut().execute("find.replace", &params);
        match result {
            Ok(v) => {
                let n = v.get("replaced").and_then(Value::as_u64).unwrap_or(0);
                self.ui.docs.find.message = format!("{n} replaced");
                self.find_all();
                let _ = before;
            }
            Err(e) => self.ui.docs.find.message = e.to_string(),
        }
    }

    /// The names of the library items a find kind can point at.
    pub(crate) fn library_names_for(&self, kind: Kind) -> Vec<String> {
        self.host
            .session()
            .doc()
            .document()
            .library
            .items
            .iter()
            .filter(|item| {
                matches!(
                    (kind, item),
                    (Kind::Symbol, anim_scene::LibraryItem::Symbol(_))
                        | (Kind::Bitmap, anim_scene::LibraryItem::Bitmap(_))
                        | (Kind::Sound, anim_scene::LibraryItem::Sound(_))
                        | (Kind::Video, anim_scene::LibraryItem::Video(_))
                )
            })
            .map(|i| i.name().to_owned())
            .collect()
    }

    /// OK of Document Settings.
    pub(crate) fn apply_settings(&mut self) {
        if let Some(state) = self.ui.docs.settings.take() {
            self.run("document.settings", &state.params());
            self.ui.stage.fit_request = true;
        }
    }
}

impl AniApp {
    /// A menu action built from a toggle, for the menus.
    #[must_use]
    pub fn doc_menu(action: DocAction) -> MenuAction {
        MenuAction::Doc(action)
    }
}
