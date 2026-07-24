//! The LSP backend: implements tower-lsp's `LanguageServer` interface and
//! wires up the document cache, parser, language registry, and
//! validation.

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, CompletionItem, CompletionItemKind,
    CompletionOptions, CompletionParams, CompletionResponse, CompletionTextEdit, Diagnostic,
    DidChangeConfigurationParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbolParams, DocumentSymbolResponse, Documentation,
    FoldingRange, FoldingRangeParams, FoldingRangeProviderCapability, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, InlayHint, InlayHintParams,
    InsertTextFormat, Location, MarkupContent, MarkupKind, MessageType, OneOf, Position,
    PrepareRenameResponse, Range, ReferenceParams, RenameOptions, RenameParams, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url, WorkDoneProgressOptions,
    WorkspaceEdit,
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

    /// Computes hover information at a position, if a schema applies.
    pub fn compute_hover(&self, uri: &Url, position: Position) -> Option<Hover> {
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

        let info = xml_hover::hover(&loaded.schema, &doc.parse.document, &doc.text, offset)?;
        let start = doc.line_index.line_col(&doc.text, info.range.start);
        let end = doc.line_index.line_col(&doc.text, info.range.end);
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info.markdown,
            }),
            range: Some(Range {
                start: Position {
                    line: start.line,
                    character: start.col,
                },
                end: Position {
                    line: end.line,
                    character: end.col,
                },
            }),
        })
    }

    /// Definition locations for the symbol at a position.
    pub fn compute_definitions(&self, uri: &Url, position: Position) -> Option<Vec<Location>> {
        let doc = self.store.get(uri)?;
        let offset = doc.line_index.offset(
            &doc.text,
            LineCol {
                line: position.line,
                col: position.character,
            },
        );
        let ranges = crate::navigation::definitions(&doc.parse.document, &doc.text, offset);
        Some(to_locations(uri, &doc.text, &doc.line_index, ranges))
    }

    /// All occurrence locations for the symbol at a position.
    pub fn compute_references(
        &self,
        uri: &Url,
        position: Position,
        include_definitions: bool,
    ) -> Option<Vec<Location>> {
        let doc = self.store.get(uri)?;
        let offset = doc.line_index.offset(
            &doc.text,
            LineCol {
                line: position.line,
                col: position.character,
            },
        );
        let ranges = crate::navigation::references(
            &doc.parse.document,
            &doc.text,
            offset,
            include_definitions,
        );
        Some(to_locations(uri, &doc.text, &doc.line_index, ranges))
    }

    /// The renameable range and placeholder at a position.
    pub fn compute_prepare_rename(
        &self,
        uri: &Url,
        position: Position,
    ) -> Option<PrepareRenameResponse> {
        let doc = self.store.get(uri)?;
        let offset = doc.line_index.offset(
            &doc.text,
            LineCol {
                line: position.line,
                col: position.character,
            },
        );
        let target = crate::rename::prepare(&doc.parse.document, &doc.text, offset)?;
        Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: to_lsp_range(&doc.text, &doc.line_index, target.range),
            placeholder: target.text,
        })
    }

    /// The workspace edit for a rename at a position.
    pub fn compute_rename(
        &self,
        uri: &Url,
        position: Position,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        let doc = self.store.get(uri)?;
        let offset = doc.line_index.offset(
            &doc.text,
            LineCol {
                line: position.line,
                col: position.character,
            },
        );
        let edits = crate::rename::rename(&doc.parse.document, &doc.text, offset, new_name)?;
        let edits = edits
            .into_iter()
            .map(|(range, new_text)| TextEdit {
                range: to_lsp_range(&doc.text, &doc.line_index, range),
                new_text,
            })
            .collect();
        let mut changes = std::collections::HashMap::new();
        changes.insert(uri.clone(), edits);
        Some(WorkspaceEdit {
            changes: Some(changes),
            ..WorkspaceEdit::default()
        })
    }

    /// Quick fixes for parser diagnostics in a range.
    pub fn compute_code_actions(&self, uri: &Url, range: Range) -> Option<CodeActionResponse> {
        let doc = self.store.get(uri)?;
        let to_offset = |position: Position| {
            doc.line_index.offset(
                &doc.text,
                LineCol {
                    line: position.line,
                    col: position.character,
                },
            )
        };
        let range = xml_ast::TextRange::new(to_offset(range.start), to_offset(range.end));
        let fixes = crate::code_actions::quick_fixes(
            &doc.parse.document,
            &doc.text,
            &doc.parse.diagnostics,
            range,
        );
        let actions = fixes
            .into_iter()
            .map(|fix| {
                let diagnostic = &doc.parse.diagnostics[fix.diagnostic];
                let edits = fix
                    .edits
                    .into_iter()
                    .map(|(range, new_text)| TextEdit {
                        range: to_lsp_range(&doc.text, &doc.line_index, range),
                        new_text,
                    })
                    .collect();
                let mut changes = std::collections::HashMap::new();
                changes.insert(uri.clone(), edits);
                CodeActionOrCommand::CodeAction(CodeAction {
                    title: fix.title,
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(parse_to_lsp(
                        &doc.text,
                        &doc.line_index,
                        std::slice::from_ref(diagnostic),
                    )),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..WorkspaceEdit::default()
                    }),
                    ..CodeAction::default()
                })
            })
            .collect();
        Some(actions)
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
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: crate::semantic_tokens::legend(),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            ..SemanticTokensOptions::default()
                        },
                    ),
                ),
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

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        Ok(self.compute_hover(&uri, position))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let locations = self.compute_definitions(&uri, position).unwrap_or_default();
        Ok(match locations.len() {
            0 => None,
            1 => Some(GotoDefinitionResponse::Scalar(
                locations.into_iter().next().expect("length checked"),
            )),
            _ => Some(GotoDefinitionResponse::Array(locations)),
        })
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_definitions = params.context.include_declaration;
        let locations = self
            .compute_references(&uri, position, include_definitions)
            .unwrap_or_default();
        Ok((!locations.is_empty()).then_some(locations))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let hints = self.store.get(&params.text_document.uri).map(|doc| {
            let to_offset = |position: Position| {
                doc.line_index.offset(
                    &doc.text,
                    LineCol {
                        line: position.line,
                        col: position.character,
                    },
                )
            };
            let range =
                xml_ast::TextRange::new(to_offset(params.range.start), to_offset(params.range.end));
            crate::inlay_hints::inlay_hints(&doc.parse.document, &doc.text, &doc.line_index, range)
        });
        Ok(hints)
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let tokens = self.store.get(&params.text_document.uri).map(|doc| {
            crate::semantic_tokens::semantic_tokens(&doc.parse.document, &doc.text, &doc.line_index)
        });
        Ok(tokens.map(|data| {
            SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data,
            })
        }))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        Ok(self.compute_code_actions(&params.text_document.uri, params.range))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        Ok(self.compute_prepare_rename(&params.text_document.uri, params.position))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        Ok(self.compute_rename(&uri, position, &params.new_name))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let symbols = self.store.get(&params.text_document.uri).map(|doc| {
            crate::symbols::document_symbols(&doc.parse.document, &doc.text, &doc.line_index)
        });
        Ok(symbols.map(DocumentSymbolResponse::Nested))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let ranges = self.store.get(&params.text_document.uri).map(|doc| {
            crate::folding::folding_ranges(&doc.parse.document, &doc.text, &doc.line_index)
        });
        Ok(ranges)
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

/// Converts a byte range to an LSP range.
fn to_lsp_range(text: &str, line_index: &xml_ast::LineIndex, range: xml_ast::TextRange) -> Range {
    let start = line_index.line_col(text, range.start);
    let end = line_index.line_col(text, range.end);
    Range {
        start: Position {
            line: start.line,
            character: start.col,
        },
        end: Position {
            line: end.line,
            character: end.col,
        },
    }
}

/// Converts byte ranges in one document to LSP locations.
fn to_locations(
    uri: &Url,
    text: &str,
    line_index: &xml_ast::LineIndex,
    ranges: Vec<xml_ast::TextRange>,
) -> Vec<Location> {
    ranges
        .into_iter()
        .map(|range| Location {
            uri: uri.clone(),
            range: to_lsp_range(text, line_index, range),
        })
        .collect()
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
