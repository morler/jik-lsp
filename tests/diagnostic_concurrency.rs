#![cfg(unix)]

use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tower_lsp::lsp_types::Url;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Harness {
    dir: PathBuf,
    child: Child,
    input: ChildStdin,
    messages: mpsc::Receiver<Value>,
    registration: Option<Value>,
}

impl Harness {
    fn new(script: &str) -> Self {
        Self::with(&json!({}), None, script)
    }

    fn with(capabilities: &Value, entry: Option<&std::path::Path>, script: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "jik-lsp-concurrency-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let compiler = dir.join("jik");
        fs::write(&compiler, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_jik-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (tx, messages) = mpsc::channel();
        std::thread::spawn(move || loop {
            let mut length = None;
            loop {
                let mut line = String::new();
                if output.read_line(&mut line).unwrap() == 0 {
                    return;
                }
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.trim_end().split_once(':').unwrap();
                if name.eq_ignore_ascii_case("content-length") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            let mut body = vec![0; length.unwrap()];
            output.read_exact(&mut body).unwrap();
            if tx.send(serde_json::from_slice(&body).unwrap()).is_err() {
                return;
            }
        });
        let mut this = Self {
            dir,
            child,
            input,
            messages,
            registration: None,
        };
        let mut options = json!({"jikPath": compiler});
        if let Some(entry) = entry {
            options["entryPath"] = json!(entry);
        }
        this.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":capabilities,"initializationOptions":options}}));
        this.wait(|m| m["id"] == 1);
        this.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
        if capabilities["workspace"]["didChangeWatchedFiles"]["dynamicRegistration"] == true {
            let request = this.wait(|m| m["method"] == "client/registerCapability");
            this.send(json!({"jsonrpc":"2.0","id":request["id"],"result":null}));
            this.registration = Some(request);
        }
        this
    }
    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.input.flush().unwrap();
    }
    fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let debug = std::env::var_os("JIK_LSP_DEBUG_MSGS").is_some();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("timed out waiting for LSP message: {error:?}"));
            if debug {
                eprintln!("recv {}", message);
            }
            if predicate(&message) {
                return message;
            }
        }
    }
    fn open(&mut self, name: &str) -> String {
        let path = self.dir.join(name);
        fs::write(&path, "source\n").unwrap();
        let uri = Url::from_file_path(path).unwrap().to_string();
        self.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"jik","version":1,"text":"source\n"}}}));
        uri
    }
    fn save(&mut self, uri: &str) {
        self.send(json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":uri}}}));
    }
    fn diagnostics(&self, uri: &str, count: usize) -> Value {
        self.wait(|m| {
            m["method"] == "textDocument/publishDiagnostics"
                && m["params"]["uri"] == uri
                && m["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|a| a.len() == count)
        })
    }
    fn wait_file(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.dir.join(name).exists() {
            assert!(Instant::now() < deadline, "timed out waiting for {name}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn invalid_compiler_output_is_reported() {
    for (script, stream) in [
        ("printf '\\377'", "stdout"),
        ("printf '\\377' >&2", "stderr"),
    ] {
        let mut h = Harness::new(script);
        let uri = h.open("entry.jik");
        h.save(&uri);
        let expected = format!("jik check {stream} is not UTF-8");
        h.wait(|message| {
            assert_ne!(message["method"], "textDocument/publishDiagnostics");
            message["method"] == "window/logMessage" && message["params"]["message"] == expected
        });
    }
}

#[test]
fn multiple_entries_merge_and_failure_clears_only_its_own_result() {
    let mut h = Harness::new(
        r#"
dir=${0%/*}
case "$2" in
  */one.jik) if [ -f "$dir/fail" ]; then exit 3; fi ;;
esac
printf 'jik error: %s\n --> %s/shared.jik:1:1\n' "${2##*/}" "$dir" >&2
exit 1
"#,
    );
    fs::write(h.dir.join("shared.jik"), "source\n").unwrap();
    let shared = Url::from_file_path(h.dir.join("shared.jik"))
        .unwrap()
        .to_string();
    let one = h.open("one.jik");
    let two = h.open("two.jik");
    h.save(&one);
    h.diagnostics(&shared, 1);
    h.save(&two);
    let both = h.diagnostics(&shared, 2);
    let messages = both["params"]["diagnostics"].as_array().unwrap();
    assert!(messages.iter().any(|d| d["message"] == "one.jik"));
    assert!(messages.iter().any(|d| d["message"] == "two.jik"));
    fs::write(h.dir.join("fail"), "").unwrap();
    h.save(&one);
    let remaining = h.diagnostics(&shared, 1);
    assert_eq!(remaining["params"]["diagnostics"][0]["message"], "two.jik");
    h.wait(|m| {
        m["method"] == "window/logMessage"
            && m["params"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("failed: exit status: 3"))
    });
}

#[test]
fn unsaved_open_buffer_skips_check() {
    let mut h = Harness::new(": > \"${0%/*}/started\"");
    let uri = h.open("entry.jik");
    h.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"unsaved"}]}}));
    h.save(&uri);
    h.wait(|m| {
        m["method"] == "window/logMessage"
            && m["params"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("open file differs from disk"))
    });
    assert!(!h.dir.join("started").exists());
}

#[test]
fn compiler_path_alias_publishes_to_open_uri() {
    use std::os::unix::fs::symlink;
    let mut h = Harness::new(
        "printf 'jik error: symlink\\n --> %s/alias.jik:1:1\\njik error: hardlink\\n --> %s/hard.jik:1:1\\n' \"${0%/*}\" \"${0%/*}\" >&2; exit 1",
    );
    let uri = h.open("entry.jik");
    symlink(h.dir.join("entry.jik"), h.dir.join("alias.jik")).unwrap();
    fs::hard_link(h.dir.join("entry.jik"), h.dir.join("hard.jik")).unwrap();
    h.save(&uri);
    let diagnostic = h.diagnostics(&uri, 2);
    let diagnostics = diagnostic["params"]["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|d| d["message"] == "symlink"));
    assert!(diagnostics.iter().any(|d| d["message"] == "hardlink"));
}

#[test]
fn disk_change_then_restore_discards_result() {
    let mut h = Harness::new(
        r#"
dir=${0%/*}
: > "$dir/started"
while [ ! -f "$dir/release" ]; do :; done
printf 'jik error: obsolete\n --> %s:1:1\n' "$2" >&2
exit 1
"#,
    );
    let uri = h.open("entry.jik");
    h.save(&uri);
    h.wait_file("started");
    fs::write(h.dir.join("entry.jik"), "changed externally\n").unwrap();
    fs::write(h.dir.join("entry.jik"), "source\n").unwrap();
    fs::write(h.dir.join("release"), "").unwrap();
    h.wait(|m| {
        assert_ne!(m["method"], "textDocument/publishDiagnostics");
        m["method"] == "window/logMessage"
            && m["params"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("open files changed on disk"))
    });
}

#[test]
fn disk_change_during_check_discards_result() {
    let mut h = Harness::new(
        r#"
dir=${0%/*}
: > "$dir/started"
while [ ! -f "$dir/release" ]; do :; done
printf 'jik error: obsolete\n --> %s:1:1\n' "$2" >&2
exit 1
"#,
    );
    let uri = h.open("entry.jik");
    h.save(&uri);
    h.wait_file("started");
    fs::write(h.dir.join("entry.jik"), "changed externally\n").unwrap();
    fs::write(h.dir.join("release"), "").unwrap();
    h.wait(|m| {
        assert_ne!(m["method"], "textDocument/publishDiagnostics");
        m["method"] == "window/logMessage"
            && m["params"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("open files changed on disk"))
    });
}

#[test]
fn newer_save_cancels_inflight_check() {
    let mut h = Harness::new(
        r#"
dir=${0%/*}
if [ ! -f "$dir/started" ]; then
  : > "$dir/started"
  while [ ! -f "$dir/release" ]; do :; done
  printf 'jik error: stale\n --> %s:1:1\n' "$2" >&2
else
  printf 'jik error: fresh\n --> %s:1:1\n' "$2" >&2
fi
exit 1
"#,
    );
    let uri = h.open("entry.jik");
    h.save(&uri);
    h.wait_file("started");
    h.save(&uri);
    let fresh = h.diagnostics(&uri, 1);
    assert_eq!(fresh["params"]["diagnostics"][0]["message"], "fresh");
    fs::write(h.dir.join("release"), "").unwrap();
    // A request/response barrier observes all messages queued before shutdown completes.
    h.send(json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
    loop {
        let message = h.messages.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!message.to_string().contains("stale"));
        if message["id"] == 2 {
            break;
        }
    }
}

#[test]
fn watcher_registration_is_dynamic_and_optional() {
    let h = Harness::with(
        &json!({"workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}}),
        None,
        ":",
    );
    let request = h.registration.as_ref().expect("watcher was not registered");
    let registration = &request["params"]["registrations"][0];
    assert_eq!(registration["method"], "workspace/didChangeWatchedFiles");
    assert_eq!(
        registration["registerOptions"]["watchers"][0]["globPattern"],
        "**/*.jik"
    );
    let mut plain = Harness::new(":");
    plain.open("entry.jik");
    plain.send(json!({"jsonrpc":"2.0","id":3,"method":"shutdown","params":null}));
    loop {
        let message = plain.messages.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(message["method"], "client/registerCapability");
        if message["id"] == 3 {
            break;
        }
    }
}

#[test]
fn watcher_change_rechecks_instead_of_clearing_valid_result() {
    let mut h = Harness::with(
        &json!({"workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}}),
        None,
        r#"
dir=${0%/*}
if [ -f "$dir/second" ]; then
  printf 'jik error: second\n --> %s:1:1\n' "$2" >&2
else
  printf 'jik error: first\n --> %s:1:1\n' "$2" >&2
fi
exit 1
"#,
    );
    let uri = h.open("entry.jik");
    h.save(&uri);
    let first = h.diagnostics(&uri, 1);
    assert_eq!(first["params"]["diagnostics"][0]["message"], "first");
    fs::write(h.dir.join("second"), "").unwrap();
    h.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":uri,"type":2}]}}));
    let second = h.diagnostics(&uri, 1);
    assert_eq!(second["params"]["diagnostics"][0]["message"], "second");
}

#[test]
fn echo_of_own_save_does_not_start_second_check() {
    let entry = std::env::temp_dir().join(format!(
        "jik-lsp-concurrency-echo-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut h = Harness::with(
        &json!({"workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}}),
        Some(&entry),
        r#"
dir=${0%/*}
echo x >> "$dir/runs"
printf 'jik error: only\n --> %s:1:1\n' "$2" >&2
exit 1
"#,
    );
    fs::write(&entry, "source\n").unwrap();
    let uri = Url::from_file_path(&entry).unwrap().to_string();
    h.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"jik","version":1,"text":"source\n"}}}));
    h.save(&uri);
    let diagnostic = h.diagnostics(&uri, 1);
    assert_eq!(diagnostic["params"]["diagnostics"][0]["message"], "only");
    // The client echoes the save as a watched-file event while the entry still
    // holds the content the finished check read: no second compiler run.
    h.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":uri,"type":2}]}}));
    h.send(json!({"jsonrpc":"2.0","id":9,"method":"shutdown","params":null}));
    h.wait(|m| m["id"] == 9);
    let runs = fs::read_to_string(h.dir.join("runs")).unwrap_or_default();
    assert_eq!(
        runs.lines().count(),
        1,
        "watcher echo started a second check"
    );
    let _ = fs::remove_file(&entry);
}

#[test]
fn replaced_entry_with_same_content_still_rechecks() {
    let entry = std::env::temp_dir().join(format!(
        "jik-lsp-concurrency-replace-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut h = Harness::with(
        &json!({"workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}}),
        Some(&entry),
        r#"
dir=${0%/*}
echo x >> "$dir/runs"
printf 'jik error: only\n --> %s:1:1\n' "$2" >&2
exit 1
"#,
    );
    fs::write(&entry, "source\n").unwrap();
    let uri = Url::from_file_path(&entry).unwrap().to_string();
    h.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"jik","version":1,"text":"source\n"}}}));
    h.save(&uri);
    h.diagnostics(&uri, 1);
    // Same file name, different content, older modification time: an echo is
    // decided by content, so the watcher event must still trigger a re-check
    // (which then reports that the open buffer differs from disk).
    fs::write(&entry, "changed externally\n").unwrap();
    Command::new("touch")
        .args(["-t", "197001010000.00"])
        .arg(&entry)
        .status()
        .unwrap();
    h.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":uri,"type":2}]}}));
    h.wait(|m| {
        m["method"] == "window/logMessage"
            && m["params"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("open file differs from disk"))
    });
    let _ = fs::remove_file(&entry);
}

#[test]
fn watcher_change_rechecks_configured_entry() {
    let entry = std::env::temp_dir().join(format!(
        "jik-lsp-concurrency-entry-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut h = Harness::with(
        &json!({"workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}}),
        Some(&entry),
        r#"
dir=${0%/*}
if [ -f "$dir/second" ]; then
  printf 'jik error: second\n --> %s:1:1\n' "$2" >&2
else
  printf 'jik error: first\n --> %s:1:1\n' "$2" >&2
fi
exit 1
"#,
    );
    fs::write(&entry, "source\n").unwrap();
    let uri = Url::from_file_path(&entry).unwrap().to_string();
    h.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"jik","version":1,"text":"source\n"}}}));
    let other = h.open("other.jik");
    h.save(&uri);
    let first = h.diagnostics(&uri, 1);
    assert_eq!(first["params"]["diagnostics"][0]["message"], "first");
    fs::write(h.dir.join("second"), "").unwrap();
    h.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":other,"type":2}]}}));
    let second = h.diagnostics(&uri, 1);
    assert_eq!(second["params"]["diagnostics"][0]["message"], "second");
    let _ = fs::remove_file(&entry);
}
