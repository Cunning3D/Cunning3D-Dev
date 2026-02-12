//! LSP (v1): rust-analyzer stdio client for diagnostics/definition.

use crate::ai_workspace_gpui::protocol::{DiagnosticSnapshot, IdeEvent};
use crossbeam_channel::Sender as XSender;
use lsp_types as lsp;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex, oneshot};
use serde_json::json;

pub struct LspManager { tx: mpsc::Sender<Req> }

enum Req {
    Open { path: PathBuf, text: String, version: i32 },
    Change { path: PathBuf, text: String, version: i32 },
    Close { path: PathBuf },
    Completion { path: PathBuf, line: u32, col: u32 },
    Definition { path: PathBuf, line: u32, col: u32 },
    Hover { path: PathBuf, line: u32, col: u32 },
}

impl LspManager {
    pub fn new(ide_event_tx: XSender<IdeEvent>) -> Self {
        let (tx, mut rx) = mpsc::channel::<Req>(128);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build();
            let Ok(rt) = rt else { return; };
            rt.block_on(async move {
                let mut conn: Option<LspConn> = None;
                while let Some(req) = rx.recv().await {
                    let needs = matches!(&req, Req::Open { path, .. } | Req::Change { path, .. } | Req::Close { path } | Req::Completion { path, .. } | Req::Definition { path, .. } | Req::Hover { path, .. } if is_rust(path));
                    if !needs { continue; }
                    if conn.is_none() {
                        conn = LspConn::start(ide_event_tx.clone()).await.ok();
                        if conn.is_none() { continue; }
                    }
                    if let Some(c) = conn.as_mut() {
                        if let Err(_) = c.handle(req).await { conn = None; }
                    }
                }
            });
        });
        Self { tx }
    }

    pub fn did_open(&self, path: &Path, text: String, version: u64) {
        let _ = self.tx.try_send(Req::Open { path: path.to_path_buf(), text, version: version as i32 });
    }
    pub fn did_change_full(&self, path: &Path, text: String, version: u64) {
        let _ = self.tx.try_send(Req::Change { path: path.to_path_buf(), text, version: version as i32 });
    }
    pub fn did_close(&self, path: &Path) { let _ = self.tx.try_send(Req::Close { path: path.to_path_buf() }); }
    pub fn request_completion(&self, path: PathBuf, line: u32, col: u32) { let _ = self.tx.try_send(Req::Completion { path, line, col }); }
    pub fn request_definition(&self, path: PathBuf, line: u32, col: u32) { let _ = self.tx.try_send(Req::Definition { path, line, col }); }
    pub fn request_hover(&self, path: PathBuf, line: u32, col: u32) { let _ = self.tx.try_send(Req::Hover { path, line, col }); }
}

struct LspConn {
    child: Child,
    stdin: tokio::process::ChildStdin,
    event_tx: XSender<IdeEvent>,
    next_id: i64,
    open: HashMap<lsp::Uri, i32>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
}

impl LspConn {
    async fn start(event_tx: XSender<IdeEvent>) -> anyhow::Result<Self> {
        let mut child = Command::new("rust-analyzer")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("rust-analyzer stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("rust-analyzer stdout"))?;
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut conn = Self { stdin, child, event_tx: event_tx.clone(), next_id: 1, open: HashMap::new(), pending: pending.clone() };
        conn.initialize().await?;
        tokio::spawn(read_loop(stdout, event_tx, pending));
        Ok(conn)
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        let root = std::env::current_dir().ok().and_then(|p| path_to_uri(&p).ok());
        let params = lsp::InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: root,
            capabilities: lsp::ClientCapabilities::default(),
            ..Default::default()
        };
        let id = self.next_id;
        self.next_id += 1;
        let _ = self.request("initialize", serde_json::to_value(params)?, Some(id)).await?;
        self.notify("initialized", Value::Object(Default::default())).await?;
        Ok(())
    }

    async fn handle(&mut self, req: Req) -> anyhow::Result<()> {
        match req {
            Req::Open { path, text, version } => {
                let uri = path_to_uri(&path)?;
                self.open.insert(uri.clone(), version);
                let params = lsp::DidOpenTextDocumentParams {
                    text_document: lsp::TextDocumentItem { uri, language_id: "rust".into(), version, text },
                };
                self.notify("textDocument/didOpen", serde_json::to_value(params)?).await?;
            }
            Req::Change { path, text, version } => {
                let uri = path_to_uri(&path)?;
                if !self.open.contains_key(&uri) { return Ok(()); }
                let params = lsp::DidChangeTextDocumentParams {
                    text_document: lsp::VersionedTextDocumentIdentifier { uri, version },
                    content_changes: vec![lsp::TextDocumentContentChangeEvent { range: None, range_length: None, text }],
                };
                self.notify("textDocument/didChange", serde_json::to_value(params)?).await?;
            }
            Req::Close { path } => {
                let uri = path_to_uri(&path)?;
                self.open.remove(&uri);
                let params = lsp::DidCloseTextDocumentParams { text_document: lsp::TextDocumentIdentifier { uri } };
                self.notify("textDocument/didClose", serde_json::to_value(params)?).await?;
            }
            Req::Completion { path, line, col } => {
                let uri = path_to_uri(&path)?;
                let v = self.request_value("textDocument/completion", json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": col }, "context": { "triggerKind": 1 } })).await?;
                let mut out: Vec<String> = Vec::new();
                if let Some(r) = v.get("result") {
                    if let Ok(resp) = serde_json::from_value::<lsp::CompletionResponse>(r.clone()) {
                        match resp {
                            lsp::CompletionResponse::Array(arr) => out.extend(arr.into_iter().map(|i| i.label)),
                            lsp::CompletionResponse::List(list) => out.extend(list.items.into_iter().map(|i| i.label)),
                        }
                    }
                }
                let _ = self.event_tx.send(IdeEvent::CompletionItems { path, items: out });
            }
            Req::Definition { path, line, col } => {
                let uri = path_to_uri(&path)?;
                let v = self.request_value("textDocument/definition", json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": col } })).await?;
                if let Some(r) = v.get("result") {
                    if let Ok(resp) = serde_json::from_value::<lsp::GotoDefinitionResponse>(r.clone()) {
                        let loc: Option<lsp::Location> = match resp {
                            lsp::GotoDefinitionResponse::Scalar(l) => Some(l),
                            lsp::GotoDefinitionResponse::Array(a) => a.into_iter().next(),
                            lsp::GotoDefinitionResponse::Link(a) => a.into_iter().next().map(|l| lsp::Location { uri: l.target_uri, range: l.target_selection_range }),
                        };
                        if let Some(loc) = loc {
                            let to = uri_to_path(&loc.uri).unwrap_or_else(|| path.clone());
                            let _ = self.event_tx.send(IdeEvent::DefinitionLocation { from: path, to, line: loc.range.start.line, col: loc.range.start.character });
                        }
                    }
                }
            }
            Req::Hover { path, line, col } => {
                let uri = path_to_uri(&path)?;
                let v = self.request_value("textDocument/hover", json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": col } })).await?;
                if let Some(r) = v.get("result") {
                    if let Ok(h) = serde_json::from_value::<lsp::Hover>(r.clone()) {
                        let md = match h.contents {
                            lsp::HoverContents::Scalar(s) => match s { lsp::MarkedString::String(t) => t, lsp::MarkedString::LanguageString(ls) => ls.value },
                            lsp::HoverContents::Array(a) => a.into_iter().map(|s| match s { lsp::MarkedString::String(t) => t, lsp::MarkedString::LanguageString(ls) => ls.value }).collect::<Vec<_>>().join("\n"),
                            lsp::HoverContents::Markup(m) => m.value,
                        };
                        let _ = self.event_tx.send(IdeEvent::HoverText { path, markdown: md });
                    }
                }
            }
        }
        Ok(())
    }

    async fn notify(&mut self, method: &str, params: Value) -> anyhow::Result<()> {
        let msg = jsonrpc(method, params, None);
        write_lsp(&mut self.stdin, &msg).await
    }

    async fn request(&mut self, method: &str, params: Value, id: Option<i64>) -> anyhow::Result<()> {
        let msg = jsonrpc(method, params, id);
        write_lsp(&mut self.stdin, &msg).await
    }

    async fn request_value(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        self.request(method, params, Some(id)).await?;
        Ok(rx.await?)
    }

}

async fn read_loop(mut stdout: tokio::process::ChildStdout, event_tx: XSender<IdeEvent>, pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>) {
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        loop {
            let mut tmp = [0u8; 4096];
            let n = match stdout.read(&mut tmp).await { Ok(0) | Err(_) => return, Ok(n) => n };
            buf.extend_from_slice(&tmp[..n]);
            while let Some((msg, rest)) = try_parse_lsp_message(&buf) {
                buf = rest;
                if let Ok(v) = serde_json::from_slice::<Value>(&msg) {
                    if let Some(id) = v.get("id").and_then(|x| x.as_i64()) {
                        if let Some(tx) = pending.lock().await.remove(&id) { let _ = tx.send(v); }
                        continue;
                    }
                    if v.get("method").and_then(|m| m.as_str()) == Some("textDocument/publishDiagnostics") {
                        if let Some(params) = v.get("params") {
                            if let Ok(p) = serde_json::from_value::<lsp::PublishDiagnosticsParams>(params.clone()) {
                                emit_diagnostics(&event_tx, p);
                            }
                        }
                    }
                }
            }
        }
}

fn is_rust(p: &Path) -> bool { p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("rs")).unwrap_or(false) }

fn emit_diagnostics(tx: &XSender<IdeEvent>, p: lsp::PublishDiagnosticsParams) {
    let Some(path) = uri_to_path(&p.uri) else { return; };
    let mut out = Vec::with_capacity(p.diagnostics.len());
    for d in p.diagnostics {
        let sev = d.severity.map(|s| {
            if s == lsp::DiagnosticSeverity::ERROR { 1 }
            else if s == lsp::DiagnosticSeverity::WARNING { 2 }
            else if s == lsp::DiagnosticSeverity::HINT { 4 }
            else { 3 }
        }).unwrap_or(3);
        out.push(DiagnosticSnapshot {
            message: d.message,
            severity: sev,
            start_line: d.range.start.line,
            start_col: d.range.start.character,
            end_line: d.range.end.line,
            end_col: d.range.end.character,
        });
    }
    let _ = tx.send(IdeEvent::DiagnosticsUpdated { path, diagnostics: out });
}

fn path_to_uri(path: &Path) -> anyhow::Result<lsp::Uri> {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = if s.len() >= 2 && s.as_bytes().get(1) == Some(&b':') && !s.starts_with('/') { format!("/{s}") } else { s };
    let s = s.replace(' ', "%20");
    Ok(format!("file://{s}").parse()?)
}

fn uri_to_path(uri: &lsp::Uri) -> Option<PathBuf> {
    let s = uri.as_str();
    if !s.starts_with("file://") { return None; }
    let mut p = &s["file://".len()..];
    if p.starts_with('/') { p = &p[1..]; }
    let p = urlencoding::decode(p).ok()?.into_owned();
    #[cfg(target_os = "windows")]
    { return Some(PathBuf::from(p.replace('/', "\\"))); }
    #[cfg(not(target_os = "windows"))]
    { Some(PathBuf::from(format!("/{}", p))) }
}

fn jsonrpc(method: &str, params: Value, id: Option<i64>) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("jsonrpc".into(), Value::String("2.0".into()));
    m.insert("method".into(), Value::String(method.into()));
    m.insert("params".into(), params);
    if let Some(id) = id { m.insert("id".into(), Value::Number(id.into())); }
    Value::Object(m)
}

async fn write_lsp(w: &mut tokio::process::ChildStdin, msg: &Value) -> anyhow::Result<()> {
    let body = serde_json::to_vec(msg)?;
    let hdr = format!("Content-Length: {}\r\n\r\n", body.len());
    w.write_all(hdr.as_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

fn try_parse_lsp_message(buf: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let sep = b"\r\n\r\n";
    let i = buf.windows(sep.len()).position(|w| w == sep)?;
    let header = &buf[..i];
    let rest = &buf[i + sep.len()..];
    let header_str = std::str::from_utf8(header).ok()?;
    let len = header_str.lines().find_map(|l| l.strip_prefix("Content-Length:")).and_then(|v| v.trim().parse::<usize>().ok())?;
    if rest.len() < len { return None; }
    let msg = rest[..len].to_vec();
    let remain = rest[len..].to_vec();
    Some((msg, remain))
}

