//! Entry point: starts the LSP server on stdin/stdout.

use tower_lsp::{LspService, Server};
use xml_server::Backend;

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    // Only allow a single message to be processed at a time to enforce a strict message order
    Server::new(stdin, stdout, socket).concurrency_level(1).serve(service).await;
}
