//! se-html: Simple Editor's HTML + CSS renderer as a helper process (Blitz, no browser, no JavaScript, no
//! network). Reads requests from stdin, answers on stdout (`proto.rs`); stderr is for humans.
//!
//! Commands (`cmd` in the request head; every response has `ok`):
//!   ping                                        -> {version, protocol, engine}
//!   load   {html, base, w, h, scale, vars?}     -> {id, height, animated, ends, period}   (a page stays loaded)
//!   frame  {id, t, vars?, ptr?, scroll?, fmt}   -> body = picture (fmt "png" | "rgba"), {w, h}
//!   frames {id, items:[{t, vars?, ptr?}], fmt}  -> body = repeated (u32 LE len | picture), {w, h, count}
//!   region {id, rect:[x,y,w,h], scale, t, fmt}  -> body = picture of part of the page at `scale`
//!   elements {id}                               -> {elements:[{tag, role, text, href, x, y, w, h}], height}
//!   free   {id}                                 -> {}
//!   quit                                        -> {} and exit
//! A panic inside the renderer (vello_cpu `unimplemented!` on colour filters, ...) is caught: the answer is
//! {"ok": false, "panic": true, "error": ...} and the page it happened on is dropped. The app then draws that
//! layer with Edge. A hang is the app's business (a per-command timeout that kills this process).

mod page;
mod proto;

use page::Page;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

static PANIC: Mutex<String> = Mutex::new(String::new());

#[repr(C)]
#[derive(Default)]
struct Pmc {
    cb: u32,
    page_faults: u32,
    peak_ws: usize,
    ws: usize,
    rest: [usize; 6],
}
#[link(name = "psapi")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(h: isize, c: *mut Pmc, cb: u32) -> i32;
}

/// (working set, peak working set) of this process in MB, for the app's status line and the benchmarks.
fn memory_mb() -> (f64, f64) {
    let mut m = Pmc { cb: std::mem::size_of::<Pmc>() as u32, ..Default::default() };
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb) };
    (m.ws as f64 / 1048576.0, m.peak_ws as f64 / 1048576.0)
}

struct State {
    pages: HashMap<u64, Page>,
    next: u64,
}

fn vars_of(v: &Value) -> Vec<(String, String)> {
    v.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    let val = v.as_f64().map(|n| n.to_string()).or_else(|| v.as_str().map(String::from)).unwrap_or_default();
                    (if k.starts_with("--") { k.clone() } else { format!("--{k}") }, val)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn picture(fmt: &str, w: u32, h: u32, rgba: Vec<u8>) -> Result<Vec<u8>, String> {
    if fmt == "rgba" {
        Ok(rgba)
    } else {
        page::encode_png(w, h, &rgba)
    }
}

fn apply(p: &mut Page, item: &Value) {
    for (n, v) in vars_of(&item["vars"]) {
        p.set_var(&n, &v);
    }
    if let Some(y) = item["scroll"].as_f64() {
        p.scroll_to(y);
    }
    let t = item["t"].as_f64().unwrap_or(0.0);
    // the pointer first, at the clock it arrived at: hover transitions start from there
    if let Some(a) = item["ptr"].as_array() {
        let g = |i: usize| a.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
        p.pointer(g(0) as f32, g(1) as f32, g(2) >= 0.5);
    }
    p.seek(t);
}

fn handle(st: &mut State, head: &Value) -> Result<(Value, Vec<u8>), String> {
    let cmd = head["cmd"].as_str().unwrap_or("");
    let page = |st: &mut State| -> Result<u64, String> {
        let id = head["id"].as_u64().ok_or("no page id")?;
        st.pages.contains_key(&id).then_some(id).ok_or_else(|| "no such page".to_string())
    };
    let fmt = head["fmt"].as_str().unwrap_or("png").to_string();
    match cmd {
        "ping" => {
            let (ws, peak) = memory_mb();
            Ok((json!({"ok": true, "version": env!("CARGO_PKG_VERSION"), "protocol": proto::PROTOCOL, "engine": "blitz 0.3.0-beta.4", "ws_mb": ws, "peak_mb": peak, "pages": st.pages.len()}), vec![]))
        }
        "load" => {
            let html = head["html"].as_str().ok_or("no html")?;
            let g = |k: &str, d: f64| head[k].as_f64().unwrap_or(d);
            let mut p = Page::load(html, head["base"].as_str().unwrap_or(""), g("w", 1920.0) as u32, g("h", 1080.0) as u32, g("scale", 1.0));
            for (n, v) in vars_of(&head["vars"]) {
                p.set_var(&n, &v);
            }
            p.seek(0.0);
            let (height, animated, ends) = (p.content_height(), p.animated, p.ends);
            st.next += 1;
            let id = st.next;
            st.pages.insert(id, p);
            Ok((json!({"ok": true, "id": id, "height": height, "animated": animated, "ends": if ends.is_finite() { json!(ends) } else { json!(-1) }}), vec![]))
        }
        "frame" => {
            let id = page(st)?;
            let p = st.pages.get_mut(&id).unwrap();
            apply(p, head);
            let (w, h, rgba) = p.paint();
            Ok((json!({"ok": true, "w": w, "h": h}), picture(&fmt, w, h, rgba)?))
        }
        "frames" => {
            let id = page(st)?;
            let p = st.pages.get_mut(&id).unwrap();
            let items = head["items"].as_array().ok_or("no items")?;
            let (mut body, mut size) = (Vec::new(), (0, 0));
            for it in items {
                apply(p, it);
                let (w, h, rgba) = p.paint();
                size = (w, h);
                let pic = picture(&fmt, w, h, rgba)?;
                body.extend_from_slice(&(pic.len() as u32).to_le_bytes());
                body.extend_from_slice(&pic);
            }
            Ok((json!({"ok": true, "w": size.0, "h": size.1, "count": items.len()}), body))
        }
        "region" => {
            let id = page(st)?;
            let p = st.pages.get_mut(&id).unwrap();
            let r: Vec<f64> = head["rect"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default();
            if r.len() != 4 {
                return Err("rect must be [x, y, w, h]".into());
            }
            let (w, h, rgba) = p.region([r[0], r[1], r[2], r[3]], head["scale"].as_f64().unwrap_or(1.0), head["t"].as_f64().unwrap_or(0.0));
            Ok((json!({"ok": true, "w": w, "h": h}), picture(&fmt, w, h, rgba)?))
        }
        "elements" => {
            let id = page(st)?;
            let p = st.pages.get(&id).unwrap();
            Ok((json!({"ok": true, "elements": p.elements(), "height": p.content_height()}), vec![]))
        }
        "free" => {
            if let Some(id) = head["id"].as_u64() {
                st.pages.remove(&id);
            }
            Ok((json!({"ok": true}), vec![]))
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("se-html {} (protocol {}, blitz 0.3.0-beta.4)", env!("CARGO_PKG_VERSION"), proto::PROTOCOL);
        return;
    }
    // a developer / test convenience: se-html --render in.html out.png [w h scale t1,t2,...]  ("{t}" in out
    // is replaced when several times are given)
    if std::env::args().nth(1).as_deref() == Some("--render") {
        let a: Vec<String> = std::env::args().collect();
        let html = std::fs::read_to_string(&a[2]).expect("html");
        let base = std::path::Path::new(&a[2]).parent().map(|p| p.canonicalize().unwrap_or_default().display().to_string()).unwrap_or_default();
        let base = base.strip_prefix(r"\\?\").unwrap_or(&base).to_string();
        let g = |i: usize, d: f64| a.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
        let times: Vec<f64> = a.get(7).map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_else(|| vec![0.0]);
        let mut p = Page::load(&html, &base, g(4, 1920.0) as u32, g(5, 1080.0) as u32, g(6, 1.0));
        for t in times {
            p.seek(t);
            let (w, h, rgba) = p.paint();
            let out = a[3].replace("{t}", &format!("{t}"));
            std::fs::write(out, page::encode_png(w, h, &rgba).expect("png")).expect("write");
        }
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("{info}");
        eprintln!("se-html panic: {msg}");
        if let Ok(mut p) = PANIC.lock() {
            *p = msg;
        }
    }));
    let mut input = BufReader::with_capacity(1 << 16, std::io::stdin().lock());
    let mut out = BufWriter::with_capacity(1 << 16, std::io::stdout().lock());
    let mut st = State { pages: HashMap::new(), next: 0 };
    loop {
        let (head, _body) = match proto::read_msg(&mut input) {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => {
                eprintln!("se-html: {e}");
                return;
            }
        };
        if head["cmd"] == "quit" {
            let _ = proto::write_msg(&mut out, &json!({"ok": true}), &[]);
            let _ = out.flush();
            return;
        }
        let r = catch_unwind(AssertUnwindSafe(|| handle(&mut st, &head)));
        let wrote = match r {
            Ok(Ok((h, body))) => proto::write_msg(&mut out, &h, &body),
            Ok(Err(e)) => proto::write_msg(&mut out, &json!({"ok": false, "error": e, "panic": false}), &[]),
            Err(_) => {
                // the page the command ran on may be half-updated: drop it
                if let Some(id) = head["id"].as_u64() {
                    st.pages.remove(&id);
                }
                let msg = PANIC.lock().map(|p| p.clone()).unwrap_or_default();
                proto::write_msg(&mut out, &json!({"ok": false, "error": msg, "panic": true}), &[])
            }
        };
        if wrote.is_err() {
            return; // the app is gone
        }
    }
}
