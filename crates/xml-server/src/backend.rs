//! The LSP backend: implements tower-lsp's `LanguageServer` interface and
//! wires up the document cache, parser, language registry, and
//! validation.

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    CompletionTextEdit, Diagnostic, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, Documentation, InitializeParams,
    InitializeResult, InitializedParams, InsertTextFormat, MarkupContent, MarkupKind, MessageType,
    Position, Range, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url,
};
use tower_lsp::{Client, LanguageServer};
use xml_ast::LineCol;
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
    /// Whether the client understands snippet completions.
    snippet_support: RwLock<bool>,
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
            snippet_support: RwLock::new(false),
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

    /// Computes completion items at a position, if a schema applies.
    pub fn compute_completions(
        &self,
        uri: &Url,
        position: Position,
    ) -> Option<Vec<CompletionItem>> {
        let doc = self.store.get(uri)?;
        let offset = doc.line_index.offset(
            &doc.text,
            LineCol {
                line: position.line,
                col: position.character,
            },
        );
        let path = uri.to_file_path().ok();
        let resolved = self
            .registry
            .read()
            .expect("registry lock is never poisoned")
            .resolve(&doc.parse.document, &doc.text, path.as_deref())?;
        let loaded = self
            .schemas
            .lock()
            .expect("schema lock is never poisoned")
            .load(&resolved.schema_path)
            .ok()?;

        let items =
            xml_completion::complete(&loaded.schema, &doc.parse.document, &doc.text, offset);
        let snippets = *self
            .snippet_support
            .read()
            .expect("snippet lock is never poisoned");
        Some(
            items
                .into_iter()
                .map(|item| to_lsp_completion(&doc.text, &doc.line_index, item, snippets))
                .collect(),
        )
    }

    /// Human-readable schema association of a document, for the log.
    fn describe_association(&self, uri: &Url) -> String {
        let Some(doc) = self.store.get(uri) else {
            return format!("{uri}: unknown document");
        };
        let path = uri.to_file_path().ok();
        let resolved = self
            .registry
            .read()
            .expect("registry lock is never poisoned")
            .resolve(&doc.parse.document, &doc.text, path.as_deref());
        match resolved {
            Some(resolved) => format!(
                "{uri}: schema {} ({:?})",
                resolved.schema_path.display(),
                resolved.origin
            ),
            None => format!("{uri}: no schema associated — well-formedness checks only"),
        }
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

        let snippets = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|c| c.completion.as_ref())
            .and_then(|c| c.completion_item.as_ref())
            .and_then(|c| c.snippet_support)
            .unwrap_or(false);
        *self
            .snippet_support
            .write()
            .expect("snippet lock is never poisoned") = snippets;

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
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(["<", "/", "\"", "'"].map(str::to_string).to_vec()),
                    ..CompletionOptions::default()
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "xml-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        let (profiles, catalog_entries) = {
            let registry = self
                .registry
                .read()
                .expect("registry lock is never poisoned");
            (registry.profile_count(), registry.catalog_count())
        };
        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "xml-lsp ready — {profiles} language profile(s), \
                     {catalog_entries} catalog entrie(s)"
                ),
            )
            .await;
        self.flush_startup_messages().await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.store.open(doc.uri.clone(), doc.version, doc.text);
        // One line per opened document showing which schema (if any)
        // applies — the first thing to look for when nothing happens.
        let association = self.describe_association(&doc.uri);
        self.client
            .log_message(MessageType::INFO, association)
            .await;
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

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        Ok(self
            .compute_completions(&uri, position)
            .map(CompletionResponse::Array))
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.store.close(&uri);
        // Clean up the closed document's diagnostics on the client side.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        // Clients (Zed among them) send null or empty settings right
        // after startup when nothing is configured in their `settings`
        // section. That must not wipe the configuration received via
        // initializationOptions.
        let meaningful = match &params.settings {
            serde_json::Value::Object(map) => !map.is_empty(),
            _ => false,
        };
        if !meaningful {
            return;
        }
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

/// Maps an editor-agnostic completion item to the LSP shape.
fn to_lsp_completion(
    text: &str,
    line_index: &xml_ast::LineIndex,
    item: xml_completion::CompletionItem,
    snippets: bool,
) -> CompletionItem {
    let start = line_index.line_col(text, item.replace.start);
    let end = line_index.line_col(text, item.replace.end);
    let range = Range {
        start: Position {
            line: start.line,
            character: start.col,
        },
        end: Position {
            line: end.line,
            character: end.col,
        },
    };

    let (new_text, format) = match (&item.snippet, snippets) {
        (Some(snippet), true) => (snippet.clone(), Some(InsertTextFormat::SNIPPET)),
        _ => (item.insert.clone(), None),
    };

    let kind = match item.kind {
        xml_completion::CompletionItemKind::Element => CompletionItemKind::CLASS,
        xml_completion::CompletionItemKind::Attribute => CompletionItemKind::PROPERTY,
        xml_completion::CompletionItemKind::Value => CompletionItemKind::VALUE,
        xml_completion::CompletionItemKind::CloseTag => CompletionItemKind::CLASS,
    };

    CompletionItem {
        kind: Some(kind),
        documentation: item.documentation.map(|value| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            })
        }),
        // Required attributes sort before optional ones.
        sort_text: Some(format!(
            "{}{}",
            if item.required { "0" } else { "1" },
            item.label
        )),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text })),
        insert_text_format: format,
        label: item.label,
        ..CompletionItem::default()
    }
}
