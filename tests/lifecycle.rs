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
    assert_eq!(initialized["result"]["capabilities"], json!({}));
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
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
