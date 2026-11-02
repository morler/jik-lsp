use crate::{
    check::{run_check, CheckOutput},
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

impl State {
    // Must be called under the same lock as publication and invalidation.
    fn accept_result(
        &mut self,
        entry: PathBuf,
        epoch: u64,
        generation: u64,
        result: HashMap<Url, Vec<Diagnostic>>,
    ) -> bool {
        if self.stopped || self.epoch != epoch || self.generations.get(&entry) != Some(&generation)
        {
            return false;
        }
        self.results.insert(entry, result);
        true
    }
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
        let job = CheckJob {
            client,
            shared,
            executable,
            entry,
            epoch,
            generation,
            snapshot,
        };
        let task = tokio::spawn(job.run());
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

struct CheckJob {
    client: Client,
    shared: Arc<Mutex<State>>,
    executable: String,
    entry: PathBuf,
    epoch: u64,
    generation: u64,
    snapshot: HashMap<Url, String>,
}

impl CheckJob {
    async fn run(self) {
        let Self {
            client,
            shared,
            executable,
            entry,
            epoch,
            generation,
            snapshot,
        } = self;
        // Read before and after the compiler runs; unsaved buffers must not receive disk diagnostics.
        let before = read_open(&snapshot).await;
        if snapshot.iter().any(|(uri, text)| {
            uri.to_file_path().is_ok() && before.get(uri).map(|file| &file.text) != Some(text)
        }) {
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
        let Some(parsed) = parse_check_output(&client, &output).await else {
            return;
        };
        let Some(result) = convert_diagnostics(&client, parsed.located, &snapshot, &before).await
        else {
            return;
        };
        // A failed process with no recognisable diagnostic is not a successful empty check.
        if !output.status.success() && result.is_empty() {
            return;
        }
        // Check again after resolving diagnostic paths and reading sources.
        if read_open(&snapshot).await != before {
            client
                .log_message(
                    MessageType::WARNING,
                    "jik check discarded: open files changed on disk",
                )
                .await;
            return;
        }
        let mut state = shared.lock().await;
        if state.accept_result(entry, epoch, generation, result) {
            Backend::publish(&client, &mut state).await;
        }
    }
}

async fn parse_check_output(
    client: &Client,
    output: &CheckOutput,
) -> Option<diagnostics::ParsedDiagnostics> {
    let stdout = decode_output(client, &output.stdout, "stdout").await?;
    let stderr = decode_output(client, &output.stderr, "stderr").await?;
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
    Some(parsed)
}

async fn decode_output<'a>(client: &Client, bytes: &'a [u8], stream: &str) -> Option<&'a str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(_) => {
            client
                .log_message(
                    MessageType::ERROR,
                    format!("jik check {stream} is not UTF-8"),
                )
                .await;
            None
        }
    }
}

async fn convert_diagnostics(
    client: &Client,
    located: Vec<diagnostics::CompilerDiagnostic>,
    snapshot: &HashMap<Url, String>,
    before: &HashMap<Url, FileSnapshot>,
) -> Option<HashMap<Url, Vec<Diagnostic>>> {
    let mut result: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
    for diagnostic in located {
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
        let diagnostic_path = path.clone();
        let diagnostic_file =
            tokio::task::spawn_blocking(move || read_file_snapshot(&diagnostic_path))
                .await
                .ok()
                .flatten();
        let matching_open: Vec<_> = before
            .iter()
            .filter(|(_, file)| {
                diagnostic_file
                    .as_ref()
                    .is_some_and(|other| file.same_identity(other))
            })
            .map(|(uri, _)| uri.clone())
            .collect();
        let text = if let Some(open_uri) = matching_open.first() {
            let unchanged = diagnostic_file
                .as_ref()
                .zip(before.get(open_uri))
                .is_some_and(|(file, open)| file.matches_version(open));
            if !unchanged {
                client
                    .log_message(
                        MessageType::WARNING,
                        "jik check discarded: open files changed on disk",
                    )
                    .await;
                return None;
            }
            snapshot[open_uri].clone()
        } else {
            let Some(text) = read_diagnostic_source(client, &path).await else {
                continue;
            };
            text
        };
        let Some(position) = compiler_position(&text, diagnostic.line, diagnostic.byte_column)
        else {
            client
                .log_message(
                    MessageType::WARNING,
                    format!("Cannot locate {}: {}", path.display(), diagnostic.message),
                )
                .await;
            continue;
        };
        let diagnostic = Diagnostic {
            range: Range::new(position, position),
            severity: Some(if diagnostic.warning {
                DiagnosticSeverity::WARNING
            } else {
                DiagnosticSeverity::ERROR
            }),
            source: Some("jik".into()),
            message: diagnostic.message,
            ..Default::default()
        };
        if matching_open.is_empty() {
            result.entry(uri).or_default().push(diagnostic);
        } else {
            for open_uri in matching_open {
                result.entry(open_uri).or_default().push(diagnostic.clone());
            }
        }
    }
    Some(result)
}

async fn read_diagnostic_source(client: &Client, path: &std::path::Path) -> Option<String> {
    let source_path = path.to_owned();
    let error =
        match tokio::task::spawn_blocking(move || std::fs::read_to_string(source_path)).await {
            Ok(Ok(text)) => return Some(text),
            Ok(Err(error)) => error.to_string(),
            Err(error) => error.to_string(),
        };
    client
        .log_message(
            MessageType::WARNING,
            format!("Cannot read {}: {error}", path.display()),
        )
        .await;
    None
}

#[derive(Debug, PartialEq, Eq)]
struct FileSnapshot {
    text: String,
    canonical: PathBuf,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl FileSnapshot {
    fn matches_version(&self, other: &Self) -> bool {
        self.same_identity(other)
            && self.text == other.text
            && self.modified == other.modified
            && self.same_version(other)
    }

    fn same_version(&self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            self.identity == other.identity
        }
        #[cfg(not(unix))]
        {
            let _ = other;
            true
        }
    }

    fn same_identity(&self, other: &Self) -> bool {
        if self.canonical == other.canonical {
            return true;
        }
        #[cfg(unix)]
        if self.identity.0 == other.identity.0 && self.identity.1 == other.identity.1 {
            return true;
        }
        false
    }
}

#[derive(PartialEq, Eq)]
struct SnapshotMetadata {
    modified: std::time::SystemTime,
    len: u64,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

fn read_file_snapshot(path: &std::path::Path) -> Option<FileSnapshot> {
    fn metadata(path: &std::path::Path) -> Option<SnapshotMetadata> {
        let info = std::fs::metadata(path).ok()?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(SnapshotMetadata {
            modified: info.modified().ok()?,
            len: info.len(),
            #[cfg(unix)]
            identity: (info.dev(), info.ino(), info.ctime(), info.ctime_nsec()),
        })
    }
    let before = metadata(path)?;
    let canonical = std::fs::canonicalize(path).ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let after = metadata(path)?;
    if before != after || canonical != std::fs::canonicalize(path).ok()? {
        return None;
    }
    Some(FileSnapshot {
        text,
        canonical,
        modified: after.modified,
        #[cfg(unix)]
        identity: after.identity,
    })
}

async fn read_open(snapshot: &HashMap<Url, String>) -> HashMap<Url, FileSnapshot> {
    let uris: Vec<_> = snapshot.keys().cloned().collect();
    tokio::task::spawn_blocking(move || {
        uris.into_iter()
            .filter_map(|uri| {
                let file = read_file_snapshot(&uri.to_file_path().ok()?)?;
                Some((uri, file))
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

    #[tokio::test]
    async fn late_uncancellable_results_cannot_replace_newer_results() {
        let entry = PathBuf::from("/entry.jik");
        let uri = Url::parse("file:///entry.jik").unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        state.lock().await.generations.insert(entry.clone(), 1);
        let (release, blocked) = tokio::sync::oneshot::channel();
        let old_state = state.clone();
        let old_entry = entry.clone();
        let old_uri = uri.clone();
        let old = tokio::spawn(async move {
            blocked.await.unwrap();
            let mut result = HashMap::new();
            result.insert(
                old_uri,
                vec![Diagnostic::new_simple(Range::default(), "stale".into())],
            );
            old_state
                .lock()
                .await
                .accept_result(old_entry, 0, 1, result)
        });
        {
            let mut current = state.lock().await;
            current.generations.insert(entry.clone(), 2);
            let mut result = HashMap::new();
            result.insert(
                uri.clone(),
                vec![Diagnostic::new_simple(Range::default(), "fresh".into())],
            );
            assert!(current.accept_result(entry.clone(), 0, 2, result));
        }
        release.send(()).unwrap();
        assert!(!old.await.unwrap());
        let mut current = state.lock().await;
        assert_eq!(current.results[&entry][&uri][0].message, "fresh");
        current.epoch += 1;
        assert!(!current.accept_result(entry.clone(), 0, 2, HashMap::new()));
        current.stopped = true;
        let epoch = current.epoch;
        assert!(!current.accept_result(entry, epoch, 2, HashMap::new()));
    }

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
