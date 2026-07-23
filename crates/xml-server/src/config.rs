//! Server configuration. Read from the client's `initializationOptions`
//! and from `workspace/didChangeConfiguration`.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub diagnostics: DiagnosticsConfig,
    pub schema: SchemaConfig,
}

/// Schema association: catalogs and language profiles. Paths may be
/// relative; the server resolves them against the workspace root.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SchemaConfig {
    /// OASIS XML catalog files (namespace → schema location).
    pub catalogs: Vec<String>,
    /// Explicitly configured languages with activation rules.
    pub languages: Vec<LanguageEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LanguageEntry {
    /// Display name; defaults to the schema file name.
    pub name: Option<String>,
    /// Path to the RELAX NG schema (.rng).
    pub schema: String,
    /// Activation rules — any matching rule activates the language, the
    /// most specific match wins.
    pub namespace: Option<String>,
    pub root_element: Option<String>,
    pub file_names: Vec<String>,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DiagnosticsConfig {
    /// Turn diagnostics off entirely.
    pub enable: bool,
    /// Upper bound per document so broken huge files do not flood the
    /// editor.
    pub max_count: usize,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            enable: true,
            max_count: 1000,
        }
    }
}

impl ServerConfig {
    /// Reads the configuration leniently from a JSON value: unknown fields
    /// are ignored, invalid values fall back to the defaults. Accepts both
    /// `{ "xml": { … } }` and the options directly at the top level.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let scoped = value.get("xml").unwrap_or(value);
        serde_json::from_value(scoped.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults() {
        let config = ServerConfig::default();
        assert!(config.diagnostics.enable);
        assert_eq!(config.diagnostics.max_count, 1000);
    }

    #[test]
    fn from_json_scoped_and_flat() {
        let scoped = ServerConfig::from_json(&json!({
            "xml": { "diagnostics": { "maxCount": 5 } }
        }));
        assert_eq!(scoped.diagnostics.max_count, 5);

        let flat = ServerConfig::from_json(&json!({
            "diagnostics": { "enable": false }
        }));
        assert!(!flat.diagnostics.enable);
    }

    #[test]
    fn from_json_tolerates_garbage() {
        let config = ServerConfig::from_json(&serde_json::json!("not an object"));
        assert_eq!(config, ServerConfig::default());
    }

    #[test]
    fn schema_config() {
        let config = ServerConfig::from_json(&json!({
            "schema": {
                "catalogs": ["catalog.xml"],
                "languages": [{
                    "name": "speedata",
                    "schema": "layoutschema-en.rng",
                    "namespace": "urn:speedata.de:2009/publisher/en",
                    "rootElement": "Layout",
                    "fileNames": ["layout.xml"]
                }]
            }
        }));
        assert_eq!(config.schema.catalogs, vec!["catalog.xml"]);
        let language = &config.schema.languages[0];
        assert_eq!(language.name.as_deref(), Some("speedata"));
        assert_eq!(language.root_element.as_deref(), Some("Layout"));
        assert_eq!(language.file_names, vec!["layout.xml"]);
    }
}
