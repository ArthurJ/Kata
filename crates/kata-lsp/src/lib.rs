pub mod analysis;
mod diagnostics;
mod hover;
pub mod server;
mod state;
mod unicode;

pub use server::KataLsp;

/// Runs the LSP server on stdio. Used by the `kata lsp` subcommand
/// and by the standalone `kata-lsp` binary.
pub fn run_stdio() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let rt = tokio::runtime::Runtime::new().expect("failed to create Tokio runtime");
    rt.block_on(async {
        let (service, socket) = tower_lsp::LspService::new(KataLsp::new);
        tower_lsp::Server::new(stdin, stdout, socket)
            .serve(service)
            .await;
    });
}
