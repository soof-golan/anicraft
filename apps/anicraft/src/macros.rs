//! Saved commands.
//!
//! Steps of the History panel kept under a name and run again from the Commands menu. A command is a
//! JSON list of registry command ids with their parameters, in `<data folder>/commands/<name>.json`.

use std::path::{Path, PathBuf};

use crate::docs::Step;

fn file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.json", crate::templates::clean_name(name)))
}

/// The names of the saved commands, sorted.
#[must_use]
pub fn list(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Keeps `steps` as the command `name`.
///
/// # Errors
/// When the name is empty, there is nothing to save, or the file cannot be written.
pub fn save(dir: &Path, name: &str, steps: &[Step]) -> Result<(), String> {
    if crate::templates::clean_name(name).is_empty() {
        return Err("give the command a name".to_owned());
    }
    if steps.is_empty() {
        return Err("none of the selected steps can be saved: pointer gestures are not commands".to_owned());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(steps).map_err(|e| e.to_string())?;
    let path = file(dir, name);
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads a saved command.
///
/// # Errors
/// When the file is missing or damaged.
pub fn load(dir: &Path, name: &str) -> Result<Vec<Step>, String> {
    let path = file(dir, name);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Deletes a saved command.
///
/// # Errors
/// When the file cannot be removed.
pub fn delete(dir: &Path, name: &str) -> Result<(), String> {
    let path = file(dir, name);
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn save_list_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        let steps = vec![Step { id: "modify.flip".to_owned(), params: json!({ "axis": "horizontal" }) }];
        save(dir.path(), "Mirror it", &steps).unwrap();
        assert_eq!(list(dir.path()), ["Mirror it"]);
        assert_eq!(load(dir.path(), "Mirror it").unwrap(), steps);
        assert!(save(dir.path(), "", &steps).is_err());
        assert!(save(dir.path(), "x", &[]).is_err());
        delete(dir.path(), "Mirror it").unwrap();
        assert!(list(dir.path()).is_empty());
        assert!(load(dir.path(), "Mirror it").is_err());
    }
}
