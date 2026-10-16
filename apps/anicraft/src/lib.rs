//! `AniCraft`, the desktop editor.
//!
//! * [`app`]: the window (an `eframe` app) whose state is an `anim-session` session of an `anicraft-cli` document;
//! * [`commands`]: the Modify and Insert commands on top of the document commands and tool commands that already
//!   exist, so every menu item is also an MCP tool;
//! * [`worker`]: the render actor (latest request wins, never blocks the UI);
//! * [`mcp`]: the running editor as an MCP server (stdio, and the loopback control channel for `anim-mcp`);
//! * [`smoke`]: the headless check that backs `--smoke`.
//!
//! The UI is a function of the document and the view state: panels report events, [`app`] turns them into commands.

#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::struct_excessive_bools,
    clippy::similar_names,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::implicit_hasher,
    clippy::chunks_exact_to_as_chunks,
    reason = "immediate-mode UI code: long match-driven functions, view models that are flag sets, and pixel loops over RGBA quads"
)]
#![cfg_attr(test, allow(clippy::arithmetic_side_effects, clippy::float_cmp, clippy::assert_is_empty, reason = "test code"))]

pub mod app;
pub mod audio;
pub mod commands;
pub mod files;
pub mod mcp;
pub mod menus;
pub mod preview;
pub mod props;
pub mod smoke;
pub mod state;
pub mod view;
pub mod worker;

pub use app::{AniApp, Options, ShotPlan};

/// Text as the widgets must be given it: Hebrew and Arabic runs reordered for display.
#[must_use]
pub fn text(s: &str) -> String {
    anim_ui::theme::visual(s).into_owned()
}

/// Opens the window and runs until it is closed.
///
/// # Errors
/// When the document cannot be read, a transport cannot start, the window cannot be created, or a requested
/// screenshot could not be taken.
pub fn run(options: Options) -> color_eyre::Result<()> {
    let (outcome_tx, outcome_rx) = std::sync::mpsc::channel();
    let (width, height) = options.window.unwrap_or((1440.0, 900.0));
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(egui::vec2(width, height)).with_min_inner_size([900.0, 560.0]).with_title("AniCraft"),
        renderer: eframe::Renderer::Wgpu,
        ..eframe::NativeOptions::default()
    };
    let wants_shot = options.shot.is_some();
    eframe::run_native(
        "AniCraft",
        native,
        Box::new(move |cc| {
            let app = AniApp::new(&cc.egui_ctx, &options, Some(outcome_tx))?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    if wants_shot {
        match outcome_rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(color_eyre::eyre::eyre!("screenshot failed: {e}")),
            Err(_) => return Err(color_eyre::eyre::eyre!("the window closed before the screenshot was taken")),
        }
    }
    Ok(())
}
