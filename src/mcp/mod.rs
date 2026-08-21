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
use serde_json::Value;
use std::sync::mpsc::{Receiver, Sender};

/// One tool invocation forwarded to the UI thread; reply with Ok(result json) or Err(message).
pub struct ToolCall {
    pub name: String,
    pub args: Value,
    pub reply: Sender<Result<Value, String>>,
}

pub struct Server {
    #[allow(dead_code)]
    port: u16,
}

impl Server {
    /// Bind 127.0.0.1:port and start the server thread. Returns the server handle and the receiver the UI
    /// thread polls every frame (`try_recv` in a loop). Err if the port is busy.
    pub fn start(_port: u16, _ctx: egui::Context) -> Result<(Server, Receiver<ToolCall>), String> {
        todo!("mcp::Server::start")
    }
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }
    /// Stop accepting connections (the thread exits after the current request).
    pub fn stop(self) {
        todo!("mcp::Server::stop")
    }
}

/// Encode an RGBA frame as PNG (store-only zlib — no deps) for the `render.frame` tool.
pub fn png_encode(_frame: &crate::media::Frame) -> Vec<u8> {
    todo!("mcp::png_encode")
}
