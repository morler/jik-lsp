use crate::{
    check::run_check,
    diagnostics,
    document::{byte_to_position, Documents},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tower_lsp::jsonrpc::{Error, Result};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, InitializeParams, InitializeResult,
    InitializedParams, MessageType, Range, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Url,
};
use tower_lsp::{Client, LanguageServer};

#[derive(Default)]
struct State {
    documents: Documents,
    executable: Option<String>,
    entry: Option<PathBuf>,
    epoch: u64,
    generations: HashMap<PathBuf, u64>,
    results: HashMap<PathBuf, HashMap<Url, Vec<Diagnostic>>>,
    published: HashSet<Url>,
    stopped: bool,
    tasks: HashMap<PathBuf, tokio::task::AbortHandle>,
}

pub struct Backend {
    client: Client,
    state: Arc<Mutex<State>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    async fn publish(client: &Client, state: &mut State) {
        let mut merged: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
        for result in state.results.values() {
            for (uri, diagnostics) in result {
                merged
                    .entry(uri.clone())
                    .or_default()
                    .extend(diagnostics.iter().cloned());
            }
        }
        let uris: HashSet<_> = state
            .published
            .iter()
            .cloned()
            .chain(merged.keys().cloned())
            .collect();
        for uri in uris {
            client
                .publish_diagnostics(uri.clone(), merged.remove(&uri).unwrap_or_default(), None)
                .await;
        }
        state.published = state
            .results
            .values()
            .flat_map(|r| r.keys().cloned())
            .collect();
    }

    async fn invalidate(&self, state: &mut State, uri: &Url) {
        state.epoch = state.epoch.wrapping_add(1);
        for (_, task) in state.tasks.drain() {
            task.abort();
        }
        // Any open source may affect diagnostics in any entry through imports.
        state.results.clear();
        let _ = uri;
        Self::publish(&self.client, state).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut state = self.state.lock().await;
        let options = params.initialization_options.as_ref();
        if options.is_some_and(|o| !o.is_object() && !o.is_null()) {
            return Err(Error::invalid_params(
                "initializationOptions must be an object",
            ));
        }
        let get_string = |key: &str| -> Result<Option<String>> {
            match options.and_then(|o| o.get(key)) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(v) => v
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| Some(s.to_owned()))
                    .ok_or_else(|| {
                        Error::invalid_params(format!("{key} must be a non-empty string"))
                    }),
            }
        };
        state.executable = Some(get_string("jikPath")?.unwrap_or_else(|| "jik".into()));
        state.entry = get_string("entryPath")?.map(PathBuf::from);
        if state.entry.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err(Error::invalid_params("entryPath must be absolute"));
        }
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
        let mut state = self.state.lock().await;
        if state.documents.open(doc.uri.clone(), doc.text, doc.version) {
            self.invalidate(&mut state, &doc.uri).await;
        }
    }
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut changes = params.content_changes;
        if changes.len() != 1 || changes[0].range.is_some() {
            eprintln!("jik-lsp: invalid full document change");
            return;
        }
        let mut state = self.state.lock().await;
        if state.documents.change(
            &params.text_document.uri,
            changes.remove(0).text,
            params.text_document.version,
        ) {
            self.invalidate(&mut state, &params.text_document.uri).await;
        }
    }
    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut state = self.state.lock().await;
        if state.documents.close(&params.text_document.uri) {
            self.invalidate(&mut state, &params.text_document.uri).await;
        }
    }
    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let mut state = self.state.lock().await;
        let uri = params.text_document.uri;
        if !state.documents.save(&uri) || state.stopped {
            return;
        }
        let entry = match state.entry.clone().or_else(|| uri.to_file_path().ok()) {
            Some(path) if path.is_absolute() => path,
            _ => {
                self.client
                    .log_message(MessageType::ERROR, "Cannot check a non-file URI")
                    .await;
                return;
            }
        };
        state.results.remove(&entry);
        Self::publish(&self.client, &mut state).await;
        let generation = state.generations.entry(entry.clone()).or_default();
        *generation = generation.wrapping_add(1);
        let generation = *generation;
        let epoch = state.epoch;
        let executable = state.executable.clone().unwrap_or_else(|| "jik".into());
        let snapshot = state.documents.texts();
        let shared = self.state.clone();
        let client = self.client.clone();
        let task_entry = entry.clone();
        if let Some(task) = state.tasks.remove(&task_entry) {
            task.abort();
        }
        let task = tokio::spawn(async move {
            // Read before and after the compiler runs; unsaved buffers must not receive disk diagnostics.
            let before = read_open(&snapshot).await;
            if snapshot
                .iter()
                .any(|(uri, text)| uri.to_file_path().is_ok() && before.get(uri) != Some(text))
            {
                client
                    .log_message(
                        MessageType::WARNING,
                        "jik check skipped: an open file differs from disk",
                    )
                    .await;
                return;
            }
            let output = run_check(executable, &entry, Duration::from_secs(30), 1024 * 1024).await;
            let after = read_open(&snapshot).await;
            if before != after {
                client
                    .log_message(
                        MessageType::WARNING,
                        "jik check discarded: open files changed on disk",
                    )
                    .await;
                return;
            }
            let output = match output {
                Ok(output) => output,
                Err(error) => {
                    client
                        .log_message(MessageType::ERROR, error.to_string())
                        .await;
                    return;
                }
            };
            let stdout = match std::str::from_utf8(&output.stdout) {
                Ok(s) => s,
                Err(_) => {
                    client
                        .log_message(MessageType::ERROR, "jik check stdout is not UTF-8")
                        .await;
                    return;
                }
            };
            let stderr = match std::str::from_utf8(&output.stderr) {
                Ok(s) => s,
                Err(_) => {
                    client
                        .log_message(MessageType::ERROR, "jik check stderr is not UTF-8")
                        .await;
                    return;
                }
            };
            let mut parsed = diagnostics::parse(stdout);
            let other = diagnostics::parse(stderr);
            parsed.located.extend(other.located);
            parsed.unlocated.extend(other.unlocated);
            for message in &parsed.unlocated {
                client.log_message(MessageType::WARNING, message).await;
            }
            if !output.status.success() {
                client
                    .log_message(
                        MessageType::ERROR,
                        format!("jik check failed: {}: {}", output.status, stderr.trim()),
                    )
                    .await;
            }
            let mut result: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
            for diagnostic in parsed.located {
                let path = if diagnostic.path.is_absolute() {
                    diagnostic.path.clone()
                } else {
                    match std::env::current_dir() {
                        Ok(dir) => dir.join(&diagnostic.path),
                        Err(_) => continue,
                    }
                };
                let Ok(uri) = Url::from_file_path(&path) else {
                    client
                        .log_message(
                            MessageType::WARNING,
                            format!("Invalid diagnostic path: {}", path.display()),
                        )
                        .await;
                    continue;
                };
                let text = if let Some(open) = snapshot.get(&uri) {
                    if before.get(&uri) != Some(open) || after.get(&uri) != Some(open) {
                        continue;
                    }
                    open.clone()
                } else {
                    let source_path = path.clone();
                    match tokio::task::spawn_blocking(move || std::fs::read_to_string(source_path))
                        .await
                    {
                        Ok(Ok(s)) => s,
                        Ok(Err(error)) => {
                            client
                                .log_message(
                                    MessageType::WARNING,
                                    format!("Cannot read {}: {error}", path.display()),
                                )
                                .await;
                            continue;
                        }
                        Err(error) => {
                            client
                                .log_message(
                                    MessageType::WARNING,
                                    format!("Cannot read {}: {error}", path.display()),
                                )
                                .await;
                            continue;
                        }
                    }
                };
                let Some(position) =
                    compiler_position(&text, diagnostic.line, diagnostic.byte_column)
                else {
                    client
                        .log_message(
                            MessageType::WARNING,
                            format!("Cannot locate {}: {}", path.display(), diagnostic.message),
                        )
                        .await;
                    continue;
                };
                result.entry(uri).or_default().push(Diagnostic {
                    range: Range::new(position, position),
                    severity: Some(if diagnostic.warning {
                        DiagnosticSeverity::WARNING
                    } else {
                        DiagnosticSeverity::ERROR
                    }),
                    source: Some("jik".into()),
                    message: diagnostic.message,
                    ..Default::default()
                });
            }
            // A failed process with no recognisable diagnostic is not a successful empty check.
            if !output.status.success() && result.is_empty() {
                return;
            }
            let mut state = shared.lock().await;
            if state.stopped
                || state.epoch != epoch
                || state.generations.get(&entry) != Some(&generation)
            {
                return;
            }
            state.results.insert(entry, result);
            Backend::publish(&client, &mut state).await;
        });
        state.tasks.insert(task_entry, task.abort_handle());
    }
    async fn shutdown(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.stopped = true;
        for (_, task) in state.tasks.drain() {
            task.abort();
        }
        state.epoch = state.epoch.wrapping_add(1);
        state.results.clear();
        Self::publish(&self.client, &mut state).await;
        Ok(())
    }
}

async fn read_open(snapshot: &HashMap<Url, String>) -> HashMap<Url, String> {
    let uris: Vec<_> = snapshot.keys().cloned().collect();
    tokio::task::spawn_blocking(move || {
        uris.into_iter()
            .filter_map(|uri| {
                let text = std::fs::read_to_string(uri.to_file_path().ok()?).ok()?;
                Some((uri, text))
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

fn compiler_position(
    text: &str,
    line: usize,
    column: usize,
) -> Option<tower_lsp::lsp_types::Position> {
    // The compiler counts lines using LF only; do not guess positions for lone CR.
    if text
        .as_bytes()
        .iter()
        .enumerate()
        .any(|(i, b)| *b == b'\r' && text.as_bytes().get(i + 1) != Some(&b'\n'))
    {
        return None;
    }
    let mut offset = 0;
    for _ in 0..line {
        let tail = text.get(offset..)?;
        let end = tail.find(['\r', '\n'])?;
        offset += end;
        if text.as_bytes()[offset] == b'\r' && text.as_bytes().get(offset + 1) == Some(&b'\n') {
            offset += 2;
        } else {
            offset += 1;
        }
    }
    let tail = text.get(offset..)?;
    let length = tail.find(['\r', '\n']).unwrap_or(tail.len());
    if column > length {
        return None;
    }
    byte_to_position(text, offset.checked_add(column)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    #[test]
    fn compiler_columns_are_checked_utf8_bytes() {
        let text = "a😀\r\n中z\n";
        assert_eq!(compiler_position(text, 0, 5), Some(Position::new(0, 3)));
        assert_eq!(compiler_position(text, 0, 2), None);
        assert_eq!(compiler_position(text, 0, 6), None);
        assert_eq!(compiler_position(text, 1, 3), Some(Position::new(1, 1)));
        assert_eq!(compiler_position(text, 2, 0), Some(Position::new(2, 0)));
        assert_eq!(compiler_position(text, 3, 0), None);
    }
}
