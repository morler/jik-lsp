//! Contract tests against the real `jik` compiler.
//!
//! The other integration tests drive fake compilers whose output is written by
//! hand, so they can only prove that the server agrees with *our* reading of the
//! diagnostic format. These tests instead run the actual compiler and assert the
//! whole pipeline: real stderr text, `diagnostics::parse`, UTF-8 byte columns to
//! UTF-16 positions, and the published LSP ranges.
//!
//! The compiler is looked up at `../jik/jik` (override with
//! `JIK_LSP_TEST_COMPILER`). When it is missing the tests skip, which is what CI
//! does: no jik checkout is available there.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

fn send(input: &mut impl Write, message: Value) {
    let body = message.to_string();
    write!(input, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
    input.flush().unwrap();
}

/// Read one LSP message, or `None` once the server closes its stdout.
fn receive(output: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if output.read_line(&mut line).unwrap() == 0 {
            return None;
        }
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.trim_end().split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length?];
    output.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).ok()
}

/// Take the next server message, failing rather than hanging when none arrives.
fn next(messages: &Receiver<Value>) -> Value {
    messages
        .recv_timeout(Duration::from_secs(60))
        .expect("server sent no message within the deadline")
}

/// The real compiler, or `None` when this machine has no jik checkout.
fn compiler() -> Option<PathBuf> {
    let path = match std::env::var_os("JIK_LSP_TEST_COMPILER") {
        Some(configured) => PathBuf::from(configured),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../jik/jik"),
    };
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("skipping real compiler test: {} not found", path.display());
        None
    }
}

/// A scratch directory that removes itself, so a panicking test leaves no state.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("jik-lsp-contract-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A running server wired to the real compiler.
///
/// Messages are drained by a reader thread so that a test can wait for one with
/// a deadline; a blocking read would turn any missing notification into a hung
/// test run instead of a failure.
struct Session {
    child: Child,
    input: std::process::ChildStdin,
    messages: Receiver<Value>,
    uri: String,
}

impl Session {
    fn start(compiler: &Path, source: &Path, text: &str) -> Self {
        let uri = tower_lsp::lsp_types::Url::from_file_path(source)
            .unwrap()
            .to_string();
        let mut child = Command::new(env!("CARGO_BIN_EXE_jik-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            while let Some(message) = receive(&mut output) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        send(
            &mut input,
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "processId": null,
                    "rootUri": null,
                    "capabilities": {},
                    "initializationOptions": { "jikPath": compiler }
                }
            }),
        );
        assert_eq!(next(&messages)["id"], 1);
        send(
            &mut input,
            json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        );
        send(
            &mut input,
            json!({
                "jsonrpc": "2.0", "method": "textDocument/didOpen",
                "params": {"textDocument": {
                    "uri": uri, "languageId": "jik", "version": 1, "text": text
                }}
            }),
        );
        Self {
            child,
            input,
            messages,
            uri,
        }
    }

    /// Save the open document and return the diagnostics the server publishes.
    fn diagnostics_after_save(&mut self) -> Vec<Value> {
        send(
            &mut self.input,
            json!({
                "jsonrpc": "2.0", "method": "textDocument/didSave",
                "params": {"textDocument": {"uri": self.uri}}
            }),
        );
        loop {
            let message = next(&self.messages);
            if message["method"] == "textDocument/publishDiagnostics" {
                assert_eq!(message["params"]["uri"], self.uri);
                return message["params"]["diagnostics"].as_array().unwrap().clone();
            }
        }
    }

    /// Wait briefly for any diagnostic publication and fail if one arrives.
    ///
    /// A clean check on a document that never had diagnostics publishes nothing
    /// at all: LSP treats a missing publication as "no diagnostics", so there is
    /// no message to assert on, only silence to verify.
    fn expect_no_publication(&self) {
        while let Ok(message) = self.messages.recv_timeout(Duration::from_secs(2)) {
            assert_ne!(
                message["method"], "textDocument/publishDiagnostics",
                "unexpected diagnostics: {message}"
            );
        }
    }

    fn shutdown(mut self) {
        send(
            &mut self.input,
            json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null}),
        );
        assert_eq!(next(&self.messages)["id"], 2);
        send(
            &mut self.input,
            json!({"jsonrpc": "2.0", "method": "exit", "params": null}),
        );
        drop(self.input);
        assert!(self.child.wait().unwrap().success());
    }
}

/// Assert a diagnostic's zero-width range and message. Ranges are deliberately a
/// single point today because the compiler does not report its span end; when it
/// does, this helper is what has to change.
fn assert_point(diagnostic: &Value, line: u64, character: u64, message: &str) {
    assert_eq!(
        diagnostic["range"],
        json!({
            "start": {"line": line, "character": character},
            "end": {"line": line, "character": character}
        }),
        "{diagnostic}"
    );
    let actual = diagnostic["message"].as_str().unwrap();
    assert!(
        actual.contains(message),
        "message {actual:?} does not contain {message:?}"
    );
    assert_eq!(diagnostic["source"], "jik");
}

#[test]
fn reports_undefined_symbol() {
    let Some(compiler) = compiler() else { return };
    let scratch = Scratch::new("undefined");
    let text = "func main():\n    x := nope\n    println(x)\nend\n";
    let source = scratch.write("source.jik", text);
    let mut session = Session::start(&compiler, &source, text);
    let diagnostics = session.diagnostics_after_save();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_point(&diagnostics[0], 1, 9, "undefined symbol: nope");
    assert_eq!(diagnostics[0]["severity"], 1);
    session.shutdown();
}

#[test]
fn converts_utf8_byte_columns_to_utf16_positions() {
    let Some(compiler) = compiler() else { return };
    let scratch = Scratch::new("utf8");
    // The compiler reports the UTF-8 byte column of `nope`, which is 27 here;
    // the character column is 19, so a server that passed the byte column
    // through unchanged would land in the middle of the string literal.
    let text = "func main():\n    x := \"中文中文\" + nope\n    println(x)\nend\n";
    let source = scratch.write("utf8.jik", text);
    let mut session = Session::start(&compiler, &source, text);
    let diagnostics = session.diagnostics_after_save();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_point(&diagnostics[0], 1, 18, "undefined symbol: nope");
    session.shutdown();
}

#[test]
fn reports_every_parse_error_but_stops_at_first_semantic_error() {
    let Some(compiler) = compiler() else { return };
    let scratch = Scratch::new("parse");

    // Parsing continues after a bad statement, so both lines are reported.
    let parse_text = "func main():\n    let a = 1\n    let b = 2\nend\n";
    let parse_source = scratch.write("parse.jik", parse_text);
    let mut session = Session::start(&compiler, &parse_source, parse_text);
    let diagnostics = session.diagnostics_after_save();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert_point(&diagnostics[0], 1, 4, "invalid statement");
    assert_point(&diagnostics[1], 2, 4, "invalid statement");
    session.shutdown();

    // Semantic analysis stops at the first undefined symbol in a module.
    let semantic_text = "func main():\n    a := nope1\n    b := nope2\n    println(a)\nend\n";
    let semantic_source = scratch.write("semantic.jik", semantic_text);
    let mut session = Session::start(&compiler, &semantic_source, semantic_text);
    let diagnostics = session.diagnostics_after_save();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_point(&diagnostics[0], 1, 9, "undefined symbol: nope1");
    session.shutdown();
}

#[test]
fn handles_paths_with_spaces_and_colons() {
    let Some(compiler) = compiler() else { return };
    let scratch = Scratch::new("paths");
    let text = "func main():\n    x := nope\n    println(x)\nend\n";
    // Both a space and a colon appear in the reported path, so a parser that
    // splits on the first colon cannot recover the right file and line.
    let source = scratch.write("dir with space/a:b source.jik", text);
    let mut session = Session::start(&compiler, &source, text);
    let diagnostics = session.diagnostics_after_save();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_point(&diagnostics[0], 1, 9, "undefined symbol: nope");
    session.shutdown();
}

#[test]
fn reports_missing_main_at_the_end_of_the_file() {
    let Some(compiler) = compiler() else { return };
    let scratch = Scratch::new("nomain");
    let text = "func helper():\n    println(\"x\")\nend\n";
    let source = scratch.write("nomain.jik", text);
    let mut session = Session::start(&compiler, &source, text);
    let diagnostics = session.diagnostics_after_save();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    // The compiler points past the last line (`4:1` in its 1-based output) at an
    // empty line, which is the first position that does not exist in the text.
    assert_point(&diagnostics[0], 3, 0, "missing main function");
    session.shutdown();
}

#[test]
fn publishes_nothing_for_valid_source() {
    let Some(compiler) = compiler() else { return };
    let scratch = Scratch::new("valid");
    let text = "func main():\n    println(\"ok\")\nend\n";
    let source = scratch.write("valid.jik", text);
    let mut session = Session::start(&compiler, &source, text);
    send(
        &mut session.input,
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didSave",
            "params": {"textDocument": {"uri": session.uri}}
        }),
    );
    // Check() runs to completion before shutdown is answered, so a successful
    // check with no diagnostics must leave the client silent.
    session.expect_no_publication();
    session.shutdown();
}
