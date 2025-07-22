use std::io::{self, BufRead, Read, Write};
use serde_json::Value;
use ve::parser::Parser;
use ve::typeck::TypeChecker;
use ve::lexer::Lexer;
use codespan::Files;
use std::collections::HashMap;

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut buffer = String::new();

    loop {
        buffer.clear();
        let mut content_length = 0usize;

        loop {
            let mut header = String::new();
            if stdin.lock().read_line(&mut header).unwrap() == 0 {
                return;
            }
            if header.trim().is_empty() {
                break;
            }
            if let Some(val) = header.strip_prefix("Content-Length:") {
                content_length = val.trim().parse::<usize>().unwrap();
            }
        }

        let mut content = vec![0; content_length];
        stdin.lock().read_exact(&mut content).unwrap();
        let body = String::from_utf8_lossy(&content);

        let msg: Value = match serde_json::from_str(&body) {
            Ok(val) => val,
            Err(_) => continue,
        };

        if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
            match method {
                "initialize" => {
                    let id = msg.get("id").cloned().unwrap_or(serde_json::json!(1));
                    let result = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "capabilities": {
                                "textDocumentSync": 1,
                                "diagnosticProvider": {}
                            }
                        }
                    });
                    let out = result.to_string();
                    write!(stdout, "Content-Length: {}\r\n\r\n", out.len()).unwrap();
                    stdout.write_all(out.as_bytes()).unwrap();
                    stdout.flush().unwrap();
                }
                "textDocument/didOpen" | "textDocument/didChange" => {
                    let Some(params) = msg.get("params") else { continue; };
                    let text = params
                        .get("textDocument")
                        .and_then(|td| td.get("text").or_else(|| td.get("newText")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    let uri = params
                        .get("textDocument")
                        .and_then(|td| td.get("uri"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("file:///main.veil");

                    let mut files = Files::new();
                    let file_id = files.add("main.veil", text.to_string());

                    let lexer = Lexer::new(&files, file_id);
                    let mut parser = Parser::new(lexer);

                    match parser.parse() {
                        Ok(mut program) => {
                            let mut checker = TypeChecker::new(
                                file_id,
                                HashMap::new(),
                                Vec::new(),
                                Vec::new(),
                            );
                            match checker.check(&mut program) {
                                Ok(()) => {
                                    send_lsp_diagnostics(&mut stdout, uri, &files, Vec::<codespan_reporting::diagnostic::Diagnostic<_>>::new());
                                }
                                Err(errors) => {
                                    send_lsp_diagnostics(&mut stdout, uri, &files, errors);
                                }
                            }
                        }
                        Err(diagnostic) => {
                            send_lsp_diagnostics(&mut stdout, uri, &files, vec![diagnostic]);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn send_lsp_diagnostics(
    stdout: &mut dyn Write,
    uri: &str,
    files: &Files<String>,
    errors: Vec<codespan_reporting::diagnostic::Diagnostic<codespan::FileId>>,
) {
    let diagnostics: Vec<_> = errors.iter().map(|e| codespan_to_lsp_diag(e, files)).collect();
    let publish = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": diagnostics
        }
    });
    let out = publish.to_string();
    write!(stdout, "Content-Length: {}\r\n\r\n", out.len()).unwrap();
    stdout.write_all(out.as_bytes()).unwrap();
    stdout.flush().unwrap();
}

fn codespan_to_lsp_diag(
    diagnostic: &codespan_reporting::diagnostic::Diagnostic<codespan::FileId>,
    files: &Files<String>,
) -> serde_json::Value {
    let label = diagnostic.labels.first().expect("diagnostic has no label");
    let file = files.source(label.file_id);
    let start = label.range.start;
    let end = label.range.end;

    let (mut line_start, mut char_start, mut line_end, mut char_end) = (0, 0, 0, 0);
    let mut acc = 0;
    for (i, line) in file.lines().enumerate() {
        let len = line.len() + 1;
        if acc <= start && start < acc + len {
            line_start = i;
            char_start = start - acc;
        }
        if acc <= end && end <= acc + len {
            line_end = i;
            char_end = end - acc;
            break;
        }
        acc += len;
    }

    serde_json::json!({
        "range": {
            "start": { "line": line_start, "character": char_start },
            "end":   { "line": line_end,   "character": char_end }
        },
        "severity": 1,
        "message": diagnostic.message.clone()
    })
}
