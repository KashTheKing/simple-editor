//! MCP server so AI agents (Claude Code etc.) can co-edit live: Streamable HTTP transport (JSON-RPC 2.0
//! over HTTP POST /mcp on 127.0.0.1:<port>, no external crates — a tiny HTTP/1.1 parser on std TcpListener),
//! toggled in Settings. The server thread handles `initialize`, `ping`, `tools/list`, `tools/call`,
//! `resources/list`, `resources/read` (project json, style summary, notes) and answers JSON
//! (no SSE stream needed for request/response). Tool calls are forwarded to the UI thread as `ToolCall`s
//! (the App executes them on the next frame against the live project, with undo, and replies through the
//! oneshot sender); `ctx.request_repaint()` wakes the UI. Tool definitions (names, descriptions, JSON schemas)
//! live in `tools.rs` — the App matches on the same names.
//!
//! Connect from Claude Code:  `claude mcp add --transport http simple-editor http://127.0.0.1:7337/mcp`

pub mod tools;

use eframe::egui;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// One tool invocation forwarded to the UI thread; reply with Ok(result json) or Err(message).
pub struct ToolCall {
    pub name: String,
    pub args: Value,
    pub reply: Sender<Result<Value, String>>,
}

const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_CONNS: usize = 16;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl Server {
    /// Bind 127.0.0.1:port and start the server thread. Returns the server handle and the receiver the UI
    /// thread polls every frame (`try_recv` in a loop). Err if the port is busy. Port 0 = ephemeral (see `port()`).
    pub fn start(port: u16, ctx: egui::Context) -> Result<(Server, Receiver<ToolCall>), String> {
        let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let (tx, rx) = mpsc::channel::<ToolCall>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let live = Arc::new(AtomicUsize::new(0));
        std::thread::Builder::new()
            .name("mcp-accept".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if stop2.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(mut stream) = conn else { continue };
                    if live.load(Ordering::Relaxed) >= MAX_CONNS {
                        let _ = write_response(&mut stream, 503, "application/json", b"", true);
                        continue;
                    }
                    live.fetch_add(1, Ordering::Relaxed);
                    let (tx2, ctx2, live2, stop3) = (tx.clone(), ctx.clone(), live.clone(), stop2.clone());
                    let spawned = std::thread::Builder::new()
                        .name("mcp-conn".into())
                        .spawn(move || {
                            let _ = handle_connection(stream, &tx2, &ctx2, &stop3);
                            live2.fetch_sub(1, Ordering::Relaxed);
                        })
                        .is_ok();
                    if !spawned {
                        live.fetch_sub(1, Ordering::Relaxed);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok((Server { port, stop }, rx))
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    /// The actual bound port (differs from the requested one when starting on port 0).
    #[cfg(test)]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stop accepting connections (the thread exits after the current request).
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock accept() so the thread sees the flag and exits.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// The `claude mcp add` command line shown in the settings UI.
pub fn claude_code_command(port: u16) -> String {
    format!("claude mcp add --transport http simple-editor http://127.0.0.1:{port}/mcp")
}

// ---------------------------------------------------------------------------
// HTTP

fn handle_connection(
    mut stream: TcpStream,
    tx: &Sender<ToolCall>,
    ctx: &egui::Context,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        // Request line (tolerate a leading empty line per RFC 9112).
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(()); // EOF: client closed
        }
        if line.trim_end().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_ascii_uppercase();
        let path = parts.next().unwrap_or("");
        let path = path.split('?').next().unwrap_or(path);
        // Headers (case-insensitive names).
        let mut content_length = 0usize;
        let mut origin: Option<String> = None;
        let mut accept = String::new();
        let mut close = false;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 {
                return Ok(());
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            let Some((name, value)) = h.split_once(':') else { continue };
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => content_length = value.parse().unwrap_or(0),
                "origin" => origin = Some(value.to_string()),
                "accept" => accept = value.to_ascii_lowercase(),
                "connection" => close = value.eq_ignore_ascii_case("close"),
                _ => {}
            }
        }
        if let Some(o) = &origin {
            if !origin_ok(o) {
                write_response(&mut stream, 403, "application/json", br#"{"error":"forbidden origin"}"#, true)?;
                return Ok(());
            }
        }
        if content_length > MAX_BODY {
            // Can't cheaply skip a huge body; refuse and drop the connection.
            write_response(&mut stream, 413, "application/json", b"", true)?;
            return Ok(());
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;
        match (method.as_str(), path == "/mcp") {
            ("POST", true) => {
                let reply = handle_post(&body, tx, ctx);
                match reply {
                    None => write_response(&mut stream, 202, "", b"", close)?,
                    Some(v) => {
                        let json = v.to_string();
                        // A client that only accepts SSE gets the response as a single event.
                        if accept.contains("text/event-stream")
                            && !accept.contains("application/json")
                            && !accept.contains("*/*")
                        {
                            let sse = format!("event: message\ndata: {json}\n\n");
                            write_response(&mut stream, 200, "text/event-stream", sse.as_bytes(), close)?;
                        } else {
                            write_response(&mut stream, 200, "application/json", json.as_bytes(), close)?;
                        }
                    }
                }
            }
            ("GET", true) => write_response(&mut stream, 405, "application/json", b"", close)?,
            ("DELETE", true) => write_response(&mut stream, 200, "application/json", b"", close)?, // session end
            _ => write_response(&mut stream, 404, "application/json", b"", close)?,
        }
        if close {
            return Ok(());
        }
    }
}

fn origin_ok(origin: &str) -> bool {
    let host = origin.split("://").nth(1).unwrap_or(origin);
    let host = host.split([':', '/']).next().unwrap_or("");
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]"
}

fn session_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        format!("se-{:x}-{ms:x}", std::process::id())
    })
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    close: bool,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "",
    };
    let mut head =
        format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nMcp-Session-Id: {}\r\n", body.len(), session_id());
    if !content_type.is_empty() {
        head.push_str("Content-Type: ");
        head.push_str(content_type);
        head.push_str("\r\n");
    }
    if close {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

// ---------------------------------------------------------------------------
// JSON-RPC

/// Handle a POST body (single request or batch). None = nothing to send back (notifications only) → 202.
fn handle_post(body: &[u8], tx: &Sender<ToolCall>, ctx: &egui::Context) -> Option<Value> {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => {
            return Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}))
        }
    };
    match parsed {
        Value::Array(items) => {
            let replies: Vec<Value> = items.iter().filter_map(|r| dispatch(r, tx, ctx)).collect();
            if replies.is_empty() {
                None
            } else {
                Some(Value::Array(replies))
            }
        }
        single => dispatch(&single, tx, ctx),
    }
}

/// One JSON-RPC request → response (None for notifications).
fn dispatch(req: &Value, tx: &Sender<ToolCall>, ctx: &egui::Context) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    if method.is_empty() {
        return Some(rpc_err(id.unwrap_or(Value::Null), -32600, "invalid request".into()));
    }
    if method.starts_with("notifications/") {
        return None; // e.g. notifications/initialized — acknowledged with 202, no body
    }
    let id = id?; // no id = notification: nothing to answer
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            let pv = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18");
            Ok(json!({
                "protocolVersion": pv,
                "capabilities": {"tools": {}, "resources": {}},
                "serverInfo": {"name": "simple-editor", "version": env!("CARGO_PKG_VERSION")},
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools::list_json()})),
        "tools/call" => match params.get("name").and_then(Value::as_str) {
            None | Some("") => Err((-32602, "missing tool name".into())),
            Some(name) => {
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                Ok(match call_tool(name, args, tx, ctx) {
                    Ok(v) => json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false}),
                    Err(msg) => json!({"content": [{"type": "text", "text": msg}], "isError": true}),
                })
            }
        },
        "resources/list" => Ok(json!({"resources": [
            {"uri": "simple-editor://project", "name": "project", "description": "Full project JSON (the .sedit document)", "mimeType": "application/json"},
            {"uri": "simple-editor://style", "name": "style", "description": "Markdown style summary of the project", "mimeType": "text/markdown"},
            {"uri": "simple-editor://notes", "name": "notes", "description": "Free-form project notes", "mimeType": "text/plain"},
        ]})),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            let (tool, mime) = match uri {
                "simple-editor://project" => ("project.get", "application/json"),
                "simple-editor://style" => ("style.summary", "text/markdown"),
                "simple-editor://notes" => ("notes.get", "text/plain"),
                _ => ("", ""),
            };
            if tool.is_empty() {
                Err((-32602, format!("unknown resource: {uri}")))
            } else {
                match call_tool(tool, json!({}), tx, ctx) {
                    Ok(v) => {
                        let text = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                        Ok(json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]}))
                    }
                    Err(msg) => Err((-32603, msg)),
                }
            }
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, msg)) => rpc_err(id, code, msg),
    })
}

fn rpc_err(id: Value, code: i64, message: String) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Forward one tool call to the UI thread and wait for the reply.
fn call_tool(name: &str, args: Value, tx: &Sender<ToolCall>, ctx: &egui::Context) -> Result<Value, String> {
    let (reply, rx) = mpsc::channel();
    tx.send(ToolCall { name: name.to_string(), args, reply }).map_err(|_| "editor is shutting down".to_string())?;
    ctx.request_repaint();
    let timeout = match name {
        "export.video" | "media.convert" => Duration::from_secs(30 * 60),
        _ => Duration::from_secs(60),
    };
    rx.recv_timeout(timeout).map_err(|_| format!("{name}: timed out"))?
}

// ---------------------------------------------------------------------------
// PNG (store-only zlib — no deps)

/// Encode an RGBA frame as PNG (store-only zlib) for the `render.frame` tool.
pub fn png_encode(frame: &crate::media::Frame) -> Vec<u8> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let stride = w * 4;
    // Filtered scanlines: filter byte 0 (None) + row.
    let mut raw = Vec::with_capacity((stride + 1) * h);
    for row in frame.rgba.chunks_exact(stride).take(h) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    // zlib: header + deflate stored blocks (<= 65535 bytes each) + adler32 of the raw data.
    let mut z = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 16);
    z.extend_from_slice(&[0x78, 0x01]);
    if raw.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]); // final empty stored block
    } else {
        let mut blocks = raw.chunks(65535).peekable();
        while let Some(b) = blocks.next() {
            z.push(blocks.peek().is_none() as u8); // BFINAL, BTYPE=00 (stored)
            let len = b.len() as u16;
            z.extend_from_slice(&len.to_le_bytes());
            z.extend_from_slice(&(!len).to_le_bytes());
            z.extend_from_slice(b);
        }
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = Vec::with_capacity(z.len() + 64);
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&frame.width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&frame.height.to_be_bytes());
    ihdr[8..].copy_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, deflate, no interlace
    write_chunk(&mut png, b"IHDR", &ihdr);
    write_chunk(&mut png, b"IDAT", &z);
    write_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_chunk(out: &mut Vec<u8>, typ: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(typ);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(typ, data).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
};

fn crc32(a: &[u8], b: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &x in a.iter().chain(b) {
        c = CRC_TABLE[((c ^ x as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn read_response(stream: &TcpStream) -> (u16, String) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let status: u16 = line.split_whitespace().nth(1).unwrap_or("0").parse().unwrap();
        let mut len = 0usize;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).unwrap();
            if h.trim_end().is_empty() {
                break;
            }
            if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).unwrap();
        (status, String::from_utf8(body).unwrap())
    }

    fn post(stream: &mut TcpStream, body: &str) -> (u16, Value) {
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(req.as_bytes()).unwrap();
        let (status, body) = read_response(stream);
        let v = if body.is_empty() { Value::Null } else { serde_json::from_str(&body).unwrap() };
        (status, v)
    }

    #[test]
    fn server_end_to_end() {
        let (server, rx) = Server::start(0, egui::Context::default()).unwrap();
        let port = server.port();
        assert_ne!(port, 0);
        assert_eq!(server.url(), format!("http://127.0.0.1:{port}/mcp"));
        // UI-thread stand-in: answer two tool calls.
        let answerer = std::thread::spawn(move || {
            for _ in 0..2 {
                let Ok(call) = rx.recv_timeout(Duration::from_secs(10)) else { return };
                let r = match call.name.as_str() {
                    "project.summary" => Ok(json!({"clips": 3})),
                    "notes.get" => Ok(json!("hello notes")),
                    other => Err(format!("unknown tool {other}")),
                };
                let _ = call.reply.send(r);
            }
        });
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();

        // initialize (echoes the client's protocol version)
        let (st, v) = post(
            &mut s,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        );
        assert_eq!(st, 200);
        assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(v["result"]["serverInfo"]["name"], "simple-editor");
        assert!(v["result"]["capabilities"]["tools"].is_object());

        // notifications/initialized -> 202, empty body
        let (st, v) = post(&mut s, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert_eq!(st, 202);
        assert_eq!(v, Value::Null);

        // ping
        let (st, v) = post(&mut s, r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#);
        assert_eq!(st, 200);
        assert!(v["result"].is_object());

        // tools/list: one entry per TOOLS row, with schemas
        let (st, v) = post(&mut s, r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        assert_eq!(st, 200);
        let list = v["result"]["tools"].as_array().unwrap();
        assert_eq!(list.len(), tools::TOOLS.len());
        let summary = list.iter().find(|t| t["name"] == "project.summary").unwrap();
        assert_eq!(summary["inputSchema"]["type"], "object");

        // tools/call project.summary
        let (st, v) = post(
            &mut s,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project.summary","arguments":{}}}"#,
        );
        assert_eq!(st, 200);
        assert_eq!(v["result"]["isError"], false);
        assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("clips"));

        // resources/read notes -> forwarded as notes.get
        let (st, v) = post(
            &mut s,
            r#"{"jsonrpc":"2.0","id":5,"method":"resources/read","params":{"uri":"simple-editor://notes"}}"#,
        );
        assert_eq!(st, 200);
        assert_eq!(v["result"]["contents"][0]["text"], "hello notes");

        // resources/list
        let (_, v) = post(&mut s, r#"{"jsonrpc":"2.0","id":6,"method":"resources/list"}"#);
        assert_eq!(v["result"]["resources"].as_array().unwrap().len(), 3);

        // unknown method -> -32601; parse error -> -32700
        let (_, v) = post(&mut s, r#"{"jsonrpc":"2.0","id":7,"method":"nope"}"#);
        assert_eq!(v["error"]["code"], -32601);
        let (_, v) = post(&mut s, "not json");
        assert_eq!(v["error"]["code"], -32700);

        // GET -> 405, DELETE -> 200 (same keep-alive connection)
        s.write_all(b"GET /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        assert_eq!(read_response(&s).0, 405);
        s.write_all(b"DELETE /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        assert_eq!(read_response(&s).0, 200);

        // Non-localhost Origin -> 403 (fresh connection; the server closes it)
        let mut evil = TcpStream::connect(("127.0.0.1", port)).unwrap();
        evil.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        evil.write_all(
            b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://evil.com\r\nContent-Length: 2\r\n\r\n{}",
        )
        .unwrap();
        assert_eq!(read_response(&evil).0, 403);

        server.stop();
        answerer.join().unwrap();
    }

    #[test]
    fn command_line() {
        assert_eq!(
            claude_code_command(7337),
            "claude mcp add --transport http simple-editor http://127.0.0.1:7337/mcp"
        );
    }

    #[test]
    fn png_signature_and_ffmpeg_roundtrip() {
        // 200x100 -> raw stream > 65535 bytes: exercises multi-block stored zlib.
        let mut f = crate::media::Frame::new(200, 100);
        for (i, b) in f.rgba.iter_mut().enumerate() {
            *b = (i * 7 % 251) as u8;
        }
        let png = png_encode(&f);
        assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        let path = std::env::temp_dir().join(format!("se-mcp-png-{}.png", std::process::id()));
        std::fs::write(&path, &png).unwrap();
        let out = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
            .output();
        let _ = std::fs::remove_file(&path);
        let Ok(out) = out else { return }; // no ffmpeg on PATH: skip
        assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.stdout, f.rgba, "decoded RGBA differs");
    }
}
