//! The person's own templates: ordinary `.fla` files in `<data folder>/templates/<Category>/<Name>.fla`. A document made
//! from one starts untitled, so saving it never overwrites the template.

use std::path::{Path, PathBuf};

use anicraft_cli::mcp::AniDoc;
use anicraft_cli::project::Project;

/// A template on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTemplate {
    pub category: String,
    pub name: String,
    pub path: PathBuf,
}

/// Category used when the person names none.
pub const DEFAULT_CATEGORY: &str = "My Templates";

/// A file or folder name made from free text: no separators, no leading dots, bounded length.
#[must_use]
pub fn clean_name(text: &str) -> String {
    let cleaned: String =
        text.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    cleaned.trim().trim_start_matches('.').chars().take(80).collect::<String>().trim().to_owned()
}

/// Lists the templates under `dir`, by category then name.
#[must_use]
pub fn list(dir: &Path) -> Vec<UserTemplate> {
    let mut out = Vec::new();
    let Ok(categories) = std::fs::read_dir(dir) else { return out };
    for category in categories.flatten() {
        let path = category.path();
        if !path.is_dir() {
            continue;
        }
        let category_name = category.file_name().to_string_lossy().into_owned();
        let Ok(files) = std::fs::read_dir(&path) else { continue };
        for file in files.flatten() {
            let file_path = file.path();
            if file_path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fla"))
                && let Some(stem) = file_path.file_stem()
            {
                out.push(UserTemplate { category: category_name.clone(), name: stem.to_string_lossy().into_owned(), path: file_path });
            }
        }
    }
    out.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
    out
}

/// Writes `doc` as a template.
///
/// # Errors
/// When the name is empty or the file cannot be written.
pub fn save(dir: &Path, category: &str, name: &str, doc: &AniDoc) -> Result<PathBuf, String> {
    let name = clean_name(name);
    if name.is_empty() {
        return Err("give the template a name".to_owned());
    }
    let category = clean_name(category);
    let category = if category.is_empty() { DEFAULT_CATEGORY.to_owned() } else { category };
    let folder = dir.join(category);
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let path = folder.join(format!("{name}.fla"));
    let mut file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    crate::files::write_fla(doc, &mut file).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Reads a template as a new, untitled document.
///
/// # Errors
/// When the file cannot be read.
pub fn load(path: &Path) -> Result<AniDoc, String> {
    Project::open(path).map(AniDoc::from).map_err(|e| format!("{}: {e}", path.display()))
}

/// Deletes a template file (and its category folder when that leaves it empty).
///
/// # Errors
/// When the file cannot be removed.
pub fn delete(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = anicraft_cli::mcp::docfile::new_document(&anicraft_cli::mcp::docfile::NewSpec {
            kind: anicraft_cli::mcp::docfile::DocType::Html5Canvas,
            width: 300.0,
            height: 250.0,
            ..Default::default()
        })
        .unwrap();
        doc.document_mut().scenes[0].layers[0].name = "Backdrop".to_owned();
        let path = save(dir.path(), "Ads", "Medium: rectangle/1", &doc).unwrap();
        assert!(path.ends_with("Ads/Medium_ rectangle_1.fla"), "{}", path.display());
        let found = list(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].category.as_str(), found[0].name.as_str()), ("Ads", "Medium_ rectangle_1"));
        let loaded = load(&found[0].path).unwrap();
        assert_eq!(loaded.document().width, 300.0);
        assert_eq!(anicraft_cli::mcp::docfile::doc_type(loaded.document()), anicraft_cli::mcp::docfile::DocType::Html5Canvas);
        assert_eq!(loaded.document().scenes[0].layers[0].name, "Backdrop");
        assert!(save(dir.path(), "x", "  ", &doc).is_err());
        delete(&found[0].path).unwrap();
        assert!(list(dir.path()).is_empty());
        assert!(!dir.path().join("Ads").exists(), "the empty category folder goes too");
    }

    #[test]
    fn names_cannot_escape_the_folder() {
        assert_eq!(clean_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(clean_name("  .hidden "), "hidden");
        assert_eq!(clean_name(""), "");
    }
}
