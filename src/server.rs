use crate::{
    check::{run_check, CheckOutput},
    diagnostics,
    document::{byte_to_position, Documents},
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tower_lsp::jsonrpc::{Error, Result};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    InitializeParams, InitializeResult, InitializedParams, MessageType, Range, Registration,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextDocumentSyncSaveOptions, Url,
};
use tower_lsp::{Client, LanguageServer};

/// What a reserved check needs from the state it was reserved in, captured under
/// the state lock so the job can start once the entry is read outside it.
struct CheckTicket {
    epoch: u64,
    generation: u64,
    executable: String,
}

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
    watch_files: bool,
    tasks: HashMap<PathBuf, tokio::task::AbortHandle>,
    /// The entry's on-disk content when its newest check was started, used to
    /// recognise a client echo of our own save without relying on timestamps.
    /// `None` when the file could not be read, which never matches an echo.
    checking: HashMap<PathBuf, Option<FileSnapshot>>,
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

    /// Supersede any in-flight check for `entry`: bump its generation and abort the
    /// running task. Callers own the state lock and are responsible for clearing
    /// this entry's old result and publishing once. Superseding before the entry is
    /// read keeps a finishing check from publishing a result the new check is about
    /// to replace. The echo snapshot is deliberately kept: emptying it here would
    /// leave a window in which a watcher event looks like a fresh change and starts
    /// a duplicate check, and `begin_check` overwrites it once the new read lands.
    fn reserve_check(&mut self, entry: &Path) -> Option<CheckTicket> {
        if self.stopped {
            return None;
        }
        let generation = self.generations.entry(entry.to_path_buf()).or_default();
        *generation = generation.wrapping_add(1);
        if let Some(task) = self.tasks.remove(entry) {
            task.abort();
        }
        Some(CheckTicket {
            epoch: self.epoch,
            generation: *generation,
            executable: self.executable.clone().unwrap_or_else(|| "jik".into()),
        })
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
        state.checking.clear();
        let _ = uri;
        Self::publish(&self.client, state).await;
    }

    /// Spawn the job of a reserved check once its entry snapshot has been read
    /// outside the lock. Returns false when the reservation was invalidated while
    /// that read was in flight.
    fn begin_check(
        &self,
        state: &mut State,
        entry: PathBuf,
        ticket: CheckTicket,
        snapshot: Arc<HashMap<Url, String>>,
        start: Option<FileSnapshot>,
    ) -> bool {
        if state.stopped
            || state.epoch != ticket.epoch
            || state.generations.get(&entry) != Some(&ticket.generation)
        {
            return false;
        }
        // Recorded for later watcher events; read by the caller so no file I/O
        // runs while this lock is held.
        state.checking.insert(entry.clone(), start);
        let task_entry = entry.clone();
        let job = CheckJob {
            client: self.client.clone(),
            shared: self.state.clone(),
            executable: ticket.executable,
            entry,
            epoch: ticket.epoch,
            generation: ticket.generation,
            snapshot,
        };
        let task = tokio::spawn(job.run());
        state.tasks.insert(task_entry, task.abort_handle());
        true
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
        state.watch_files = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.did_change_watched_files.as_ref())
            .and_then(|capability| capability.dynamic_registration)
            .unwrap_or(false);
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
    async fn initialized(&self, _: InitializedParams) {
        if !self.state.lock().await.watch_files {
            return;
        }
        let registration = Registration {
            id: "jik-file-watcher".into(),
            method: "workspace/didChangeWatchedFiles".into(),
            register_options: Some(serde_json::json!({
                "watchers": [{"globPattern": "**/*.jik"}]
            })),
        };
        if let Err(error) = self.client.register_capability(vec![registration]).await {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!("Cannot watch Jik files: {error}"),
                )
                .await;
        }
    }
    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        if params.changes.is_empty() {
            return;
        }
        // A client may echo our own save back as a watched-file event. When every
        // reported file is the configured entry and it still holds the content the
        // running check is reading, that check covers this event and a second
        // compiler run would be pure duplication. Content is compared instead of
        // timestamps, whose granularity is not reliable enough to order a save and
        // the echo it produces. Deleted or missing files, dependencies and
        // unconfigured entrysets always re-check.
        let only_entry = {
            let state = self.state.lock().await;
            if state.stopped {
                return;
            }
            let entry = state.entry.clone().filter(|entry| {
                params
                    .changes
                    .iter()
                    .all(|change| change.uri.to_file_path().as_deref() == Ok(entry.as_path()))
            });
            // `checking` keeps the snapshot of the newest check even after it
            // finishes, so that an echo arriving later is still recognised. It is
            // only empty for an entry that was never checked, and only then can the
            // read below be skipped.
            entry.filter(|entry| state.checking.contains_key(entry))
        };
        if let Some(entry) = only_entry {
            // Read outside the state lock: deciding whether this is an echo must not
            // stall every other request behind file I/O.
            let path = entry.clone();
            let current = tokio::task::spawn_blocking(move || read_file_snapshot(&path))
                .await
                .unwrap_or(None);
            // Compare against the snapshot of the newest started check, taken after
            // the read, so a check that began meanwhile still covers this event.
            let state = self.state.lock().await;
            if state.stopped {
                return;
            }
            let echo = current.is_some_and(|current| {
                state
                    .checking
                    .get(&entry)
                    .and_then(Option::as_ref)
                    .is_some_and(|start| current.same_contents(start))
            });
            if echo {
                return;
            }
        }
        let mut state = self.state.lock().await;
        if state.stopped {
            return;
        }
        // Forget entries whose document is gone instead of keeping them forever.
        let open_documents = state.documents.paths();
        let configured = state.entry.clone();
        state.generations.retain(|entry, _| {
            configured.as_ref() == Some(entry) || open_documents.contains(entry)
        });
        let entries: Vec<_> = match configured {
            Some(entry) => vec![entry],
            None => state.generations.keys().cloned().collect(),
        };
        if entries.is_empty() {
            return;
        }
        // Supersede the running checks before publishing, so a check that is still
        // finishing cannot republish the results cleared here.
        let mut tickets = Vec::new();
        for entry in &entries {
            state.results.remove(entry);
            if let Some(ticket) = state.reserve_check(entry) {
                tickets.push((entry.clone(), ticket));
            }
        }
        // One snapshot per batch: every ticket of this event sees the same buffers.
        let texts = Arc::new(state.documents.texts());
        Self::publish(&self.client, &mut state).await;
        drop(state);
        // Take each entry snapshot for the echo check outside the state lock.
        for (entry, ticket) in tickets {
            let path = entry.clone();
            let start = tokio::task::spawn_blocking(move || read_file_snapshot(&path))
                .await
                .unwrap_or(None);
            let mut state = self.state.lock().await;
            // A false result means the reservation was invalidated while the entry
            // was read; the superseding event owns the follow-up check.
            self.begin_check(&mut state, entry, ticket, texts.clone(), start);
        }
    }
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
        let uri = params.text_document.uri;
        if !state.documents.close(&uri) {
            return;
        }
        // Forget per-entry bookkeeping for a closed document, so it is neither
        // re-checked by later watcher events nor kept forever.
        if let Ok(path) = uri.to_file_path() {
            if state.entry.as_ref() != Some(&path) {
                state.generations.remove(&path);
                state.results.remove(&path);
                state.checking.remove(&path);
            }
        }
        self.invalidate(&mut state, &uri).await;
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
        // Reserve before publishing: superseding the previous generation here keeps
        // a finishing check from republishing the result cleared above.
        let ticket = state.reserve_check(&entry);
        let texts = Arc::new(state.documents.texts());
        Self::publish(&self.client, &mut state).await;
        // Snapshot the saved file outside the state lock; it decides whether a later
        // watcher event is merely the client echoing this save.
        drop(state);
        if let Some(ticket) = ticket {
            let path = entry.clone();
            let start = tokio::task::spawn_blocking(move || read_file_snapshot(&path))
                .await
                .unwrap_or(None);
            let mut state = self.state.lock().await;
            // A false result means the reservation was invalidated while the entry
            // was read; whichever event superseded it owns the follow-up check.
            self.begin_check(&mut state, entry, ticket, texts, start);
        }
    }
    async fn shutdown(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.stopped = true;
        for (_, task) in state.tasks.drain() {
            task.abort();
        }
        state.epoch = state.epoch.wrapping_add(1);
        state.results.clear();
        state.checking.clear();
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
    snapshot: Arc<HashMap<Url, String>>,
}

impl CheckJob {
    async fn run(self) {
        let entry = self.entry.clone();
        let shared = self.shared.clone();
        self.check().await;
        shared.lock().await.tasks.remove(&entry);
    }

    async fn check(self) {
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
        let pending = if before == after {
            // Read the current state once before parsing: the latest possible read
            // is priced in, and a changed file can no longer overwrite the warning
            // with "jik check failed" or "is not UTF-8" errors from stale output.
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
            let Some(result) =
                convert_diagnostics(&client, parsed.located, &snapshot, &before).await
            else {
                return;
            };
            // A failed process with no recognisable diagnostic is not a successful empty check.
            if !output.status.success() && result.is_empty() {
                return;
            }
            Some(result)
        } else {
            None
        };
        // Re-read before taking the lock: concurrent buffer changes are already
        // rejected through the epoch/generation check in `accept_result`, and
        // holding the lock across file I/O would stall every other request.
        if read_open(&snapshot).await != before {
            client
                .log_message(
                    MessageType::WARNING,
                    "jik check discarded: open files changed on disk",
                )
                .await;
            return;
        }
        let Some(result) = pending else {
            client
                .log_message(
                    MessageType::WARNING,
                    "jik check discarded: open files changed on disk",
                )
                .await;
            return;
        };
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

    /// Content alone, deliberately ignoring timestamps: a client echo of our own
    /// save can carry an equal or even older modification time, so identity plus
    /// text is what decides whether the running check already covers the file.
    fn same_contents(&self, other: &Self) -> bool {
        self.same_identity(other) && self.text == other.text
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
