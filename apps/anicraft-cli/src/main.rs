//! `anicraft-cli`: inspect, render, export and convert Animate documents, and serve them over MCP.

use clap::Parser as _;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    anicraft_cli::cli::run(anicraft_cli::cli::Cli::parse())
}
