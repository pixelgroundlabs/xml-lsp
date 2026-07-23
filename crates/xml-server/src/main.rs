//! Entry point: starts the LSP server on stdin/stdout.

use tower_lsp::{LspService, Server};
use xml_server::Backend;

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
