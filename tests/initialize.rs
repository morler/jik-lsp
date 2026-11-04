//! Initialize-time validation of `jikPath` and `entryPath`.
//!
//! A wrong compiler path or a missing entry used to surface only after every
//! save, as a repeated "failed to spawn" log line the user has to notice. These
//! tests pin the behaviour down: the mistakes are refused at `initialize` with a
//! message naming the offending path.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn send(input: &mut impl Write, message: Value) {
    let body = message.to_string();
    write!(input, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
    input.flush().unwrap();
}

fn receive(output: &mut impl BufRead) -> Value {
    let mut length = None;
    loop {
        let mut line = String::new();
        assert_ne!(output.read_line(&mut line).unwrap(), 0, "unexpected EOF");
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.trim_end().split_once(':').expect("invalid LSP header");
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("missing Content-Length")];
    output.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// A scratch directory that removes itself even when a test panics.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("jik-lsp-initialize-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
fn make_unreadable_by_exec(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// Run `initialize` with the given options and return the response message.
fn initialize(options: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jik-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut input,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "processId": null, "rootUri": null, "capabilities": {},
                "initializationOptions": options
            }
        }),
    );
    let response = receive(&mut output);
    // The server exits on its own once stdin closes, so this needs no shutdown
    // round trip even when the initialization was refused.
    drop(input);
    let _ = child.wait();
    response
}

fn error_message(response: &Value) -> &str {
    response["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an initialize error, got {response}"))
}

#[test]
fn refuses_a_compiler_path_that_does_not_exist() {
    let scratch = Scratch::new("missing-compiler");
    let missing = scratch.path("no such jik");
    let response = initialize(json!({ "jikPath": missing }));
    let message = error_message(&response);
    assert!(
        message.contains("jikPath does not exist") && message.contains("no such jik"),
        "{message}"
    );
}

#[cfg(unix)]
#[test]
fn refuses_a_compiler_that_is_not_executable() {
    let scratch = Scratch::new("not-executable");
    let compiler = scratch.write("jik", "#!/bin/sh\nexit 0\n");
    make_unreadable_by_exec(&compiler);
    let response = initialize(json!({ "jikPath": compiler }));
    let message = error_message(&response);
    assert!(
        message.contains("not an executable file"),
        "expected an executability complaint, got {message}"
    );
}

#[test]
fn refuses_a_bare_compiler_name_that_is_not_on_path() {
    let response = initialize(json!({ "jikPath": "jik-lsp-nonexistent-compiler-xyz" }));
    let message = error_message(&response);
    assert!(
        message.contains("was not found on PATH"),
        "expected a PATH complaint, got {message}"
    );
}

#[test]
fn refuses_a_relative_entry_path() {
    let response = initialize(json!({ "entryPath": "relative/entry.jik" }));
    let message = error_message(&response);
    assert!(message.contains("must be absolute"), "{message}");
}

#[test]
fn refuses_an_entry_path_that_does_not_exist() {
    let scratch = Scratch::new("missing-entry");
    let entry = scratch.path("absent.jik");
    let response = initialize(json!({ "entryPath": entry }));
    let message = error_message(&response);
    assert!(
        message.contains("entryPath does not exist") && message.contains("absent.jik"),
        "{message}"
    );
}

#[cfg(unix)]
#[test]
fn accepts_a_valid_compiler_and_entry() {
    let scratch = Scratch::new("valid");
    let compiler = scratch.write("jik", "#!/bin/sh\nexit 0\n");
    make_executable(&compiler);
    let entry = scratch.write("entry.jik", "func main():\nend\n");
    let response = initialize(json!({ "jikPath": compiler, "entryPath": entry }));
    assert_eq!(response["id"], 1);
    assert_eq!(
        response["result"]["serverInfo"]["name"], "jik-lsp",
        "{response}"
    );
    assert!(response.get("error").is_none(), "{response}");
}

/// The bare default name `jik` must not be validated at initialize time: it is
/// resolved through `PATH` when the compiler is spawned, and refusing to start
/// without a compiler installed would break the unconfigured case.
#[test]
fn accepts_the_default_compiler_name() {
    let response = initialize(json!({}));
    assert_eq!(response["id"], 1);
    assert!(response.get("error").is_none(), "{response}");
}
