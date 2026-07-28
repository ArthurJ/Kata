//! tower-lsp LanguageServer impl.
//!
//! O servidor mantém um `DocumentStore` (estado por arquivo) e um canal de
//! debounce para evitar re-análise a cada keystroke.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    HoverParams, InitializeParams, InitializeResult, InitializedParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind,
};
use tower_lsp::{Client, LanguageServer};

use crate::analysis::run_frontend;
use crate::diagnostics::to_diagnostics;
use crate::hover::hover_at;
use crate::state::{DEBOUNCE_MS, DebounceRx, DebounceTx, DocumentStore, debounce_channel};

pub struct KataLsp {
    client: Client,
    docs: Arc<Mutex<DocumentStore>>,
    debounce_tx: DebounceTx,
}

impl KataLsp {
    pub fn new(client: Client) -> Self {
        let (tx, rx) = debounce_channel();
        let docs = Arc::new(Mutex::new(DocumentStore::new()));

        // Spawn debounce worker
        let docs_clone = Arc::clone(&docs);
        let client_clone = client.clone();
        tokio::spawn(debounce_worker(rx, docs_clone, client_clone));

        KataLsp {
            client,
            docs,
            debounce_tx: tx,
        }
    }

    /// Roda análise no documento e publica diagnósticos.
    async fn analyze_and_publish(&self, uri: &tower_lsp::lsp_types::Url) {
        let text = {
            let docs = self.docs.lock().await;
            match docs.get_text(uri) {
                Some(t) => t.to_string(),
                None => return,
            }
        };

        let (analysis, diagnostics) = match run_frontend(&text, uri_to_path(uri)) {
            Ok(result) => {
                let diags = Vec::new(); // Sem erros — diagnósticos vazios
                (Some(result), diags)
            }
            Err(errors) => {
                let diags = to_diagnostics(&errors, &text);
                (None, diags)
            }
        };

        // Atualiza store
        {
            let mut docs = self.docs.lock().await;
            docs.set_analysis(uri, analysis, diagnostics.clone());
        }

        // Publica diagnósticos
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for KataLsp {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(tower_lsp::lsp_types::HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(tower_lsp::lsp_types::ServerInfo {
                name: "kata-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {}

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        let text = params.text_document.text;

        {
            let mut docs = self.docs.lock().await;
            docs.open(uri.clone(), version, text);
        }

        self.analyze_and_publish(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;

        // Full sync: último change tem o texto completo
        if let Some(change) = params.content_changes.into_iter().last() {
            {
                let mut docs = self.docs.lock().await;
                docs.update(&uri, version, change.text);
            }
            // Envia para debounce — worker aguarda 100ms antes de re-analisar
            let _ = self.debounce_tx.send(uri);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        {
            let mut docs = self.docs.lock().await;
            docs.close(&uri);
        }
        // Limpa diagnósticos do arquivo fechado
        self.client
            .publish_diagnostics(uri.clone(), Vec::new(), None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<tower_lsp::lsp_types::Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let docs = self.docs.lock().await;
        let text = match docs.get_text(&uri) {
            Some(t) => t,
            None => return Ok(None),
        };
        let doc = match docs.get(&uri) {
            Some(d) => d,
            None => return Ok(None),
        };

        let analysis = match &doc.analysis {
            Some(a) => a,
            None => return Ok(None),
        };

        Ok(hover_at(&analysis.typed, text, pos))
    }
}

/// Worker de debounce: aguarda 100ms de silêncio antes de re-analisar.
async fn debounce_worker(mut rx: DebounceRx, docs: Arc<Mutex<DocumentStore>>, client: Client) {
    while let Some(uri) = rx.recv().await {
        // Aguarda debounce
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;

        // Drena URIs pendentes (múltiplas changes acumuladas)
        while rx.try_recv().is_ok() {}

        // Re-analisa
        let text = {
            let docs = docs.lock().await;
            match docs.get_text(&uri) {
                Some(t) => t.to_string(),
                None => continue,
            }
        };

        let (analysis, diagnostics) = match run_frontend(&text, uri_to_path(&uri)) {
            Ok(result) => (Some(result), Vec::new()),
            Err(errors) => {
                let diags = to_diagnostics(&errors, &text);
                (None, diags)
            }
        };

        {
            let mut docs = docs.lock().await;
            docs.set_analysis(&uri, analysis, diagnostics.clone());
        }

        client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

/// Converte Url → Option<&str> para file_path (usado por run_frontend).
fn uri_to_path(uri: &tower_lsp::lsp_types::Url) -> Option<&str> {
    if uri.scheme() == "file" {
        uri.path().into()
    } else {
        None
    }
}
