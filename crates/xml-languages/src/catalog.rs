//! Minimal OASIS XML catalog support: `uri` entries map a namespace (or
//! any URI) to a schema location, as used by the speedata catalogs.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use xml_ast::{unescape, Document, NodeId};

const CATALOG_NAMESPACE: &str = "urn:oasis:names:tc:entity:xmlns:xml:catalog";

#[derive(Debug)]
pub enum CatalogError {
    Io(std::io::Error),
    /// The file parsed, but contained no usable `uri` entries.
    NoEntries,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogError::Io(err) => write!(f, "cannot read catalog: {err}"),
            CatalogError::NoEntries => write!(f, "catalog contains no <uri> entries"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Reads `uri` entries from an OASIS catalog file. Relative locations are
/// resolved against the catalog's directory.
pub(crate) fn read_catalog(path: &Path) -> Result<HashMap<String, PathBuf>, CatalogError> {
    let text = std::fs::read_to_string(path).map_err(CatalogError::Io)?;
    let parse = xml_parser::parse(&text);
    let base = path.parent().unwrap_or(Path::new(""));

    let mut entries = HashMap::new();
    if let Some(root) = parse.document.root_element() {
        collect_uris(&parse.document, root, base, &mut entries);
    }
    if entries.is_empty() {
        return Err(CatalogError::NoEntries);
    }
    Ok(entries)
}

fn collect_uris(doc: &Document, node: NodeId, base: &Path, entries: &mut HashMap<String, PathBuf>) {
    for &child in doc.children(node) {
        let Some(element) = doc.element(child) else {
            continue;
        };
        if element.name.namespace.as_deref() != Some(CATALOG_NAMESPACE) {
            continue;
        }
        match element.name.local.as_str() {
            "uri" => {
                let name = element.attribute(None, "name");
                let uri = element.attribute(None, "uri");
                if let (Some(name), Some(uri)) = (name, uri) {
                    let name = unescape(&name.value).into_owned();
                    let uri = unescape(&uri.value).into_owned();
                    entries.insert(name, resolve_location(&uri, base));
                }
            }
            // Groups just nest further entries.
            "group" => collect_uris(doc, child, base, entries),
            _ => {}
        }
    }
}

/// Resolves a catalog/xml-model location against a base directory.
/// Absolute paths and `file://` URLs pass through; remote URLs are not
/// supported and yield the path verbatim (the caller will report the
/// missing file).
pub(crate) fn resolve_location(location: &str, base: &Path) -> PathBuf {
    if let Some(stripped) = location.strip_prefix("file://") {
        return PathBuf::from(stripped);
    }
    let path = Path::new(location);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("xml-languages-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_uri_entries_with_relative_resolution() {
        let dir = temp_dir("catalog");
        let catalog = dir.join("catalog.xml");
        std::fs::write(
            &catalog,
            r#"<catalog xmlns="urn:oasis:names:tc:entity:xmlns:xml:catalog">
  <uri name="urn:example:one" uri="one.rng"/>
  <group>
    <uri name="urn:example:two" uri="/absolute/two.rng"/>
  </group>
</catalog>"#,
        )
        .unwrap();

        let entries = read_catalog(&catalog).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries["urn:example:one"], dir.join("one.rng"));
        assert_eq!(
            entries["urn:example:two"],
            PathBuf::from("/absolute/two.rng")
        );
    }

    #[test]
    fn empty_catalog_is_an_error() {
        let dir = temp_dir("empty");
        let catalog = dir.join("catalog.xml");
        std::fs::write(
            &catalog,
            "<catalog xmlns='urn:oasis:names:tc:entity:xmlns:xml:catalog'/>",
        )
        .unwrap();
        assert!(matches!(
            read_catalog(&catalog),
            Err(CatalogError::NoEntries)
        ));
    }
}
