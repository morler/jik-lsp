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
}

impl Harness {
    fn new(script: &str) -> Self {
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
        };
        this.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{},"initializationOptions":{"jikPath":compiler}}}));
        this.wait(|m| m["id"] == 1);
        this.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
        this
    }
    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.input.flush().unwrap();
    }
    fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("timed out waiting for LSP message");
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
