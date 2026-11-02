#[allow(dead_code)] // The runner is not yet connected to the server.
mod check;
#[allow(dead_code)] // The parser is wired into the server in the next diagnostic integration step.
mod diagnostics;
mod document;
mod server;

use server::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let (service, socket) = LspService::new(Backend::new);
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .concurrency_level(1)
        .serve(service)
        .await;
}
