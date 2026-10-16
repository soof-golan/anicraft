//! The editor window: an [`eframe::App`] whose state is a `SessionHost` (document, history, MCP) plus view state.
//!
//! Every frame runs the same steps, none of which keeps UI state outside [`AniApp`]:
//!
//! 1. answer MCP requests, collect finished renders, advance playback;
//! 2. build the view models from the document ([`crate::view`], [`crate::props`]) and draw the panels, which return
//!    events instead of changing anything;
//! 3. turn the events into commands on the session ([`crate::actions`], [`crate::input`]);
//! 4. tell the render worker what to draw next, and finish screenshots.

mod actions;
mod doc_dialogs;
mod docs;
mod draw;
mod fileops;
mod input;
mod layout_actions;
mod library_preview;
mod testmovie;
mod toolui_events;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anicraft_cli::mcp::AniDoc;
use anicraft_cli::project::Project;
use anim_core::num;
use anim_mcp::{Confinement, Reply};
use anim_session::JobSet;
use anim_ui::Theme;
use egui::{ColorImage, Context, TextureHandle, TextureOptions, UserData, ViewportCommand};
use serde_json::json;

use crate::files::Loaded;
use crate::mcp::{Host, LinkOptions, Links, PendingShot};
use crate::state::UiState;
use crate::worker::{Done, InPlace, Job, Onion, RenderJob, RenderWorker, Target};

/// What to do with the window before closing it, for `--screenshot`.
#[derive(Debug, Clone)]
pub struct ShotPlan {
    /// Where the PNG goes.
    pub out: PathBuf,
    /// 1-based frame to show, or the document's first.
    pub frame: Option<u32>,
    /// Crop to one panel (`stage`, `timeline`, ...).
    pub panel: Option<String>,
    /// Tool to activate before the shot, for demonstrating overlays.
    pub tool: Option<String>,
    /// Open the Properties panel on this selection: `layer:keyframe:index`.
    pub select: Option<String>,
    /// Select this library item (its full path) so the preview shows it.
    pub library: Option<String>,
    /// With `library`: start the preview playing.
    pub library_play: bool,
    /// Things to show before the shot, comma separated: `new-document`, `templates`, `settings`, `find`, `history`,
    /// `output`, `errors`, `tabs`, `recovery`, `save-changes`, `menu-file`.
    pub show: Option<String>,
}

/// Everything the command line can ask for.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub file: Option<PathBuf>,
    pub shot: Option<ShotPlan>,
    pub dark: bool,
    pub links: LinkOptions,
    pub roots: Vec<PathBuf>,
    pub window: Option<(f32, f32)>,
}

/// The part of a screenshot a panel occupies (the whole image without a panel).
fn crop(image: &ColorImage, rect: Option<&egui::Rect>, ppp: f32) -> ColorImage {
    rect.map_or_else(|| image.clone(), |rect| image.region(rect, Some(ppp)))
}

/// Most frames a screenshot requested over MCP waits for the stage to catch up with the document.
const MAX_SHOT_WAIT_FRAMES: u32 = 120;

/// What the render worker was last asked for; two requests with equal keys draw the same pixels.
#[derive(Debug, Clone, PartialEq)]
struct RenderKey {
    revision: u64,
    target: Target,
    frame: u32,
    size: [u32; 2],
    onion: Option<(u32, u32)>,
    playing: bool,
    in_place: Option<InPlace>,
    margin: f32,
}

#[derive(Debug, Default)]
struct Sync {
    fingerprint: Option<Fingerprint>,
    media: (usize, u64),
    revision: u64,
    next_id: u64,
    requested: Option<RenderKey>,
    inflight: HashMap<u64, RenderKey>,
    shown_id: u64,
    shown: Option<RenderKey>,
    last_took: Duration,
    last_error: Option<String>,
}

/// The context a symbol edited in place is shown in: the timeline the chain of in-place edits started from, at the
/// frame it was left on, and the matrix that places the symbol in it.
fn in_place_context(state: &anicraft_tools::EditorState, scenes: usize) -> Option<InPlace> {
    let matrix = state.in_place?;
    // The root is the last level that was not itself in place (its space is the stage's).
    let root = state.in_place_stack.iter().rposition(Option::is_none)?;
    let (timeline, _, frame) = state.timeline_stack.get(root)?;
    let target = match timeline {
        anim_scene::TimelineId::Scene(i) => Target::Scene((*i).min(scenes.saturating_sub(1))),
        anim_scene::TimelineId::Symbol(name) => Target::Symbol(name.clone()),
    };
    Some(InPlace { target, frame: *frame, matrix })
}

/// A cheap summary of "the document may have changed".
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    undo: usize,
    redo: usize,
    dirty: bool,
    edits: u64,
    path: Option<PathBuf>,
    scenes: usize,
}

#[derive(Debug)]
enum ShotStage {
    /// Wait for fonts, layout and the first render to settle.
    Settling {
        frames: u32,
    },
    Requested,
}

#[derive(Debug)]
struct ShotRun {
    plan: ShotPlan,
    stage: ShotStage,
}

/// The editor.
pub struct AniApp {
    pub(crate) host: Host,
    pub(crate) ui: UiState,
    pub(crate) worker: RenderWorker,
    pub(crate) jobs: JobSet<Loaded>,
    links: Links,
    stage_tex: Option<TextureHandle>,
    sync: Sync,
    /// Counts document edits made by the window itself (pointer gestures change the document outside history).
    pub(crate) edits: u64,
    shot: Option<ShotRun>,
    mcp_shots: HashMap<u64, Vec<PendingShot>>,
    /// Frames an agent's screenshot has waited for the stage to settle.
    shot_waited: u32,
    next_shot: u64,
    title: String,
    pub(crate) audio: crate::audio::Audio,
    pub(crate) soundtrack_id: u64,
    fonts: bool,
    frames_drawn: u32,
    closing: bool,
    outcome: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    ctx: Context,
    /// Distinguishes property drags, so each drag is one undo step.
    prop_gesture: u64,
    /// Tabs, settings, autosave and recovery.
    pub(crate) docs: crate::docs::Docs,
}

impl AniApp {
    /// Builds the window state. Loading the file happens here, before the first frame, so a bad file is reported at
    /// start-up.
    ///
    /// # Errors
    /// When the document cannot be read or a transport cannot start.
    pub fn new(ctx: &Context, options: &Options, outcome: Option<std::sync::mpsc::Sender<Result<(), String>>>) -> color_eyre::Result<Self> {
        let (doc, path) = match &options.file {
            Some(path) => (AniDoc::from(Project::open(path)?), Some(path.clone())),
            None => (AniDoc::default(), None),
        };
        let size = (doc.document().width, doc.document().height);
        let mut roots = options.roots.clone();
        if let Some(parent) = path.as_ref().and_then(|p| p.parent()).filter(|p| !p.as_os_str().is_empty()) {
            roots.push(parent.to_path_buf());
        }
        roots.retain(|r| r.is_dir());
        roots.sort();
        roots.dedup();
        let confinement = Confinement::new(&roots, &roots).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
        let host = crate::mcp::host(doc, path, confinement);
        let wake = {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        let worker = RenderWorker::spawn(wake.clone())?;
        let links = Links::start(&options.links, wake)?;
        let rtl_fonts = anim_ui::theme::install_fallback_fonts(ctx);
        let theme = if options.dark { Theme::Dark } else { Theme::Light };
        theme.apply(ctx);
        let mut ui = UiState::new(size, theme);
        if rtl_fonts == 0 {
            "no installed font covers Hebrew or Arabic; right-to-left text will show boxes".clone_into(&mut ui.status);
        }
        let mut app = Self {
            host,
            ui,
            worker,
            jobs: JobSet::new(),
            links,
            stage_tex: None,
            sync: Sync::default(),
            edits: 0,
            shot: options.shot.clone().map(|plan| ShotRun { plan, stage: ShotStage::Settling { frames: 0 } }),
            mcp_shots: HashMap::new(),
            shot_waited: 0,
            next_shot: 1,
            title: String::new(),
            audio: crate::audio::Audio::default(),
            soundtrack_id: 0,
            fonts: false,
            frames_drawn: 0,
            closing: false,
            outcome,
            ctx: ctx.clone(),
            prop_gesture: 0,
            docs: crate::docs::Docs::new(crate::prefs::Store::open(options.shot.is_some())),
        };
        if let Some(path) = &options.file {
            app.remember(path);
        }
        app.docs_start();
        if let Some(run) = &app.shot {
            let plan = run.plan.clone();
            app.prepare_shot(&plan);
        }
        Ok(app)
    }

    fn prepare_shot(&mut self, plan: &ShotPlan) {
        if let Some(frame) = plan.frame {
            self.goto_frame(frame.saturating_sub(1));
        }
        if let Some(tool) = plan.tool.as_deref().and_then(anicraft_tools::ToolId::from_toolbar_id) {
            let doc = self.host.session_mut().doc_mut();
            doc.editor.select_tool(tool);
        }
        if let Some(name) = &plan.library {
            self.show_library_item(name, plan.library_play);
        }
        if let Some(show) = &plan.show {
            self.show_for_shot(show);
        }
        if let Some(spec) = &plan.select {
            let parts: Vec<usize> = spec.split(':').filter_map(|p| p.parse().ok()).collect();
            if let [layer, keyframe, index] = parts.as_slice() {
                let doc = self.host.session_mut().doc_mut();
                doc.editor.state.layer = *layer;
                doc.editor.state.selection =
                    anicraft_tools::Selection::one(anicraft_tools::ElementRef::top_level(*layer, u32::try_from(*keyframe).unwrap_or(0), *index));
            }
        }
    }

    /// Opens the dialogs and panels `--show` names, so a screenshot can capture them.
    fn show_for_shot(&mut self, show: &str) {
        use crate::docs::{DocAction, DocDialog, Level, NewDocState, NewTab, SettingsState, TemplateChoice};
        let ctx = self.ctx.clone();
        for what in show.split(',').map(str::trim) {
            match what {
                "new-document" => self.doc_action(&ctx, DocAction::NewDialog),
                "templates" => {
                    let mut state = NewDocState { tab: NewTab::Templates, ..NewDocState::default() };
                    state.template = Some(TemplateChoice::Builtin("character_scene"));
                    self.ui.docs.dialog = Some(DocDialog::New(state));
                }
                "settings" => {
                    self.ui.docs.settings = Some(SettingsState::of(self.host.session().doc().document()));
                    self.ui.dialog = Some(crate::state::Dialog::DocumentSettings);
                }
                "find" => self.doc_action(&ctx, DocAction::Find),
                "history" => {
                    for (id, params) in [("layer.add", serde_json::json!({})), ("scene.add", serde_json::json!({ "name": "Scene 2" }))] {
                        self.run(id, &params);
                    }
                    self.doc_action(&ctx, DocAction::TogglePanel(crate::docs::HISTORY_PANEL));
                }
                "output" => {
                    self.ui.docs.log(Level::Info, "Opened the document");
                    self.ui.docs.log(Level::Warning, "Font Foo is missing; substituting");
                    self.ui.docs.log(Level::Error, "Frame 12: stop( is never closed");
                    self.doc_action(&ctx, DocAction::TogglePanel(anim_ui::PanelId::Output));
                }
                "errors" => {
                    let first = self
                        .host
                        .session_mut()
                        .doc_mut()
                        .editor
                        .document
                        .scenes
                        .first_mut()
                        .and_then(|s| s.layers.first_mut())
                        .and_then(|l| l.frames.first_mut());
                    if let Some(frame) = first {
                        frame.actionscript = Some("function f( { trace(1;".to_owned());
                    }
                    self.refresh_problems();
                    self.doc_action(&ctx, DocAction::TogglePanel(crate::docs::ERRORS_PANEL));
                }
                "actions" | "testmovie" => {
                    let script = "// Stop on the last frame, and jump to the label \"end\" when the button is clicked.\nstop();\nvar clicks = 0;\nbtn.addEventListener(MouseEvent.CLICK, function (e) {\n    clicks++;\n    trace(\"clicked \" + clicks);\n    gotoAndPlay(\"end\");\n});";
                    self.run("timeline.set_script", &serde_json::json!({ "layer": 0, "frame": 1, "script": script }));
                    if what == "testmovie" {
                        self.toggle_test_movie();
                    }
                    self.doc_action(&ctx, DocAction::TogglePanel(crate::docs::ACTIONS_PANEL));
                }
                "tabs" => {
                    self.open_tab(AniDoc::default(), None);
                    self.open_tab(AniDoc::default(), None);
                }
                "save-changes" => {
                    self.run("layer.add", &serde_json::json!({}));
                    let id = self.docs.tabs.active;
                    self.request_close(vec![id], crate::docs::After::CloseTabs);
                }
                "revert" => self.ui.docs.dialog = Some(DocDialog::Revert),
                "motion" => self.ui.motion.open = true,
                "motion-curve" => {
                    self.ui.motion.open = true;
                    self.ui.motion.curve = Some(crate::motion_ui::CurveEdit {
                        target: crate::motion_ui::CurveTarget::Tween(anim_scene::EaseTarget::All),
                        p1: [0.25, 0.0],
                        p2: [0.75, 1.15],
                    });
                }
                "optimize-dialog" => self.ui.dialog = Some(crate::state::Dialog::Shape(crate::shape_ui::ShapeDialog::optimize())),
                "expand-dialog" => self.ui.dialog = Some(crate::state::Dialog::Shape(crate::shape_ui::ShapeDialog::expand_fill())),
                "soften-dialog" => self.ui.dialog = Some(crate::state::Dialog::Shape(crate::shape_ui::ShapeDialog::soften_edges())),
                "scale-dialog" => self.ui.dialog = Some(crate::state::Dialog::Shape(crate::shape_ui::ShapeDialog::scale_rotate())),
                "trace-dialog" => self.ui.dialog = Some(crate::state::Dialog::Shape(crate::shape_ui::ShapeDialog::trace_bitmap())),
                "paste-dialog" => self.ui.dialog = Some(crate::state::Dialog::Shape(crate::shape_ui::ShapeDialog::paste_special())),
                other => self.layout_shot(other),
            }
        }
    }

    /// Moves the playhead (0-based) without recording history.
    pub(crate) fn goto_frame(&mut self, frame: u32) {
        let doc = self.host.session_mut().doc_mut();
        let last = doc.editor.document.timeline(&doc.editor.state.timeline).map_or(0, |t| t.frame_count().saturating_sub(1));
        doc.editor.state.frame = frame.min(last.max(frame.min(10_000)));
    }

    fn fingerprint(&self) -> Fingerprint {
        let session = self.host.session();
        Fingerprint {
            undo: session.history().undo_len(),
            redo: session.history().redo_len(),
            dirty: session.is_dirty(),
            edits: self.edits,
            path: session.path().map(std::path::Path::to_path_buf),
            scenes: session.doc().document().scenes.len(),
        }
    }

    /// Step 1 of the frame: everything that arrives from outside the window.
    fn receive(&mut self, ctx: &Context) {
        let handled = self.links.pump(&mut self.host);
        if handled > 0 {
            ctx.request_repaint();
        }
        self.host.hooks_mut().playing = self.ui.playback.is_some();
        self.collect_shots(ctx);
        self.poll_worker(ctx);
        self.poll_jobs();
        self.tick_playback(ctx);
        self.tick_test_movie(ctx);
        let title = self.host.session().title("AniCraft");
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn poll_worker(&mut self, ctx: &Context) {
        for done in self.worker.poll() {
            match done {
                Done::Frame { id, image, took } => {
                    if id < self.sync.shown_id {
                        continue;
                    }
                    self.sync.shown_id = id;
                    self.sync.last_took = took;
                    self.sync.last_error = None;
                    self.sync.shown = self.sync.inflight.remove(&id);
                    self.sync.inflight.retain(|k, _| *k > id);
                    let size = [num::u32_to_usize(image.width), num::u32_to_usize(image.height)];
                    let colour = ColorImage::from_rgba_premultiplied(size, &image.premultiplied_rgba);
                    match &mut self.stage_tex {
                        Some(tex) => tex.set(colour, TextureOptions::LINEAR),
                        None => self.stage_tex = Some(ctx.load_texture("stage", colour, TextureOptions::LINEAR)),
                    }
                }
                Done::Failed { id: _, message } => {
                    self.sync.last_error = Some(message.clone());
                    self.ui.status = format!("render failed: {message}");
                }
                Done::Sound { id, track, notes } => self.soundtrack_ready(id, track, &notes),
                Done::Preview { name, frame: _, data } => self.preview_ready(ctx, &name, data),
                Done::Thumbnail { name, image } => {
                    if let Some(image) = image {
                        let size = [num::u32_to_usize(image.width), num::u32_to_usize(image.height)];
                        let colour = ColorImage::from_rgba_premultiplied(size, &image.premultiplied_rgba);
                        let tex = ctx.load_texture(format!("thumb:{name}"), colour, TextureOptions::LINEAR);
                        self.ui.thumbs.insert(name, tex);
                    }
                }
            }
        }
    }

    /// Step 4: keep the worker's document in step with the session and ask for the pixels the stage needs.
    fn sync_worker(&mut self, ctx: &Context) {
        let fingerprint = self.fingerprint();
        let doc = self.host.session().doc();
        let media_now = (doc.media.len(), doc.media.total_bytes());
        if self.sync.fingerprint.as_ref() != Some(&fingerprint) || self.sync.revision == 0 {
            self.sync.revision = self.sync.revision.saturating_add(1);
            let media = (media_now != self.sync.media || self.sync.revision == 1).then(|| Box::new(doc.media.clone()));
            self.sync.media = media_now;
            self.worker.send(Job::Load { document: Box::new(doc.document().clone()), media });
            self.sync.fingerprint = Some(fingerprint);
        }
        let state = &doc.editor.state;
        let scenes = doc.document().scenes.len().max(1);
        let target = match &state.timeline {
            anim_scene::TimelineId::Scene(i) => Target::Scene((*i).min(scenes.saturating_sub(1))),
            anim_scene::TimelineId::Symbol(name) => Target::Symbol(name.clone()),
        };
        let size = self.ui.stage.desired_texture_size(ctx.pixels_per_point());
        let onion = (self.ui.onion.enabled && self.ui.playback.is_none()).then_some((self.ui.onion.before, self.ui.onion.after));
        let in_place = in_place_context(state, scenes);
        let key = RenderKey {
            revision: self.sync.revision,
            target: target.clone(),
            frame: state.frame,
            size,
            onion,
            playing: self.ui.playback.is_some() || self.ui.test_movie.is_some(),
            in_place,
            margin: self.ui.stage.texture_margin,
        };
        if self.sync.requested.as_ref() != Some(&key) {
            self.sync.next_id = self.sync.next_id.saturating_add(1);
            let id = self.sync.next_id;
            self.worker.send(Job::Render(RenderJob {
                id,
                target,
                frame: key.frame,
                size,
                onion: onion.map(|(before, after)| Onion { before, after }),
                playing: key.playing,
                transparent: false,
                in_place: key.in_place.clone(),
                margin: key.margin,
            }));
            self.sync.inflight.insert(id, key.clone());
            self.sync.requested = Some(key);
        }
        self.request_thumbnails();
        self.drive_preview(ctx);
    }

    fn request_thumbnails(&mut self) {
        let doc = self.host.session().doc().document();
        let mut budget = 6;
        let names = doc.library.symbols().map(|s| &s.name).chain(doc.library.bitmaps().map(|b| &b.name));
        for name in names {
            if budget == 0 {
                break;
            }
            if self.ui.thumbs.contains_key(name) || self.ui.thumb_requested.contains(name) {
                continue;
            }
            if name.rsplit_once('/').is_some_and(|(parent, _)| self.ui.collapsed_folders.contains(parent)) {
                continue;
            }
            self.ui.thumb_requested.insert(name.clone());
            self.worker.send(Job::Thumbnail { name: name.clone(), size: 48 });
            budget = usize::saturating_sub(budget, 1);
        }
    }

    /// Forgets everything derived from the previous document (after Open or New).
    pub(crate) fn reset_view_for_new_document(&mut self) {
        let doc = self.host.session().doc().document();
        self.ui.stage.doc_size = egui::vec2(num::f64_to_f32(doc.width), num::f64_to_f32(doc.height));
        self.ui.stage.fit_request = true;
        self.ui.thumbs.clear();
        self.ui.thumb_requested.clear();
        self.ui.collapsed_folders.clear();
        self.ui.collapsed_layers.clear();
        self.ui.library_selected = None;
        self.ui.preview = crate::preview::PreviewState::default();
        self.ui.timeline.selection = anim_ui::timeline::FrameSelection::default();
        self.ui.timeline.selected_layers.clear();
        self.stop_playback();
        self.sync.requested = None;
        self.sync.fingerprint = None;
        self.sync.revision = 0;
    }

    /// True when the stage on screen shows the document as it is now: nothing changed since the last render request
    /// and the worker has answered it.
    fn settled(&self) -> bool {
        let frame = self.host.session().doc().editor.state.frame;
        self.sync.fingerprint.as_ref() == Some(&self.fingerprint())
            && self.sync.shown.is_some()
            && self.sync.shown == self.sync.requested
            && self.sync.requested.as_ref().is_some_and(|k| k.frame == frame)
    }

    fn collect_shots(&mut self, ctx: &Context) {
        // A screenshot asked for by an agent waits until the stage shows what the agent just did (but not for ever).
        let waiting = !self.host.hooks_mut().shots.is_empty();
        let ready = !waiting || self.settled() || self.shot_waited >= MAX_SHOT_WAIT_FRAMES;
        if waiting && !ready {
            self.shot_waited = self.shot_waited.saturating_add(1);
            ctx.request_repaint();
        }
        let pending: Vec<PendingShot> = if ready { std::mem::take(&mut self.host.hooks_mut().shots) } else { Vec::new() };
        if ready {
            self.shot_waited = 0;
        }
        if !pending.is_empty() {
            let tag = self.next_shot;
            self.next_shot = self.next_shot.saturating_add(1);
            self.mcp_shots.insert(tag, pending);
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(tag)));
            ctx.request_repaint();
        }
        let events: Vec<(UserData, ColorImage)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => Some((user_data.clone(), (**image).clone())),
                    _ => None,
                })
                .collect()
        });
        for (data, image) in events {
            let tag = data.data.as_ref().and_then(|d| d.downcast_ref::<u64>().copied());
            match tag {
                Some(0) | None => self.finish_cli_shot(&image),
                Some(tag) => self.finish_mcp_shots(tag, &image, ctx.pixels_per_point()),
            }
        }
    }

    fn finish_mcp_shots(&mut self, tag: u64, image: &ColorImage, ppp: f32) {
        let Some(shots) = self.mcp_shots.remove(&tag) else { return };
        for shot in shots {
            let rect = shot.request.panel.as_deref().and_then(|p| self.ui.panel_rects.get(&p.to_lowercase()));
            let shown = crop(image, rect, ppp);
            let [w, h] = shown.size;
            let rgba: Vec<u8> = shown.pixels.iter().flat_map(egui::Color32::to_array).collect();
            let result = u32::try_from(w)
                .ok()
                .zip(u32::try_from(h).ok())
                .ok_or_else(|| anim_mcp::ControlError::failed("the screenshot is too large"))
                .and_then(|(w, h)| Reply::value(json!({ "width": w, "height": h, "panel": shot.request.panel })).with_png_rgba(w, h, &rgba));
            let result = result.and_then(|reply| {
                if let (Some(path), Some(img)) = (&shot.request.save_to, reply.images.first()) {
                    anim_mcp::save_image(path, img)?;
                }
                Ok(reply)
            });
            shot.ticket.finish(result);
        }
    }

    fn finish_cli_shot(&mut self, image: &ColorImage) {
        let Some(run) = self.shot.take() else { return };
        let rect = run.plan.panel.as_deref().and_then(|p| self.ui.panel_rects.get(&p.to_lowercase()));
        let shown = crop(image, rect, self.ctx.pixels_per_point());
        let [w, h] = shown.size;
        let rgba: Vec<u8> = shown.pixels.iter().flat_map(egui::Color32::to_array).collect();
        let written = u32::try_from(w)
            .ok()
            .zip(u32::try_from(h).ok())
            .ok_or_else(|| "screenshot too large".to_owned())
            .and_then(|(w, h)| anim_export::encode_png(w, h, &rgba).map_err(|e| e.to_string()))
            .and_then(|png| std::fs::write(&run.plan.out, png).map_err(|e| format!("{}: {e}", run.plan.out.display())));
        match &written {
            Ok(()) => eprintln!("anicraft: wrote {} ({w}x{h})", run.plan.out.display()),
            Err(e) => eprintln!("anicraft: screenshot failed: {e}"),
        }
        if let Some(outcome) = &self.outcome {
            let _ = outcome.send(written);
        }
        self.closing = true;
    }

    /// The `--screenshot` state machine: wait until the stage shows the requested frame, then ask for the capture.
    fn drive_shot(&mut self, ctx: &Context) {
        let ready = self.sync.shown.is_some()
            && self.sync.shown == self.sync.requested
            && self.sync.inflight.is_empty()
            && self.fonts
            && self.preview_settled()
            && self.sync.last_error.is_none();
        let Some(run) = &mut self.shot else { return };
        match &mut run.stage {
            ShotStage::Settling { frames } => {
                if ready {
                    *frames = frames.saturating_add(1);
                } else {
                    *frames = 0;
                }
                // A few settled frames so layout, fonts, thumbnails and the dock have all been drawn.
                if *frames >= 8 {
                    run.stage = ShotStage::Requested;
                    ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(0_u64)));
                }
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            ShotStage::Requested => ctx.request_repaint_after(Duration::from_millis(30)),
        }
        if self.frames_drawn > 1200 && !self.closing {
            eprintln!("anicraft: screenshot timed out waiting for the stage to render");
            if let Some(outcome) = &self.outcome {
                let _ = outcome.send(Err("timed out".to_owned()));
            }
            self.closing = true;
        }
    }
}
