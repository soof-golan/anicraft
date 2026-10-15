//! `convert`: read an Animate document and write it again in another container.

use std::path::Path;

use anicraft_xfl::WriteOptions;
use serde::Serialize;

use crate::error::{CliError, Result};
use crate::project::Project;

/// The container a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Container {
    /// A zipped `.fla`.
    Fla,
    /// An unzipped XFL folder (any path that does not end in `.fla`, for example `scene.xfl` or `scene-xfl`).
    XflFolder,
}

impl Container {
    /// Chooses by the output path's extension.
    #[must_use]
    pub fn of(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("fla") => Self::Fla,
            _ => Self::XflFolder,
        }
    }
}

/// What a conversion wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConvertReport {
    /// The container written.
    pub container: Container,
    /// Archive entries written.
    pub entries: usize,
    /// Symbols written under `LIBRARY/`.
    pub symbols: usize,
    /// Media entries written under `bin/`.
    pub media_entries: usize,
    /// Uncompressed bytes written.
    pub bytes: u64,
    /// Content that had to be changed or left out.
    pub warnings: Vec<String>,
}

/// Reads `input` and writes it to `output`: a `.fla` file, or an XFL folder for any other name.
///
/// # Errors
/// [`CliError::Usage`] when both paths are the same, otherwise the reader's or the writer's error.
pub fn convert(input: &Path, output: &Path) -> Result<ConvertReport> {
    if input == output {
        return Err(CliError::usage("the input and the output are the same path"));
    }
    let project = Project::open(input)?;
    write_project(&project, output)
}

/// Writes an opened project to `output`.
///
/// # Errors
/// The writer's error.
pub fn write_project(project: &Project, output: &Path) -> Result<ConvertReport> {
    let container = Container::of(output);
    let stem = output.file_stem().and_then(|s| s.to_str()).unwrap_or("document").to_owned();
    let options = WriteOptions { name: stem, ..WriteOptions::default() };
    let loaded = &project.loaded;
    let report = match container {
        Container::Fla => anicraft_xfl::write_fla(output, &loaded.document, &loaded.media, &loaded.sidecars, &options)?,
        Container::XflFolder => anicraft_xfl::write_xfl_folder(output, &loaded.document, &loaded.media, &loaded.sidecars, &options)?,
    };
    Ok(ConvertReport {
        container,
        entries: report.entries,
        symbols: report.symbols,
        media_entries: report.media_entries,
        bytes: report.bytes,
        warnings: report.warnings.iter().map(ToString::to_string).collect(),
    })
}
