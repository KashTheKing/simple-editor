use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolKind, ToolOutcome};

/// The pure decision half of `App::run_snapshot_if_mutate`: `Some(project.to_json())` iff `kind` is
/// `ToolKind::Mutate`. Split out so `run_tool_undoable_snapshots_only_mutate` can exercise it against a
/// bare `Project` — no `App` (and so no live `eframe::CreationContext`) required.
pub(super) fn snapshot_if_mutate(project: &Project, kind: ToolKind) -> Option<String> {
    (kind == ToolKind::Mutate).then(|| project.to_json())
}

/// The pure half of `App::run_rollback`: parses `snap` back into a `Project`, or `None` if it fails to
/// parse. Split out (mirrors `snapshot_if_mutate` above) so a test can exercise the actual restore logic
/// `run_rollback` runs, instead of re-implementing `Project::from_json` inline.
pub(super) fn rollback_project(snap: &str) -> Option<Project> {
    Project::from_json(snap).ok()
}

impl App {
    // ---- ws:forgiveness ----
    // deviation: this workstream's own fire_hook stub (scanning scripts for a bare `-- @on <event>`
    // marker) is superseded by command-palette's real dispatcher (palette_ctl::fire_hook — reentrancy
    // guard, per-hook budget, disable-on-overrun) now that PR #45 has merged; removed to avoid a
    // duplicate-method conflict. files.rs's project_open/project_save call sites are unaffected — both
    // pass string literals, which resolve to palette_ctl::fire_hook's `&'static str` parameter as-is.

    pub(super) fn run_script(&mut self, path: &std::path::Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => return self.toast(format!("{name}: {e}")),
        };
        let snap = self.project.to_json();
        let mut logs = Vec::new();
        let result = {
            let app = std::cell::RefCell::new(&mut *self);
            let mut call = |tool: &str, args: &serde_json::Value| -> Result<serde_json::Value, String> {
                let mut app = app.borrow_mut();
                // ---- ws:registries-schema-hooks ----
                // Every tool now runs through its own ToolDef.run (was: the hand-kept run_tool dispatch
                // chain, still reachable through it — see tools_*.rs's row! macro). ToolKind::Mutate
                // (via run_snapshot_if_mutate) replaces the old hand-kept mutating-tool name-set check;
                // the snapshot/rollback primitive is shared with handle_tool below, but the undo-PUSH
                // decision stays here (one entry for the WHOLE script, not per call) — collapsing that
                // onto handle_tool's per-call push would regress run_script's undo count.
                let Some(def) = mcp::tools::find(tool) else {
                    return Err(format!("unknown tool '{tool}'"));
                };
                let before = app.run_snapshot_if_mutate(def);
                match (def.run)(&mut app, args) {
                    Ok(ToolOutcome::Done(v)) => {
                        if before.is_some() {
                            app.after_edit();
                        }
                        Ok(v)
                    }
                    // a script runs synchronously and can't await a background job's reply
                    Ok(ToolOutcome::Job(..)) => {
                        Err(format!("'{tool}' starts a background job and can't be called from a script"))
                    }
                    Err(e) => {
                        if let Some(snap) = before {
                            app.run_rollback(snap); // a failed tool is a no-op
                        }
                        Err(e)
                    }
                }
            };
            crate::scripting::run(&src, &name, &mut call, &mut logs)
        };
        if self.project.to_json() != snap {
            push_undo_json(&mut self.undo, &mut self.redo, snap);
        }
        for l in &logs {
            self.toast(format!("{name}: {l}"));
        }
        match result {
            Ok(()) if logs.is_empty() => self.toast(format!("{name}: done")),
            Ok(()) => {}
            Err(e) => {
                eprintln!("script {name}: {e}");
                let line = e.lines().next().unwrap_or("failed").to_string();
                self.toast(format!("{name}: {line}"));
            }
        }
    }

    // ---- ws:registries-schema-hooks ----
    /// Shared snapshot half of the Mutate rollback shape: `Some(project_json)` when `tool` is a
    /// registered `ToolKind::Mutate` (a project.to_json() snapshot taken before running it), `None`
    /// otherwise (Read/Job/Ui tools, or an unknown name — `run_tool`'s own "unknown tool" error covers
    /// that). Callers push undo themselves (`handle_tool` per call, `run_script` per whole script) —
    /// this only decides WHETHER to snapshot, not when to push.
    pub(super) fn run_snapshot_if_mutate(&self, def: &mcp::tools::ToolDef) -> Option<String> {
        snapshot_if_mutate(&self.project, def.kind)
    }

    /// Restore `snap` after a Mutate-kind call returned `Err` (some arms mutate before returning Err —
    /// e.g. subtitles.set, clip.set — so a failed tool must still be a no-op).
    pub(super) fn run_rollback(&mut self, snap: String) {
        if let Some(p) = rollback_project(&snap) {
            self.project = p;
        }
    }

    // ---------------- MCP ----------------

    /// Start/stop/restart the MCP server to match the settings; runs every frame (cheap when in sync).
    pub(super) fn sync_mcp(&mut self, ctx: &egui::Context) {
        let (want, port) = (self.settings.mcp_enabled, self.settings.mcp_port);
        if !want {
            if let Some((server, _)) = self.mcp.take() {
                server.stop();
                self.toast("MCP server stopped");
            }
            return;
        }
        if self.mcp.is_some() && self.mcp_port_running == port {
            return;
        }
        if let Some((server, _)) = self.mcp.take() {
            server.stop();
        }
        match mcp::Server::start(port, ctx.clone()) {
            Ok((server, rx)) => {
                self.toast(format!("MCP server at {}", server.url()));
                self.mcp = Some((server, rx));
                self.mcp_port_running = port;
            }
            Err(e) => {
                self.toast(format!("MCP server failed: {e}"));
                self.settings.mcp_enabled = false; // don't retry every frame
                self.settings.save();
            }
        }
    }

    pub(super) fn poll_mcp(&mut self, ctx: &egui::Context) {
        // finish blocking jobs (export.video / media.convert) — reply when their thread is done
        if !self.mcp_jobs.is_empty() {
            let mut i = 0;
            while i < self.mcp_jobs.len() {
                if self.mcp_jobs[i].prog.is_done() {
                    let j = self.mcp_jobs.remove(i);
                    let r = match j.prog.error() {
                        // ws:job-completion-hitches: `status` = the worker's last line (timeline.import's
                        // clip/track/missing counts); "Done" for the older jobs
                        None => Ok(json!({"ok": true, "path": j.out.to_string_lossy(), "status": j.prog.status()})),
                        Some(e) => Err(e),
                    };
                    let _ = j.reply.send(r);
                } else {
                    i += 1;
                }
            }
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // one per frame: render.frame blocks the UI thread for up to 3 s, so a queue must not run in one go
        let call = self.mcp.as_ref().and_then(|(_, rx)| rx.try_recv().ok());
        if let Some(c) = call {
            self.handle_tool(c);
            ctx.request_repaint(); // anything else queued runs on the next frames
        }
    }

    pub(super) fn handle_tool(&mut self, call: mcp::ToolCall) {
        let mcp::ToolCall { name, args, reply } = call;
        // ---- ws:registries-schema-hooks ----
        // Every tool (including the two ToolKind::Job ones) now runs through its own ToolDef.run,
        // replacing the old hand-matched "export.video" | "media.convert" arm and the old hand-kept
        // mutating-tool name-set check (ToolKind::Mutate, via run_snapshot_if_mutate). An unknown name
        // has no ToolDef: the "unknown tool" error matches run_tool's own fallback wording exactly.
        let Some(def) = mcp::tools::find(&name) else {
            let _ = reply.send(Err(format!("unknown tool '{name}'")));
            return;
        };
        let before = self.run_snapshot_if_mutate(def);
        match (def.run)(self, &args) {
            Ok(ToolOutcome::Job(prog, out)) => self.mcp_jobs.push(McpJob { prog, reply, out }),
            Ok(ToolOutcome::Done(v)) => {
                if let Some(snap) = before {
                    if snap != self.project.to_json() {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                    }
                    self.after_edit();
                }
                let _ = reply.send(Ok(v));
            }
            Err(e) => {
                if let Some(snap) = before {
                    // a failed tool is a no-op: some arms mutate before returning Err (subtitles.set, clip.set)
                    self.run_rollback(snap);
                }
                let _ = reply.send(Err(e));
            }
        }
    }

    // ---- ws:command-palette ----
    /// Run one non-`Job` tool by name outside the MCP/scripting call sites (`handle_tool`/`run_script`
    /// above), which already had their own inline snapshot/undo logic before this workstream needed a
    /// THIRD caller: the palette's Enter/arg-form-Run path and the `scripts.run` MCP tool
    /// (`tools_commands.rs`). Same shape as `handle_tool`'s non-`Job` arms (snapshot iff `Mutate`, push
    /// undo iff the JSON actually changed, `after_edit`, rollback on `Err`) — a small shared wrapper is
    /// less code than a third copy of that logic, and a smaller diff than refactoring `handle_tool`/
    /// `run_script` (each has its own reply-channel / per-script-undo shape) around a new abstraction.
    pub(crate) fn run_tool_undoable(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let def = mcp::tools::find(name).ok_or_else(|| format!("unknown tool '{name}'"))?;
        let before = self.run_snapshot_if_mutate(def);
        match (def.run)(self, args) {
            Ok(ToolOutcome::Done(v)) => {
                if let Some(snap) = before {
                    if snap != self.project.to_json() {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                    }
                    self.after_edit();
                }
                Ok(v)
            }
            Ok(ToolOutcome::Job(..)) => Err(format!("'{name}' starts a background job — not runnable from here")),
            Err(e) => {
                if let Some(snap) = before {
                    self.run_rollback(snap); // a failed tool is a no-op
                }
                Err(e)
            }
        }
    }

    pub(super) fn start_tool_job(&mut self, name: &str, args: &Value) -> Result<(Arc<Progress>, PathBuf), String> {
        if media::ffpipe::ffmpeg_exe().is_none() {
            return Err("ffmpeg.exe not found".into());
        }
        let out_size = match (arg_u64(args, "width"), arg_u64(args, "height")) {
            (Some(w), Some(h)) => Some((w as u32, h as u32)),
            _ => None,
        };
        let scaler = arg_str(args, "scaler").unwrap_or(&self.settings.export_scaler).to_string();
        if name == "export.video" {
            if self.export.is_some() {
                return Err("an export is already running".into());
            }
            let out = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let opts = ExportOptions {
                out_path: out.clone(),
                encoder: arg_str(args, "encoder").unwrap_or(&self.settings.encoder).to_string(),
                crf: arg_u64(args, "crf").map(|c| c as u32).unwrap_or(self.settings.crf),
                preset: self.settings.preset.clone(),
                backend: self.backend(),
                out_size,
                scaler,
                frames: self.export_frames(),
                metadata: Vec::new(),
                // ---- ws:export-deliver ----
                // explicit, so export.video keeps today's command line byte for byte
                range: None,
                loudnorm: false,
                letterbox: false,
            };
            let mut project = self.export_project();
            // MCP exports have no background opt-in either — "use_project_bg": true opts in per call
            if !arg_bool(args, "use_project_bg").unwrap_or(false) {
                project.preview_bg = crate::model::BackgroundMode::Black;
            }
            let prog = export::start_export(project, opts, self.text.clone());
            // same slot the UI uses: exclusion, the progress/Cancel window and the close guard all key off it
            self.export = Some((prog.clone(), ExportKind::File { path: out.clone() }));
            Ok((prog, out))
        } else {
            let src = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let ext = req(arg_str(args, "ext"), "ext")?.trim_start_matches('.').to_string();
            // never write over the source (converting to its own container used to do exactly that)
            let out = converted_path(&src, &ext);
            let opts = crate::engine::convert::ConvertOptions {
                src,
                out: out.clone(),
                encoder: self.settings.encoder.clone(),
                crf: self.settings.crf,
                preset: self.settings.preset.clone(),
                out_size,
                scaler,
                gif_fps: 15,
                target_bytes: arg_u64(args, "target_bytes"),
                // ---- ws:export-deliver ----
                vf_extra: None,
                af_extra: None,
            };
            Ok((crate::engine::convert::start_convert(opts), out))
        }
    }

    /// Execute one (non-job) MCP tool by name, trying each group's dispatch chain in turn. Kept as a
    /// single, name-only entry point (every `ToolDef.run` in the non-Job tool groups is a thin wrapper
    /// calling into exactly this chain) even though `handle_tool`/`run_script` now go through
    /// `ToolDef.run` directly for the undo-kind dispatch — a generic "run any tool by name" fn is worth
    /// keeping as one definition rather than none.
    #[allow(dead_code)]
    pub(super) fn run_tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        if let Some(r) = tools_timeline::dispatch(self, name, args) {
            return r;
        }
        if let Some(r) = tools_media::dispatch(self, name, args) {
            return r;
        }
        if let Some(r) = tools_clip::dispatch(self, name, args) {
            return r;
        }
        if let Some(r) = tools_subtitles::dispatch(self, name, args) {
            return r;
        }
        if let Some(r) = tools_playback::dispatch(self, name, args) {
            return r;
        }
        Err(format!("unknown tool '{name}'"))
    }
}
