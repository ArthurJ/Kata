//! DocumentStore — estado por arquivo aberto no editor.
//!
//! Mantém o texto atual e o último resultado de análise para cada documento.
//! O debounce é via canal Tokio: `didChange` envia a URI para o canal;
//! o worker aguarda 100ms de silêncio antes de re-analisar.

use std::collections::HashMap;

use tokio::sync::mpsc;
use tower_lsp::lsp_types::{Diagnostic, Url};

use crate::analysis::FrontendResult;

/// Estado de um documento aberto no editor.
pub(crate) struct Document {
    pub version: i32,
    pub text: String,
    pub analysis: Option<FrontendResult>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Document {
    fn new(version: i32, text: String) -> Self {
        Document {
            version,
            text,
            analysis: None,
            diagnostics: Vec::new(),
        }
    }
}

/// Estado global do servidor — map de URI → Document.
pub(crate) struct DocumentStore {
    docs: HashMap<Url, Document>,
}

impl DocumentStore {
    pub(crate) fn new() -> Self {
        DocumentStore {
            docs: HashMap::new(),
        }
    }

    pub(crate) fn open(&mut self, uri: Url, version: i32, text: String) {
        self.docs.insert(uri, Document::new(version, text));
    }

    pub(crate) fn update(&mut self, uri: &Url, version: i32, text: String) {
        if let Some(doc) = self.docs.get_mut(uri) {
            doc.version = version;
            doc.text = text;
        }
    }

    pub(crate) fn close(&mut self, uri: &Url) {
        self.docs.remove(uri);
    }

    pub(crate) fn get(&self, uri: &Url) -> Option<&Document> {
        self.docs.get(uri)
    }

    pub(crate) fn get_text(&self, uri: &Url) -> Option<&str> {
        self.docs.get(uri).map(|d| d.text.as_str())
    }

    /// Atualiza o resultado da análise e diagnósticos de um documento.
    pub(crate) fn set_analysis(
        &mut self,
        uri: &Url,
        analysis: Option<FrontendResult>,
        diagnostics: Vec<Diagnostic>,
    ) {
        if let Some(doc) = self.docs.get_mut(uri) {
            doc.analysis = analysis;
            doc.diagnostics = diagnostics;
        }
    }
}

/// Canal de debounce: `didChange` envia URI; worker consome com 100ms de espera.
pub(crate) type DebounceTx = mpsc::UnboundedSender<Url>;
pub(crate) type DebounceRx = mpsc::UnboundedReceiver<Url>;

/// Cria o canal de debounce e retorna tx + rx.
pub(crate) fn debounce_channel() -> (DebounceTx, DebounceRx) {
    mpsc::unbounded_channel()
}

/// Constante: tempo de debounce em millisegundos.
pub(crate) const DEBOUNCE_MS: u64 = 100;
