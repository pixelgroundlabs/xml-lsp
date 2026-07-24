//! The language registry: resolves a document to its schema.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use xml_ast::Document;

use crate::catalog::{read_catalog, resolve_location, CatalogError};
use crate::xml_model::relaxng_model_href;

/// Activation rules of a language profile. Every matching criterion adds
/// to the score; the profile with the highest score wins. Specific
/// signals (namespace) outweigh generic ones (file extension).
#[derive(Debug, Clone, Default)]
pub struct Activation {
    pub namespace: Option<String>,
    pub root_element: Option<String>,
    pub file_names: Vec<String>,
    pub extensions: Vec<String>,
}

impl Activation {
    fn score(
        &self,
        namespace: Option<&str>,
        root_local: Option<&str>,
        file_name: Option<&str>,
    ) -> u32 {
        let mut score = 0;
        if let Some(want) = &self.namespace {
            if namespace == Some(want.as_str()) {
                score += 8;
            }
        }
        if let Some(want) = &self.root_element {
            if root_local == Some(want.as_str()) {
                score += 4;
            }
        }
        if let Some(file_name) = file_name {
            if self.file_names.iter().any(|name| name == file_name) {
                score += 2;
            }
            if self
                .extensions
                .iter()
                .any(|ext| file_name.ends_with(ext.as_str()))
            {
                score += 1;
            }
        }
        score
    }
}

/// A configured XML language: activation rules plus the schema to use.
#[derive(Debug, Clone)]
pub struct LanguageProfile {
    pub name: String,
    pub schema: PathBuf,
    pub activation: Activation,
}

/// Where a schema resolution came from (useful for logging/debugging).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionOrigin {
    /// `<?xml-model?>` in the document itself.
    XmlModelPi,
    /// A configured language profile.
    Profile,
    /// A namespace entry in an XML catalog.
    Catalog,
}

#[derive(Debug, Clone)]
pub struct ResolvedSchema {
    /// Name of the matched profile, if any.
    pub language: Option<String>,
    pub schema_path: PathBuf,
    pub origin: ResolutionOrigin,
}

/// Holds all known language profiles and catalog mappings.
#[derive(Debug, Default)]
pub struct LanguageRegistry {
    profiles: Vec<LanguageProfile>,
    catalog: HashMap<String, PathBuf>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_profile(&mut self, profile: LanguageProfile) {
        self.profiles.push(profile);
    }

    /// Loads an OASIS catalog and merges its `uri` entries. Later entries
    /// win over earlier ones with the same name.
    pub fn load_catalog(&mut self, path: &Path) -> Result<usize, CatalogError> {
        let entries = read_catalog(path)?;
        let count = entries.len();
        self.catalog.extend(entries);
        Ok(count)
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.catalog.is_empty()
    }

    pub fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    pub fn catalog_count(&self) -> usize {
        self.catalog.len()
    }

    /// Resolves the schema for a document. `doc_path` is the document's
    /// file location (for relative `xml-model` hrefs and file-name rules).
    pub fn resolve(
        &self,
        doc: &Document,
        text: &str,
        doc_path: Option<&Path>,
    ) -> Option<ResolvedSchema> {
        // 1. An xml-model PI is the author's explicit choice.
        if let Some(href) = relaxng_model_href(doc, text) {
            let base = doc_path.and_then(Path::parent).unwrap_or(Path::new(""));
            return Some(ResolvedSchema {
                language: None,
                schema_path: resolve_location(&href, base),
                origin: ResolutionOrigin::XmlModelPi,
            });
        }

        let root = doc.root_element().and_then(|id| doc.element(id));
        let namespace = root.and_then(|el| el.name.namespace.as_deref());
        let root_local = root.map(|el| el.name.local.as_str());
        let file_name = doc_path
            .and_then(Path::file_name)
            .and_then(|name| name.to_str());

        // 2. The best-scoring configured profile.
        let best = self
            .profiles
            .iter()
            .map(|profile| {
                (
                    profile.activation.score(namespace, root_local, file_name),
                    profile,
                )
            })
            .filter(|(score, _)| *score > 0)
            .max_by_key(|(score, _)| *score);
        if let Some((_, profile)) = best {
            return Some(ResolvedSchema {
                language: Some(profile.name.clone()),
                schema_path: profile.schema.clone(),
                origin: ResolutionOrigin::Profile,
            });
        }

        // 3. Catalog lookup by root namespace.
        let namespace = namespace?;
        self.catalog.get(namespace).map(|path| ResolvedSchema {
            language: None,
            schema_path: path.clone(),
            origin: ResolutionOrigin::Catalog,
        })
    }
}
