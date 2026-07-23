//! The LSP backend: implements tower-lsp's `LanguageServer` interface and
//! wires up the document cache, parser, language registry, and
//! validation.

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    Diagnostic, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, InitializeParams, InitializeResult,
    InitializedParams, MessageType, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer};
use xml_languages::{Activation, LanguageProfile, LanguageRegistry, SchemaRepository};
use xml_parser::Severity;

use crate::config::ServerConfig;
use crate::diagnostics::{parse_to_lsp, server_notice, validation_to_lsp};
use crate::documents::DocumentStore;

pub struct Backend {
    client: Client,
    store: DocumentStore,
    config: RwLock<ServerConfig>,
    registry: RwLock<LanguageRegistry>,
    schemas: Mutex<SchemaRepository>,
    workspace_root: RwLock<Option<PathBuf>>,
    /// Warnings collected before the client is ready for messages.
    startup_messages: Mutex<Vec<String>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            store: DocumentStore::default(),
            config: RwLock::new(ServerConfig::default()),
            registry: RwLock::new(LanguageRegistry::new()),
            schemas: Mutex::new(SchemaRepository::new()),
            workspace_root: RwLock::new(None),
            startup_messages: Mutex::new(Vec::new()),
        }
    }

    /// Access to the document cache (also used by integration tests).
    pub fn store(&self) -> &DocumentStore {
        &self.store
    }

    pub fn config(&self) -> ServerConfig {
        self.config
            .read()
            .expect("config lock is never poisoned")
            .clone()
    }

    fn set_config(&self, config: ServerConfig) {
        *self.config.write().expect("config lock is never poisoned") = config;
    }

    fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace_root
            .read()
            .expect("root lock is never poisoned")
            .clone()
    }

    /// Rebuilds the language registry from the current configuration.
    /// Returns human-readable problems (unreadable catalogs, …).
    fn rebuild_registry(&self) -> Vec<String> {
        let config = self.config();
        let root = self.workspace_root();
        let resolve = |path: &str| -> PathBuf {
            let path = PathBuf::from(path);
            match (&root, path.is_absolute()) {
                (Some(root), false) => root.join(path),
                _ => path,
            }
        };

        let mut warnings = Vec::new();
        let mut registry = LanguageRegistry::new();
        for language in &config.schema.languages {
            let schema = resolve(&language.schema);
            let name = language.name.clone().unwrap_or_else(|| {
                schema
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| language.schema.clone())
            });
            registry.add_profile(LanguageProfile {
                name,
                schema,
                activation: Activation {
                    namespace: language.namespace.clone(),
                    root_element: language.root_element.clone(),
                    file_names: language.file_names.clone(),
                    extensions: language.extensions.clone(),
                },
            });
        }
        for catalog in &config.schema.catalogs {
            let path = resolve(catalog);
            if let Err(err) = registry.load_catalog(&path) {
                warnings.push(format!("catalog {}: {err}", path.display()));
            }
        }
        *self
            .registry
            .write()
            .expect("registry lock is never poisoned") = registry;
        warnings
    }

    /// Computes the full diagnostics set for a document: parser
    /// diagnostics plus schema validation, if a schema applies.
    pub fn compute_diagnostics(&self, uri: &Url) -> Option<(i32, Vec<Diagnostic>)> {
        let config = self.config();
        let doc = self.store.get(uri)?;
        if !config.diagnostics.enable {
            return Some((doc.version, Vec::new()));
        }

        let mut diagnostics = parse_to_lsp(&doc.text, &doc.line_index, &doc.parse.diagnostics);

        let path = uri.to_file_path().ok();
        let resolved = self
            .registry
            .read()
            .expect("registry lock is never poisoned")
            .resolve(&doc.parse.document, &doc.text, path.as_deref());
        if let Some(resolved) = resolved {
            let loaded = self
                .schemas
                .lock()
                .expect("schema lock is never poisoned")
                .load(&resolved.schema_path);
            match loaded {
                Ok(loaded) => {
                    let schema_errors = loaded
                        .diagnostics
                        .iter()
                        .filter(|d| d.severity == Severity::Error)
                        .count();
                    if schema_errors > 0 {
                        diagnostics.push(server_notice(
                            &doc.text,
                            &doc.line_index,
                            Severity::Warning,
                            "schema-load",
                            &format!(
                                "schema {} has {schema_errors} problem(s); validation may be incomplete",
                                loaded.path.display()
                            ),
                        ));
                    }
                    let validation =
                        xml_validation::validate(&loaded.schema, &doc.parse.document, &doc.text);
                    diagnostics.extend(validation_to_lsp(&doc.text, &doc.line_index, &validation));
                }
                Err(err) => {
                    diagnostics.push(server_notice(
                        &doc.text,
                        &doc.line_index,
                        Severity::Warning,
                        "schema-load",
                        &format!(
                            "cannot load schema {}: {err}",
                            resolved.schema_path.display()
                        ),
                    ));
                }
            }
        }

        diagnostics.truncate(config.diagnostics.max_count);
        Some((doc.version, diagnostics))
    }

    /// Publishes the current diagnostics of a document. The computation
    /// happens before the `await` so no cache reference is held across an
    /// await point.
    async fn publish_diagnostics(&self, uri: &Url) {
        if let Some((version, diagnostics)) = self.compute_diagnostics(uri) {
            self.client
                .publish_diagnostics(uri.clone(), diagnostics, Some(version))
                .await;
        }
    }

    async fn flush_startup_messages(&self) {
        let messages: Vec<String> = std::mem::take(
            &mut *self
                .startup_messages
                .lock()
                .expect("messages lock is never poisoned"),
        );
        for message in messages {
            self.client.log_message(MessageType::WARNING, message).await;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Workspace root for resolving relative configuration paths.
        #[allow(deprecated)]
        let root = params
            .workspace_folders
            .as_ref()
            .and_then(|folders| folders.first())
            .map(|folder| folder.uri.clone())
            .or_else(|| params.root_uri.clone())
            .and_then(|uri| uri.to_file_path().ok());
        *self
            .workspace_root
            .write()
            .expect("root lock is never poisoned") = root;

        if let Some(options) = &params.initialization_options {
            self.set_config(ServerConfig::from_json(options));
        }
        let warnings = self.rebuild_registry();
        self.startup_messages
            .lock()
            .expect("messages lock is never poisoned")
            .extend(warnings);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "xml-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "xml-lsp ready")
            .await;
        self.flush_startup_messages().await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.store.open(doc.uri.clone(), doc.version, doc.text);
        self.publish_diagnostics(&doc.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let known = self
            .store
            .change(&uri, params.text_document.version, &params.content_changes);
        if known {
            self.publish_diagnostics(&uri).await;
        } else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!("didChange for unknown document: {uri}"),
                )
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.store.close(&uri);
        // Clean up the closed document's diagnostics on the client side.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.set_config(ServerConfig::from_json(&params.settings));
        for warning in self.rebuild_registry() {
            self.client.log_message(MessageType::WARNING, warning).await;
        }
        // Apply changed settings (schemas, limits, …) to all open
        // documents right away.
        for uri in self.store.uris() {
            self.publish_diagnostics(&uri).await;
        }
    }
}
