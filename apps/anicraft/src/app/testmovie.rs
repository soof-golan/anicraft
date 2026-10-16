//! Control > Test Movie: runs the document's main timeline in the preview runtime ([`anim_script::Player`]).
//!
//! While a test movie runs, the stage shows the frame the player is on, the pointer drives the buttons' event
//! handlers instead of the tools, and `trace` output goes to the Output panel. Ending it puts the editor back on the
//! timeline and frame it was on.

use std::time::{Duration, Instant};

use anim_core::{Point, num};
use anim_script::Player;
use anim_ui::stage::{PointerPhase, StagePointer};
use egui::Context;

use super::AniApp;
use crate::docs::Level;
use crate::state::TestMovie;

impl AniApp {
    /// Starts the test movie, or ends the one that is running.
    pub(crate) fn toggle_test_movie(&mut self) {
        if self.ui.test_movie.is_some() {
            self.end_test_movie();
        } else {
            self.start_test_movie();
        }
    }

    fn start_test_movie(&mut self) {
        self.stop_playback();
        let (mut player, saved) = {
            let doc = self.host.session().doc();
            (Player::new(doc.document()), (doc.editor.state.timeline.clone(), doc.editor.state.frame))
        };
        let version = player.version();
        for (scene, layer, frame, line, message) in player.diagnostics() {
            let place = format!("scene {}, layer {}, frame {}, line {line}", scene.saturating_add(1), layer.saturating_add(1), frame.saturating_add(1));
            self.ui.docs.log(Level::Warning, format!("Test Movie: {message} ({place})"));
        }
        player.start(self.host.session().doc().document());
        self.ui.docs.log(Level::Info, format!("Test Movie started ({version:?}). Supported: timeline control, frame labels, button event handlers, trace."));
        self.ui.test_movie = Some(TestMovie { player, last_tick: Instant::now(), saved, hover: None, pressed: None });
        self.sync_test_movie();
        if !self.ui.layout.contains(&anim_ui::PanelId::Output) {
            let _ = self.open_doc_panel(anim_ui::PanelId::Output);
        }
        self.status("Test Movie running; press Escape to end it");
    }

    /// Ends the test movie and restores the editor's timeline and frame.
    pub(crate) fn end_test_movie(&mut self) {
        let Some(movie) = self.ui.test_movie.take() else { return };
        let (timeline, frame) = movie.saved;
        let state = &mut self.host.session_mut().doc_mut().editor.state;
        state.timeline = timeline;
        state.frame = frame;
        self.ui.docs.log(Level::Info, "Test Movie ended.");
        self.status("Test Movie ended");
    }

    /// Moves the playhead of the running movie on by the frames the clock says have passed.
    pub(crate) fn tick_test_movie(&mut self, ctx: &Context) {
        let fps = self.host.session().doc().document().frame_rate.max(1.0);
        let Some(movie) = self.ui.test_movie.as_mut() else { return };
        let period = Duration::from_secs_f64(1.0 / fps);
        let behind = Instant::now().saturating_duration_since(movie.last_tick);
        let due = u32::try_from(num::floor_f64_to_i32(behind.as_secs_f64() * fps)).unwrap_or(0).min(4);
        if due > 0 {
            let doc = self.host.session().doc().document();
            for _ in 0..due {
                movie.player.advance(doc);
            }
            movie.last_tick = Instant::now();
        }
        self.sync_test_movie();
        ctx.request_repaint_after(period);
    }

    /// Shows the player's frame on the stage and moves what it wrote into the Output panel.
    fn sync_test_movie(&mut self) {
        let Some(movie) = self.ui.test_movie.as_mut() else { return };
        let (scene, frame) = movie.player.position();
        let lines = movie.player.take_output();
        let state = &mut self.host.session_mut().doc_mut().editor.state;
        state.timeline = anim_scene::TimelineId::Scene(scene);
        state.frame = frame;
        state.selection = anicraft_tools::Selection::default();
        for line in lines {
            let level = if line.starts_with("Error") { Level::Error } else { Level::Info };
            self.ui.docs.log(level, line);
        }
    }

    /// The name of the symbol instance under `at` on the stage, if it has one.
    fn instance_name_at(&self, at: Point) -> Option<String> {
        let doc = self.host.session().doc();
        let pick = doc.editor.with_context(|ctx| anicraft_tools::pick(ctx, at))?;
        let resolved = anicraft_tools::resolve_ref(doc.document(), &doc.editor.state.timeline, &pick.element)?;
        match resolved.element {
            anim_scene::Element::SymbolInstance(i) => i.name.clone(),
            _ => None,
        }
    }

    fn send_event(&mut self, name: Option<&str>, event: &str) {
        let Some(name) = name else { return };
        let Some(movie) = self.ui.test_movie.as_mut() else { return };
        movie.player.dispatch(self.host.session().doc().document(), name, event);
        self.sync_test_movie();
    }

    /// Pointer input on the stage while the movie runs: hover, press and click reach the buttons' handlers.
    pub(super) fn test_movie_pointer(&mut self, p: &StagePointer) {
        let name = self.instance_name_at(Point::new(f64::from(p.pos.x), f64::from(p.pos.y)));
        match p.phase {
            PointerPhase::Move | PointerPhase::Drag => {
                let previous = self.ui.test_movie.as_ref().and_then(|m| m.hover.clone());
                if previous != name {
                    self.send_event(previous.as_deref(), "rollOut");
                    self.send_event(previous.as_deref(), "mouseOut");
                    self.send_event(name.as_deref(), "rollOver");
                    self.send_event(name.as_deref(), "mouseOver");
                    if let Some(m) = self.ui.test_movie.as_mut() {
                        m.hover = name;
                    }
                }
            }
            PointerPhase::Down => {
                self.send_event(name.as_deref(), "mouseDown");
                if let Some(m) = self.ui.test_movie.as_mut() {
                    m.pressed = name;
                }
            }
            PointerPhase::Up => {
                self.send_event(name.as_deref(), "mouseUp");
                let pressed = self.ui.test_movie.as_mut().and_then(|m| m.pressed.take());
                if pressed.is_some() && pressed == name {
                    self.send_event(name.as_deref(), "click");
                }
            }
            PointerPhase::Click => {
                self.send_event(name.as_deref(), "mouseDown");
                self.send_event(name.as_deref(), "mouseUp");
                self.send_event(name.as_deref(), "click");
            }
            PointerPhase::DoubleClick => self.send_event(name.as_deref(), "doubleClick"),
        }
    }
}
