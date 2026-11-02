use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, InitializeParams, InitializeResult, InitializedParams,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextDocumentSyncSaveOptions,
};
use tower_lsp::{Client, LanguageServer};

use crate::document::Documents;

pub struct Backend {
    documents: Mutex<Documents>,
}

impl Backend {
    pub fn new(_client: Client) -> Self {
        Self {
            documents: Mutex::new(Documents::default()),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "jik-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        if !self
            .documents
            .lock()
            .unwrap()
            .open(doc.uri, doc.text, doc.version)
        {
            eprintln!("jik-lsp: ignored duplicate didOpen");
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut changes = params.content_changes;
        if changes.len() != 1 || changes[0].range.is_some() {
            eprintln!("jik-lsp: ignored invalid full document change");
            return;
        }
        if !self.documents.lock().unwrap().change(
            &params.text_document.uri,
            changes.remove(0).text,
            params.text_document.version,
        ) {
            eprintln!("jik-lsp: ignored change for unknown document or stale version");
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if !self
            .documents
            .lock()
            .unwrap()
            .save(&params.text_document.uri)
        {
            eprintln!("jik-lsp: ignored save for unknown document");
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        if !self
            .documents
            .lock()
            .unwrap()
            .close(&params.text_document.uri)
        {
            eprintln!("jik-lsp: ignored close for unknown document");
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
