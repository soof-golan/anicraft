//! Autosave and crash recovery.
//!
//! Every run keeps a copy of each document that has unsaved changes in `<data folder>/recovery/`, as
//! `<run>-<tab>.fla` with a `<run>-<tab>.json` note of where the document came from. The copies are removed when the
//! document is saved or closed and when the program exits normally. A run touches `alive-<run>` every few seconds;
//! copies of a run whose file has gone quiet were left by a crash, and the next start offers to recover them.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anicraft_cli::mcp::AniDoc;
use serde::{Deserialize, Serialize};

/// A run that has not touched its alive file for this long has crashed.
pub const STALE_AFTER: Duration = Duration::from_secs(20);

/// How often a run touches its alive file.
pub const HEARTBEAT: Duration = Duration::from_secs(5);

/// What is stored beside an autosaved document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The file the document was opened from or last saved to.
    pub original: Option<String>,
    /// The name the tab showed.
    pub title: String,
    /// Seconds since the Unix epoch when the copy was written.
    pub saved_at: u64,
}

/// A copy a crashed run left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub id: String,
    pub note: Note,
    pub fla: PathBuf,
}

/// The recovery folder of one run.
#[derive(Debug, Clone)]
pub struct Recovery {
    dir: PathBuf,
    run: String,
}

fn now_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Recovery {
    /// A recovery area in `dir` for the run called `run` (the process id and start time make it unique).
    #[must_use]
    pub fn new(dir: PathBuf, run: &str) -> Self {
        Self { dir, run: run.to_owned() }
    }

    /// A name for this process's run.
    #[must_use]
    pub fn run_name() -> String {
        format!("{}-{}", std::process::id(), now_seconds())
    }

    fn alive_path(&self) -> PathBuf {
        self.dir.join(format!("alive-{}", self.run))
    }

    /// The id of tab `tab` in this run.
    #[must_use]
    pub fn id_for(&self, tab: u64) -> String {
        format!("{}-{tab}", self.run)
    }

    /// Tells other runs this one is alive.
    pub fn heartbeat(&self) {
        if std::fs::create_dir_all(&self.dir).is_ok() {
            let _ = std::fs::write(self.alive_path(), now_seconds().to_string());
        }
    }

    /// Writes the autosaved copy of a document.
    ///
    /// # Errors
    /// When the folder or a file cannot be written.
    pub fn write(&self, id: &str, doc: &AniDoc, original: Option<&Path>, title: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let fla = self.dir.join(format!("{id}.fla"));
        let mut file = std::fs::File::create(&fla).map_err(|e| format!("{}: {e}", fla.display()))?;
        crate::files::write_fla(doc, &mut file).map_err(|e| format!("{}: {e}", fla.display()))?;
        let note = Note { original: original.map(|p| p.display().to_string()), title: title.to_owned(), saved_at: now_seconds() };
        let text = serde_json::to_string(&note).map_err(|e| e.to_string())?;
        let json = self.dir.join(format!("{id}.json"));
        std::fs::write(&json, text).map_err(|e| format!("{}: {e}", json.display()))
    }

    /// Removes the copy of one document.
    pub fn remove(&self, id: &str) {
        let _ = std::fs::remove_file(self.dir.join(format!("{id}.fla")));
        let _ = std::fs::remove_file(self.dir.join(format!("{id}.json")));
    }

    /// Removes every copy of this run and its alive file (a normal exit).
    pub fn finish(&self) {
        let prefix = format!("{}-", self.run);
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(&prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let _ = std::fs::remove_file(self.alive_path());
    }

    /// Copies left by runs that are no longer alive.
    #[must_use]
    pub fn orphans(&self) -> Vec<Orphan> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return out };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            let Some(run) = id.rsplit_once('-').map(|(run, _)| run.to_owned()) else { continue };
            if run == self.run || self.run_is_alive(&run) {
                continue;
            }
            let fla = self.dir.join(format!("{id}.fla"));
            let note = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Note>(&t).ok());
            if let (Some(note), true) = (note, fla.is_file()) {
                out.push(Orphan { id, note, fla });
            } else {
                // A half-written copy: nothing to offer.
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_file(&fla);
            }
        }
        out.sort_by(|a, b| a.note.saved_at.cmp(&b.note.saved_at).then_with(|| a.id.cmp(&b.id)));
        out
    }

    fn run_is_alive(&self, run: &str) -> bool {
        std::fs::metadata(self.dir.join(format!("alive-{run}"))).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age < STALE_AFTER))
    }

    /// Deletes a stale run's alive file once none of its copies remain.
    pub fn tidy(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        for name in &names {
            if let Some(run) = name.strip_prefix("alive-")
                && run != self.run
                && !self.run_is_alive(run)
                && !names.iter().any(|n| n.starts_with(&format!("{run}-")))
            {
                let _ = std::fs::remove_file(self.dir.join(name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crashed_run_is_offered_and_a_live_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = AniDoc::default();
        doc.document_mut().width = 321.0;
        let crashed = Recovery::new(dir.path().to_path_buf(), "111-1");
        crashed.write(&crashed.id_for(2), &doc, Some(Path::new("/work/a.fla")), "a.fla").unwrap();
        crashed.write(&crashed.id_for(3), &doc, None, "Untitled").unwrap();
        // No alive file at all: the run is dead.
        let me = Recovery::new(dir.path().to_path_buf(), "222-5");
        let found = me.orphans();
        assert_eq!(found.len(), 2);
        let with_original = found.iter().find(|o| o.note.original.is_some()).unwrap();
        assert_eq!(with_original.note.original.as_deref(), Some("/work/a.fla"));
        let loaded = crate::templates::load(&with_original.fla).unwrap();
        assert_eq!(loaded.document().width, 321.0);
        // The same run alive: nothing is offered.
        crashed.heartbeat();
        assert!(me.orphans().is_empty());
        // A run does not offer its own copies, and a clean exit removes them.
        me.write(&me.id_for(1), &doc, None, "Untitled").unwrap();
        assert!(me.orphans().is_empty());
        me.heartbeat();
        me.finish();
        assert!(!dir.path().join("222-5-1.fla").exists());
        assert!(!dir.path().join("alive-222-5").exists());
    }

    #[test]
    fn half_written_copies_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("9-9-1.json"), "{\"original\":null,\"title\":\"x\",\"saved_at\":1}").unwrap();
        let me = Recovery::new(dir.path().to_path_buf(), "1-1");
        assert!(me.orphans().is_empty());
        assert!(!dir.path().join("9-9-1.json").exists());
    }

    #[test]
    fn removing_a_copy_removes_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::new(dir.path().to_path_buf(), "7-7");
        r.write(&r.id_for(1), &AniDoc::default(), None, "t").unwrap();
        assert!(dir.path().join("7-7-1.fla").exists() && dir.path().join("7-7-1.json").exists());
        r.remove(&r.id_for(1));
        assert!(!dir.path().join("7-7-1.fla").exists() && !dir.path().join("7-7-1.json").exists());
    }
}
