//! Loading and caching of schema files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use xml_schema::{load_rng, Schema, SchemaDiagnostic};

/// A schema loaded from disk, together with its load diagnostics.
pub struct LoadedSchema {
    pub path: PathBuf,
    pub schema: Schema,
    pub diagnostics: Vec<SchemaDiagnostic>,
}

struct CacheEntry {
    modified: Option<SystemTime>,
    loaded: Arc<LoadedSchema>,
}

/// Caches loaded schemas by path and reloads them when the file changes
/// on disk (cheap mtime check per access).
#[derive(Default)]
pub struct SchemaRepository {
    cache: HashMap<PathBuf, CacheEntry>,
}

impl SchemaRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(&mut self, path: &Path) -> std::io::Result<Arc<LoadedSchema>> {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some(entry) = self.cache.get(path) {
            if entry.modified == modified && modified.is_some() {
                return Ok(entry.loaded.clone());
            }
        }
        let text = std::fs::read_to_string(path)?;
        let result = load_rng(&text);
        let loaded = Arc::new(LoadedSchema {
            path: path.to_path_buf(),
            schema: result.schema,
            diagnostics: result.diagnostics,
        });
        self.cache.insert(
            path.to_path_buf(),
            CacheEntry {
                modified,
                loaded: loaded.clone(),
            },
        );
        Ok(loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_and_reloads_on_change() {
        let dir = std::env::temp_dir().join(format!("xml-repo-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.rng");
        std::fs::write(
            &path,
            r#"<element name="a" xmlns="http://relaxng.org/ns/structure/1.0"><empty/></element>"#,
        )
        .unwrap();

        let mut repo = SchemaRepository::new();
        let first = repo.load(&path).unwrap();
        let second = repo.load(&path).unwrap();
        assert!(Arc::ptr_eq(&first, &second), "cache hit expected");

        // Change the file. The short sleep keeps the test robust on
        // filesystems with coarse mtime granularity.
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(
            &path,
            r#"<element name="b" xmlns="http://relaxng.org/ns/structure/1.0"><empty/></element>"#,
        )
        .unwrap();

        let third = repo.load(&path).unwrap();
        assert!(!Arc::ptr_eq(&first, &third), "reload expected");
        assert!(third.schema.find_element(None, "b").is_some());
    }

    #[test]
    fn missing_file_is_an_error() {
        let mut repo = SchemaRepository::new();
        assert!(repo.load(Path::new("/nonexistent/schema.rng")).is_err());
    }
}
