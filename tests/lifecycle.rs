use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
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

#[test]
fn stdio_lifecycle() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jik-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}),
    );
    let initialized = receive(&mut output);
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["serverInfo"]["name"], "jik-lsp");
    assert_eq!(
        initialized["result"]["capabilities"]["textDocumentSync"],
        json!({"openClose": true, "change": 1, "save": true})
    );
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    let uri = "file:///test.jik";
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"jik","version":1,"text":"hello"}}}),
    );
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"changed 😀"}]}}),
    );
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri}}}),
    );
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}),
    );
    let shutdown = receive(&mut output);
    assert_eq!(shutdown["id"], 2);
    assert!(shutdown["result"].is_null());
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    );
    drop(input);
    let mut remaining = Vec::new();
    output.read_to_end(&mut remaining).unwrap();
    assert!(remaining.is_empty(), "unexpected stdout: {remaining:?}");
    assert!(child.wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn publishes_compiler_location_and_clears_on_change() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let dir = std::env::temp_dir().join(format!("jik-lsp-lifecycle-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source with space.jik");
    let compiler = dir.join("fake jik");
    fs::write(&source, "a😀z\n").unwrap();
    fs::write(
        &compiler,
        "#!/bin/sh\nprintf 'jik error: bad\\n --> %s:1:6\\n' \"$2\" >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&compiler, fs::Permissions::from_mode(0o700)).unwrap();
    let uri = tower_lsp::lsp_types::Url::from_file_path(&source)
        .unwrap()
        .to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_jik-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{},"initializationOptions":{"jikPath":compiler}}}),
    );
    assert_eq!(receive(&mut output)["id"], 1);
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"jik","version":1,"text":"a😀z\n"}}}),
    );
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":uri}}}),
    );
    loop {
        let message = receive(&mut output);
        if message["method"] == "textDocument/publishDiagnostics" {
            assert_eq!(message["params"]["uri"], uri);
            assert_eq!(
                message["params"]["diagnostics"][0]["range"],
                json!({"start":{"line":0,"character":3},"end":{"line":0,"character":3}})
            );
            break;
        }
    }
    // A subsequent check that cannot start must clear the previous result.
    fs::remove_file(&compiler).unwrap();
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":uri}}}),
    );
    let clear = receive(&mut output);
    assert_eq!(clear["method"], "textDocument/publishDiagnostics");
    assert_eq!(clear["params"]["diagnostics"], json!([]));
    loop {
        let message = receive(&mut output);
        if message["method"] == "window/logMessage" {
            assert!(message["params"]["message"]
                .as_str()
                .unwrap()
                .contains("failed to spawn"));
            break;
        }
    }
    fs::write(
        &compiler,
        "#!/bin/sh\nprintf 'jik error: bad\\n --> %s:1:6\\n' \"$2\" >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&compiler, fs::Permissions::from_mode(0o700)).unwrap();
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":uri}}}),
    );
    loop {
        if receive(&mut output)["method"] == "textDocument/publishDiagnostics" {
            break;
        }
    }
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"changed"}]}}),
    );
    let clear = receive(&mut output);
    assert_eq!(clear["method"], "textDocument/publishDiagnostics");
    assert_eq!(clear["params"]["diagnostics"], json!([]));
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}),
    );
    assert_eq!(receive(&mut output)["id"], 2);
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    );
    drop(input);
    assert!(child.wait().unwrap().success());
    fs::remove_dir_all(dir).unwrap();
}
