//! ---- ws:browser-layer ----
//! A real web page as an HTML layer for the browser layer.
//!
//! Importing reuses what HTML layers have: `html::capture_mhtml` opens an address in a private Edge
//! and saves it as a browser would, `webcap::mhtml_to_folder` unpacks that into `index.html` +
//! `files/` on disk (scripts removed, nothing fetched from the network afterwards). The page then
//! becomes an ordinary linked HTML layer clip (`html_ctl::import_file_w`), drawn by the HTML engine
//! like every other one - effects, masks, fields and all.
//!
//! What this file adds: a map of what can be clicked on the page (`PageMap`, a `.json` beside the
//! page's file - or beside a plain picture of a page), a pass that turns the page's headings and
//! labels into `se:field`s (`make_editable`), and pictures of a part of a page at any scale for deep
//! zooms (`render_region`).
//!
//! Renderer note: the three things here that need the page laid out (`scan`, `make_editable`,
//! `render_region`) go through `html::with_page` only - one place to port if the HTML engine ever
//! draws with something other than Edge.

use crate::engine::export::Progress;
use crate::engine::html;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Something on the page a mouse can go to. The box is in CSS px from the page's top-left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub tag: String,
    pub role: String,
    pub text: String,
    pub href: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageMap {
    pub url: String,
    pub title: String,
    /// The page's size in CSS px.
    pub width: f64,
    pub height: f64,
    pub favicon: String,
    /// Pictures that had not arrived when the page was saved (placeholders in their place).
    pub missing: usize,
    pub elements: Vec<Element>,
}

/// Where the page map of a page file (or a picture of a page) lives.
pub fn map_path(page: &Path) -> PathBuf {
    page.with_extension("json")
}

// By hand through `Value` (which the app parses everywhere already): derived (de)serialisers for these
// two structs would be another 15 KB of exe for a file read a few times per session.
impl PageMap {
    pub fn from_json(text: &str) -> Option<PageMap> {
        let v: Value = serde_json::from_str(text).ok()?;
        let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
        let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
        let elements = v["elements"].as_array()?;
        let m = PageMap {
            url: s(&v, "url"),
            title: s(&v, "title"),
            width: f(&v, "width"),
            height: f(&v, "height"),
            favicon: s(&v, "favicon"),
            missing: f(&v, "missing") as usize,
            elements: elements
                .iter()
                .map(|e| Element {
                    tag: s(e, "tag"),
                    role: s(e, "role"),
                    text: s(e, "text"),
                    href: s(e, "href"),
                    x: f(e, "x"),
                    y: f(e, "y"),
                    w: f(e, "w"),
                    h: f(e, "h"),
                })
                .filter(|e| e.w > 0.0 && e.h > 0.0)
                .collect(),
        };
        (m.width > 0.0 && m.height > 0.0).then_some(m)
    }
    /// The map beside `page` (a page's `.html` file, or a picture of a page), if it has one.
    pub fn of(page: &str) -> Option<PageMap> {
        PageMap::from_json(&std::fs::read_to_string(map_path(Path::new(page))).ok()?)
    }
    /// The `nth` element (0 = first, in page order) that `target` names: its exact text first, then a
    /// text that starts with it, then one that contains it, then an address that contains it. Case
    /// and extra spaces do not matter.
    pub fn find(&self, target: &str, nth: usize) -> Option<&Element> {
        let want = target.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if want.is_empty() {
            return None;
        }
        let text = |e: &Element| e.text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        let tests: [&dyn Fn(&Element) -> bool; 4] = [
            &|e| text(e) == want,
            &|e| text(e).starts_with(&want),
            &|e| text(e).contains(&want),
            &|e| e.href.to_lowercase().contains(&want),
        ];
        tests.iter().find_map(|t| self.elements.iter().filter(|e| t(e)).nth(nth))
    }
}

const SCAN: &str = r#"(()=>{const out=[];
for(const e of document.querySelectorAll('a[href],button,input,select,textarea,[role=button],[role=link],[role=tab],[role=menuitem]')){
const r=e.getBoundingClientRect();if(r.width<4||r.height<4)continue;
const st=getComputedStyle(e);if(st.visibility==='hidden'||st.display==='none'||+st.opacity===0)continue;
let t=(e.innerText||e.value||e.getAttribute('aria-label')||e.title||e.placeholder||'').replace(/\s+/g,' ').trim().slice(0,120);
if(!t){const i=e.querySelector('img[alt]');if(i)t=i.alt}
if(!t)t=(e.getAttribute('data-testid')||e.id||(''+(e.className.baseVal||e.className||'')).split(' ')[0]||'').replace(/[-_]+/g,' ').trim();
out.push({tag:e.tagName.toLowerCase(),role:e.getAttribute('role')||'',text:t,href:e.getAttribute('href')||'',x:r.left+scrollX,y:r.top+scrollY,w:r.width,h:r.height});
if(out.length>=4000)break}
// landmarks the mouse (or a scroll) can be sent to although nothing happens there: headings, figures
for(const e of document.querySelectorAll('h1,h2,h3,h4,[class*=stat] p,[class*=stat] span,[class*=stat] div')){
if(e.children.length||out.length>=4400)continue;const r=e.getBoundingClientRect();if(r.width<4||r.height<4)continue;
const t=(e.textContent||'').replace(/\s+/g,' ').trim().slice(0,120);if(!t)continue;
out.push({tag:e.tagName.toLowerCase(),role:'text',text:t,href:'',x:r.left+scrollX,y:r.top+scrollY,w:r.width,h:r.height})}
const missing=[...document.images].filter(i=>i.getBoundingClientRect().width>24&&(!i.complete||!i.naturalWidth)).length
+[...document.querySelectorAll('[class*=shimmer],[class*=skeleton]')].filter(e=>e.getBoundingClientRect().width>24).length;
const h=Math.max(document.documentElement.scrollHeight,document.body?document.body.scrollHeight:0);
const ic=document.querySelector('link[rel~=icon]');
return JSON.stringify({height:h,missing:missing,title:document.title,favicon:ic?ic.getAttribute('href')||'':'',elements:out})})()"#;

/// Headings, button / link labels and short paragraphs that are plain text become `{{field}}`s (their
/// text the default), and the page background a colour field; `MAX` caps how many.
const EDITABLE: &str = r#"(()=>{const max=MAX,f=[];
const clean=s=>s.replace(/[|"<>{}]/g,' ').replace(/\s+/g,' ').trim();
const take=(sel,base,cap)=>{let n=0;for(const e of document.querySelectorAll(sel)){if(f.length>=max||n>=cap)break;
if(e.children.length||!e.textContent||e.closest('script,style,head'))continue;const t=clean(e.textContent);
if(t.length<2||t.length>80)continue;n++;const nm=base+'_'+n;f.push({name:nm,type:'text',def:t});e.textContent='{{'+nm+'}}'}};
take('h1','heading',3);take('h2','subheading',4);take('h3','title',4);take('button','button',4);take('a','link',6);take('p','text',4);
const hex=c=>{const m=c.match(/\d+(\.\d+)?/g);if(!m||m.length<3||(m.length>3&&+m[3]===0))return '';
return '#'+m.slice(0,3).map(x=>(+x|0).toString(16).padStart(2,'0')).join('')};
const bg=hex(getComputedStyle(document.body).backgroundColor)||hex(getComputedStyle(document.documentElement).backgroundColor)||'#ffffff';
f.push({name:'page_background',type:'color',def:bg});document.body.style.backgroundColor='{{page_background}}';
// pictures: the biggest ones that are files of the page, while there is room
const imgs=[...document.querySelectorAll('img[src]')].map(e=>({e,r:e.getBoundingClientRect(),s:e.getAttribute('src')}))
.filter(i=>i.r.width>=64&&i.r.height>=64&&!/^(data:|https?:|\/\/)/.test(i.s)&&!/[|"<>{}]/.test(i.s)).sort((a,b)=>b.r.width*b.r.height-a.r.width*a.r.height);
let n=0;for(const i of imgs){if(f.length>=max+1||n>=8)break;n++;const nm='image_'+n;f.push({name:nm,type:'image',def:i.s});i.e.setAttribute('src','{{'+nm+'}}');i.e.removeAttribute('srcset')}
return JSON.stringify({fields:f,html:'<!DOCTYPE html>\n'+document.documentElement.outerHTML})})()"#;

/// Stops every CSS animation and transition of an imported page.
const STILL: &str = "<style>*,*::before,*::after{animation:none!important;transition:none!important}</style>";

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The folder of a page file, as `html::prepare_in` wants it.
fn base_of(index: &Path) -> String {
    index.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Import a page: `src` is an address, a saved `.mhtml`, or an `.html` file. `width` is the layout
/// width in CSS px. Returns the page's `.html` file, ready to become a linked HTML layer; its page
/// map is written beside it. Blocking (seconds): run it on a job thread.
pub fn import(src: &str, width: u32, prog: &Progress) -> Result<PathBuf, String> {
    let width = width.clamp(320, 3840);
    let file = Path::new(src);
    let is = |ext: &str| file.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext));
    let (index, url) = if file.is_file() && (is("html") || is("htm")) {
        (file.to_path_buf(), String::new())
    } else {
        let (bytes, name, url) = if file.is_file() {
            let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            (std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?, name, String::new())
        } else {
            let url = if src.contains("://") { src.trim().to_string() } else { format!("https://{}", src.trim()) };
            let host = url.split("://").last().unwrap_or("").replace(['/', '?', '#', '&', '='], "-");
            (html::capture_mhtml(&url, width, 1080, prog)?, host, url)
        };
        let slug = crate::engine::library::slug(&name);
        let web = crate::settings::Settings::dir().join("web");
        let dir = web.join(format!("{}-{:08x}", if slug.is_empty() { "page" } else { &slug }, hash(src) as u32));
        let _ = std::fs::remove_dir_all(&dir);
        (crate::engine::webcap::mhtml_to_folder(&bytes, &dir)?, url)
    };
    prog.set(0.9, "Reading the page's links and buttons…");
    let mut text = std::fs::read_to_string(&index).map_err(|e| format!("{}: {e}", index.display()))?;
    // A saved page is a still: its loading shimmers and hover fades would make the HTML engine render
    // it as a film (minutes, and a new decode for every zoom step). Only in a folder this import
    // made - a .html file of the user's own is left as it is.
    if !url.is_empty() || !is("html") && !is("htm") {
        if let Some(i) = text.to_ascii_lowercase().find("</head>").filter(|_| !text.contains(STILL)) {
            text.insert_str(i, STILL);
            std::fs::write(&index, &text).map_err(|e| format!("{}: {e}", index.display()))?;
        }
    }
    let map = scan(&text, &base_of(&index), width, &url)?;
    std::fs::write(map_path(&index), map).map_err(|e| format!("page map: {e}"))?;
    Ok(index)
}

/// The page map of HTML `text` laid out at `width`, as JSON.
fn scan(text: &str, base: &str, width: u32, url: &str) -> Result<String, String> {
    let page = html::prepare_in(text, &[], base).1;
    let found = html::with_page(&page, width, &mut |s| s.eval(SCAN))?;
    let mut v: Value = serde_json::from_str(found.as_str().unwrap_or("")).map_err(|e| format!("page scan: {e}"))?;
    v["width"] = json!(width);
    v["url"] = json!(url);
    // a page icon that came with the page is a file beside it
    let icon = v["favicon"].as_str().map(|f| Path::new(base).join(f)).filter(|p| p.is_file());
    v["favicon"] = json!(icon.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    Ok(v.to_string())
}

/// "Make editable": the page's source with up to `max` of its headings, labels and short paragraphs
/// turned into `se:field`s (plus the page background as a colour), so they are edited in the
/// Inspector like any HTML layer's fields. Returns (new source, how many fields). Blocking.
pub fn make_editable(src: &str, base: &str, width: u32, max: usize) -> Result<(String, usize), String> {
    let page = html::prepare_in(src, &[], base).1;
    let js = EDITABLE.replace("MAX", &max.clamp(1, 40).to_string());
    let found = html::with_page(&page, width, &mut |s| s.eval(&js))?;
    let v: Value = serde_json::from_str(found.as_str().unwrap_or("")).map_err(|e| format!("make editable: {e}"))?;
    let fields = v["fields"].as_array().ok_or("make editable: no fields")?;
    // the loaded page carried the <base> the engine adds while rendering: the source must not
    let mut out = html::strip_base_tags(v["html"].as_str().ok_or("make editable: no page")?);
    let mut metas = String::new();
    let mut made = 0;
    for f in fields {
        let s = |k: &str| f[k].as_str().unwrap_or("");
        let mut def = s("def").to_string();
        if s("type") == "image" {
            // an image field holds a file: the page's own picture, by its full path - or, when that
            // file is not there, the picture stays as it was and is no field
            let file = Path::new(base).join(&def);
            if !file.is_file() {
                out = out.replace(&format!("{{{{{}}}}}", s("name")), &def);
                continue;
            }
            def = file.to_string_lossy().into_owned();
        }
        made += 1;
        metas.push_str(&format!("<meta name=\"se:field\" content=\"{} | {} | {}\">\n", s("name"), s("type"), html::escape(&def)));
    }
    let at = out.find("<head").and_then(|i| out[i..].find('>').map(|j| i + j + 1)).ok_or("the page has no <head>")?;
    Ok((format!("{}\n{metas}{}", &out[..at], &out[at..]), made))
}

/// A picture of the part `rect` (x, y, w, h in CSS px) of an HTML page (its source, field values,
/// folder and layout width) at `scale` picture px per CSS px, drawn at that scale: text and vector
/// art are sharp at any zoom. Kept in the HTML render cache; asked for again it is the same file.
/// The scale is lowered when the picture would exceed `budget` pixels. Blocking.
pub fn render_region(
    src: &str,
    fields: &[(String, String)],
    base: &str,
    width: u32,
    rect: [f64; 4],
    scale: f64,
    budget: f64,
) -> Result<PathBuf, String> {
    let (w, h) = (rect[2].max(1.0), rect[3].max(1.0));
    let scale = scale.min(8192.0 / w).min(8192.0 / h).min((budget.max(1.0e6) / (w * h)).sqrt()).clamp(0.25, 64.0);
    let key = hash(&format!("{src}{fields:?}{base}{width}{rect:?}{scale}"));
    let dir = html::dir();
    let out = dir.join(format!("zoom-{key:016x}.png"));
    if !out.is_file() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let page = html::prepare_in(src, fields, base).1;
        let png = html::with_page(&page, width, &mut |s| s.shot(rect, scale))?;
        std::fs::write(&out, png).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(out)
}

/// The folder of the browser profile "Sign in to import pages as me" uses: a profile of its own, so
/// the user's everyday browser is never touched. The app only ever passes this folder to Edge; it
/// does not read, copy or log anything in it.
pub fn sign_in_profile() -> PathBuf {
    crate::settings::Settings::dir().join("browser-profile")
}

/// Open a visible Edge window on the sign-in profile at `url`, for the user to sign in themselves.
pub fn open_sign_in(url: &str) -> Result<(), String> {
    let exe = html::edge_exe().ok_or("Microsoft Edge not found")?;
    let dir = sign_in_profile();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::process::Command::new(exe)
        .arg(format!("--user-data-dir={}", dir.join("p").display()))
        .args(["--no-first-run", "--no-default-browser-check", "--new-window"])
        .arg(if url.is_empty() { "about:blank" } else { url })
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Microsoft Edge: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> PageMap {
        let text = r#"{"url":"https://site.example/charts","title":"Charts","width":1424,"height":3667,"scale":2,
            "elements":[
              {"tag":"a","role":"","text":"Charts","href":"/charts","x":10,"y":10,"w":80,"h":30},
              {"tag":"a","role":"","text":"Forsaken  97% 120K","href":"/games/18687417158/Forsaken","x":218,"y":219,"w":150,"h":240},
              {"tag":"a","role":"","text":"Forsaken Tips","href":"/games/2","x":400,"y":219,"w":150,"h":240},
              {"tag":"button","role":"button","text":"Play","href":"","x":900,"y":400,"w":300,"h":60},
              {"tag":"a","role":"","text":"","href":"/hidden","x":0,"y":0,"w":0,"h":0}
            ]}"#;
        PageMap::from_json(text).unwrap()
    }

    #[test]
    fn a_page_map_finds_elements_by_text() {
        let m = map();
        assert_eq!((m.width, m.height, m.elements.len()), (1424.0, 3667.0, 4), "the empty box is dropped");
        assert_eq!(m.find("play", 0).unwrap().x, 900.0);
        assert_eq!(m.find("  CHARTS ", 0).unwrap().href, "/charts", "exact text wins, case and spaces aside");
        // no element is called exactly "Forsaken": the first that starts with it, then the next one
        assert_eq!(m.find("Forsaken", 0).unwrap().x, 218.0);
        assert_eq!(m.find("Forsaken", 1).unwrap().x, 400.0);
        assert_eq!(m.find("tips", 0).unwrap().x, 400.0, "contained text");
        assert_eq!(m.find("18687417158", 0).unwrap().x, 218.0, "part of the address");
        assert!(m.find("nothing like this", 0).is_none() && m.find("", 0).is_none());
        assert!(PageMap::from_json("{}").is_none() && PageMap::from_json("not json").is_none());
        assert_eq!(map_path(Path::new("C:/web/site/index.html")), Path::new("C:/web/site/index.json"));
    }
}
