//! The LSP backend: implements tower-lsp's `LanguageServer` interface and
//! wires up the document cache, parser, and diagnostics.

use std::sync::RwLock;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    DidChangeConfigurationParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializeResult, InitializedParams, MessageType,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer};

use crate::config::ServerConfig;
use crate::diagnostics::to_lsp_diagnostics;
use crate::documents::DocumentStore;

pub struct Backend {
    client: Client,
    store: DocumentStore,
    config: RwLock<ServerConfig>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            store: DocumentStore::default(),
            config: RwLock::new(ServerConfig::default()),
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

    /// Computes the current diagnostics of a document and sends them to the
    /// client. The computation happens before the `await` so no cache
    /// reference is held across an await point.
    async fn publish_diagnostics(&self, uri: &Url) {
        let config = self.config();
        let payload = self.store.get(uri).map(|doc| {
            let diagnostics = to_lsp_diagnostics(
                &doc.text,
                &doc.line_index,
                &doc.parse.diagnostics,
                &config.diagnostics,
            );
            (doc.version, diagnostics)
        });
        if let Some((version, diagnostics)) = payload {
            self.client
                .publish_diagnostics(uri.clone(), diagnostics, Some(version))
                .await;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(options) = &params.initialization_options {
            self.set_config(ServerConfig::from_json(options));
        }

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
        // Apply changed settings (e.g. disabled diagnostics) to all open
        // documents right away.
        for uri in self.store.uris() {
            self.publish_diagnostics(&uri).await;
        }
    }
}
