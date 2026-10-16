//! File menu: New, Open, Save, Import and Export. Dialogs come from `rfd`; reading and exporting run as session jobs.

use std::path::{Path, PathBuf};

use anicraft_cli::export::Format;
use anicraft_cli::mcp::AniDoc;
use anim_session::JobEvent;

use super::AniApp;
use crate::files::{self, ExportRequest, IMPORT_EXTENSIONS, Loaded, OPEN_EXTENSIONS};
use crate::menus::{ExportKind, MenuAction};
use crate::state::Dialog;

impl AniApp {
    /// True when it is fine to discard the document now; otherwise asks first and runs `action` again once the
    /// person has agreed.
    fn confirmed(&mut self, action: &MenuAction) -> bool {
        if self.host.session().is_dirty() {
            self.ui.dialog = Some(Dialog::Discard(Box::new(action.clone())));
            return false;
        }
        true
    }

    /// Runs the action the person confirmed in the "unsaved changes" dialog.
    pub(super) fn discard_confirmed(&mut self, action: MenuAction) {
        match action {
            MenuAction::New => self.replace_document(AniDoc::default(), None),
            MenuAction::Open => self.pick_and_open(),
            MenuAction::OpenRecent(path) => self.start_open(path),
            MenuAction::Quit => {
                self.closing = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
    }

    /// Replaces the document of the session; the history starts afresh.
    pub(crate) fn replace_document(&mut self, doc: AniDoc, path: Option<PathBuf>) {
        if let Some(path) = &path {
            self.remember(path);
        }
        self.host.session_mut().replace_document(doc, path);
        self.edits = self.edits.saturating_add(1);
        self.reset_view_for_new_document();
    }

    fn remember(&mut self, path: &Path) {
        self.ui.recent.retain(|p| p != path);
        self.ui.recent.insert(0, path.to_path_buf());
        self.ui.recent.truncate(10);
    }

    pub(super) fn new_document(&mut self) {
        if self.confirmed(&MenuAction::New) {
            self.replace_document(AniDoc::default(), None);
            self.status("new document");
        }
    }

    pub(super) fn open_dialog(&mut self) {
        if self.confirmed(&MenuAction::Open) {
            self.pick_and_open();
        }
    }

    fn pick_and_open(&mut self) {
        let picked = rfd::FileDialog::new().set_title("Open").add_filter("Animate documents", &OPEN_EXTENSIONS).pick_file();
        if let Some(path) = picked {
            self.start_open(path);
        }
    }

    pub(super) fn open_path(&mut self, path: PathBuf) {
        if self.confirmed(&MenuAction::OpenRecent(path.clone())) {
            self.start_open(path);
        }
    }

    fn start_open(&mut self, path: PathBuf) {
        self.status(format!("opening {}...", path.display()));
        self.jobs.spawn("open", files::open_job(path));
    }

    /// Collects what finished in the background.
    pub(crate) fn poll_jobs(&mut self) {
        for (_, event) in self.jobs.poll() {
            match event {
                JobEvent::Progress { fraction, message } => self.finish_export_status(&message, fraction),
                JobEvent::Finished(Loaded::Opened { path, project }) => {
                    self.ui.export = None;
                    let warnings = project.loaded.warnings.len();
                    self.replace_document(AniDoc::from(*project), Some(path.clone()));
                    self.status(format!("opened {}{}", path.display(), if warnings > 0 { format!(" ({warnings} reader warnings)") } else { String::new() }));
                }
                JobEvent::Finished(Loaded::Exported { label, outcome }) => {
                    self.ui.export = None;
                    let first = outcome.files.first().map(|p| p.display().to_string()).unwrap_or_default();
                    self.status(format!("exported {label}: {} frame(s), {} bytes to {first}", outcome.frames, outcome.bytes));
                }
                JobEvent::Failed(message) => {
                    self.ui.export = None;
                    self.ui.dialog = Some(Dialog::Message { title: "That did not work".to_owned(), text: message });
                }
                JobEvent::Cancelled => {
                    self.ui.export = None;
                    self.status("cancelled");
                }
            }
        }
    }

    pub(super) fn save(&mut self, save_as: bool) {
        let existing = self.host.session().path().map(Path::to_path_buf);
        let path = match (existing, save_as) {
            (Some(path), false) => path,
            (existing, _) => {
                let name = existing.as_deref().and_then(Path::file_name).map_or_else(|| "Untitled.fla".to_owned(), |n| n.to_string_lossy().into_owned());
                let picked = rfd::FileDialog::new()
                    .set_title("Save As")
                    .add_filter("Animate document", &["fla"])
                    .add_filter("XFL folder", &["xfl"])
                    .set_file_name(name)
                    .save_file();
                match picked {
                    Some(path) => path,
                    None => return,
                }
            }
        };
        let is_xfl = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xfl"));
        let result = if is_xfl {
            let session = self.host.session_mut();
            let written = files::write_xfl(session.doc(), &path);
            if written.is_ok() {
                session.set_path(Some(path.clone()));
                session.mark_saved();
            }
            written
        } else {
            self.host.session_mut().save_as(path.clone(), files::write_fla).map_err(|e| e.to_string())
        };
        match result {
            Ok(()) => {
                self.remember(&path);
                self.status(format!("saved {}", path.display()));
            }
            Err(e) => self.ui.dialog = Some(Dialog::Message { title: "Could not save".to_owned(), text: e }),
        }
    }

    pub(super) fn import_dialog(&mut self, to_stage: bool) {
        let picked = rfd::FileDialog::new().set_title("Import").add_filter("Artwork, images and sounds", &IMPORT_EXTENSIONS).pick_file();
        if let Some(path) = picked {
            self.import_path(&path, to_stage);
        }
    }

    /// Imports one file into the library (and the stage).
    pub(crate) fn import_path(&mut self, path: &Path, to_stage: bool) {
        let result = self.host.session_mut().edit("Import", None, |doc| files::import_artwork(doc, path, to_stage));
        match result {
            Ok(name) => self.status(format!("imported {name}")),
            Err(e) => {
                self.host.session_mut().undo();
                self.ui.dialog = Some(Dialog::Message { title: "Could not import".to_owned(), text: e });
            }
        }
    }

    pub(super) fn export_dialog(&mut self, kind: ExportKind) {
        let stem = self.host.session().path().and_then(Path::file_stem).map_or_else(|| "animation".to_owned(), |s| s.to_string_lossy().into_owned());
        match kind {
            ExportKind::Image => {
                let picked = rfd::FileDialog::new().set_title("Export Image").add_filter("PNG", &["png"]).set_file_name(format!("{stem}.png")).save_file();
                let Some(path) = picked else { return };
                match files::current_frame_png(self.host.session().doc(), 1.0).and_then(|png| std::fs::write(&path, png).map_err(|e| e.to_string())) {
                    Ok(()) => self.status(format!("exported {}", path.display())),
                    Err(e) => self.ui.dialog = Some(Dialog::Message { title: "Could not export".to_owned(), text: e }),
                }
            }
            ExportKind::Format(format) => {
                let picked = if format == Format::PngSeq {
                    rfd::FileDialog::new().set_title("Export PNG Sequence").pick_folder().map(|dir| dir.join(&stem))
                } else {
                    rfd::FileDialog::new()
                        .set_title("Export")
                        .add_filter(format.describe(), &[format.extension()])
                        .set_file_name(format!("{stem}.{}", format.extension()))
                        .save_file()
                };
                if let Some(out) = picked {
                    self.start_export(format, out);
                }
            }
        }
    }

    /// Exports the current scene in the background.
    pub(crate) fn start_export(&mut self, format: Format, out: PathBuf) {
        let doc = self.host.session().doc();
        let scene = match &doc.editor.state.timeline {
            anim_scene::TimelineId::Scene(i) => *i,
            anim_scene::TimelineId::Symbol(_) => doc.document().current_timeline,
        };
        let request = ExportRequest { doc: doc.document().clone(), media: doc.media.clone(), scene, frame: doc.editor.state.frame, format, out, scale: 1.0 };
        self.finish_export_status(&format!("exporting {}", format.id()), 0.0);
        self.jobs.spawn("export", files::export_job(request));
    }
}
