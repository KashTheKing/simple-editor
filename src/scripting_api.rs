//! ---- feat/script-intellisense ----
//! The one description of the Luau API a script can use: every global, table, function (parameters with
//! types, the return type, a one-line doc) and value the VMs in `scripting.rs` and `engine::plugins`
//! register, the Luau standard library their sandboxes keep, and Luau's keywords. The Script Editor's
//! completion, signature help, hover and unknown-global check read it, and `docs/scripting-api.md` is
//! written from it (`markdown`).
//!
//! It cannot drift from the registrations: `tests::api_matches_the_registered_names` starts each VM the
//! way the app does (`scripting::run`, `run_hook`, `run_api`, a plugin's `Vm`), walks its globals and fails
//! when a registered name has no row here, or a row names something that context does not register.

/// Where a name exists (a bit set): a user script (Scripts folder, the Script Editor's Run, hotkeys,
/// `scripts.run`), a fired `-- @on` hook, the Assistant's `scripts.eval`, a plugin script or panel.
pub type Ctx = u8;
pub const SCRIPT: Ctx = 1;
pub const HOOK: Ctx = 2;
pub const AGENT: Ctx = 4;
pub const PLUGIN: Ctx = 8;
pub const ALL: Ctx = SCRIPT | HOOK | AGENT | PLUGIN;
/// The user's own scripts and hooks: the full Luau sandbox, nothing removed.
const OWN: Ctx = SCRIPT | HOOK;
/// The Assistant's runner and plugins: `library` / `project` / `query`, `json`.
const LIB: Ctx = AGENT | PLUGIN;

#[cfg_attr(not(test), allow(dead_code))]
pub const CONTEXTS: [(Ctx, &str); 4] =
    [(SCRIPT, "Script"), (HOOK, "Hook"), (AGENT, "Assistant script"), (PLUGIN, "Plugin")];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemKind {
    /// A library / namespace table (`library`, `math`).
    Table,
    Function,
    /// A field that holds a value (`math.pi`, `event`).
    Value,
}

#[derive(Clone, Copy, Debug)]
pub struct ApiItem {
    /// The full dotted name: `print`, `library.get`, `math.pi`.
    pub name: &'static str,
    pub kind: ItemKind,
    /// `name: type` pairs separated by `, ` (`...: any` for a variadic); a value's type for `Value`.
    pub params: &'static str,
    /// The return type(s), empty for none.
    pub ret: &'static str,
    pub doc: &'static str,
    pub ctx: Ctx,
}

impl ApiItem {
    /// The last part of the name (`get` of `library.get`).
    pub fn short(&self) -> &'static str {
        self.name.rsplit('.').next().unwrap_or(self.name)
    }

    /// The table it belongs to (`library` of `library.get`), or `None` for a global.
    pub fn parent(&self) -> Option<&'static str> {
        self.name.rsplit_once('.').map(|(p, _)| p)
    }

    /// The parameters as `(name, type)`.
    pub fn param_list(&self) -> Vec<(&'static str, &'static str)> {
        if self.kind != ItemKind::Function || self.params.is_empty() {
            return Vec::new();
        }
        self.params.split(", ").map(|p| p.split_once(": ").unwrap_or((p, "any"))).collect()
    }

    /// `tool(name: string, args: table?) -> any`, `math.pi: number`, `math` (a table).
    pub fn signature(&self) -> String {
        match self.kind {
            ItemKind::Table => format!("{}: table", self.name),
            ItemKind::Value => format!("{}: {}", self.name, self.params),
            ItemKind::Function => {
                let ret = if self.ret.is_empty() { String::new() } else { format!(" -> {}", self.ret) };
                format!("{}({}){ret}", self.name, self.params)
            }
        }
    }
}

const fn t(name: &'static str, doc: &'static str, ctx: Ctx) -> ApiItem {
    ApiItem { name, kind: ItemKind::Table, params: "", ret: "", doc, ctx }
}
const fn f(name: &'static str, params: &'static str, ret: &'static str, doc: &'static str, ctx: Ctx) -> ApiItem {
    ApiItem { name, kind: ItemKind::Function, params, ret, doc, ctx }
}
const fn v(name: &'static str, ty: &'static str, doc: &'static str, ctx: Ctx) -> ApiItem {
    ApiItem { name, kind: ItemKind::Value, params: ty, ret: "", doc, ctx }
}

// ---- ws:visualizer-helper ----
/// App tools made for a plugin (the Visualizer Helper add-on), called with `tool(name, args)` like every
/// tool: (name, arguments, what it does). Listed so the API page names them; a test checks each exists.
#[cfg_attr(not(test), allow(dead_code))]
pub const PLUGIN_TOOLS: &[(&str, &str, &str)] = &[
    (
        "audio.analyze",
        "{clip?, action = \"status\" | \"start\" | \"stems\", tempo_min?, tempo_max?, gate?, drops?, after?}",
        "Analyse a song in the background (stems through the Stem Separation add-on when installed): hits, drops, the calm part, the vocal envelope, onsets and vibrato, stored on the project with markers. `status` returns the state and the summary; `stems` opens Settings ▸ Add-ons",
    ),
    (
        "visualizer.template",
        "{clip?, template?, palette?, colors?, spikes?, sensitivity?, smoothing?, harder?, ball?, wave?, bolts?, beam?}",
        "Give the song's visualizer a template (Olympus Storm, Chrome Waveform, Storm Lightning) keyed from the analysis; the same inputs give the same keys",
    ),
    (
        "lyrics.karaoke",
        "{lyrics, clip?, font?, size?, y?, look?, giant?}",
        "The exact lyrics as timed Lyric Chrome text clips, each word filling on its sung syllable",
    ),
    ("visualizer.drop_fx", "{clip?, flash?, invert?, shatter?, build?, grade?}", "Flash and invert at each drop, the shatter into the biggest drop, the background grade"),
    ("media.loop", "{clip?, action = \"status\" | \"start\", seconds?}", "A GIF, picture or clip as a seamless 60 fps ping-pong loop under the song (ffmpeg, in the background)"),
];

/// Luau's reserved words (`continue`, `type` and `export` are contextual, but a script reads them so).
pub const KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "else", "elseif", "end", "export", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "type", "until", "while",
];

/// The built-in type names a type annotation may use (`local n: number`).
pub const TYPES: &[&str] = &[
    "any", "boolean", "buffer", "never", "nil", "number", "string", "table", "thread", "unknown", "userdata", "vector",
];

/// One row per name. Rows that share a name have disjoint `ctx` (a test checks it).
pub const API: &[ApiItem] = &[
    // ---- the app's globals (scripting::run / run_hook / run_api, plugins' BOOT) ----
    // The hidden `editor` alias table (editor.tool / tools / log / event / json / purpose) that keeps scripts
    // written before these were globals working is left out on purpose: see `api_matches_the_registered_names`.
    f("tool", "name: string, args: table?", "any", "Call a tool by name (the MCP catalogue) and return its result", ALL),
    f("tools", "", "table", "List the tools this script may call, as {name, description} rows", ALL),
    f("print", "...: any", "", "Show a line in the script's output and log, as a toast (tables as JSON)", OWN),
    f("print", "...: any", "", "Print a line into the script's output (tables as JSON)", LIB),
    f("warn", "...: any", "", "Show a line as a warning in the script's output and log (tables as JSON)", ALL),
    v("event", "table", "The payload of the event that fired this -- @on hook", HOOK),
    f("json", "value: any", "string", "The value as compact JSON (tables) or text", LIB),
    v("purpose", "string", "What the Assistant said this script is for", AGENT),
    // ---- the prelude (scripting::PRELUDE): library ----
    t("library", "The media library: assets, bins, tags", LIB),
    f(
        "library.assets",
        "filter: table?",
        "table",
        "Every asset matching {kind, folder, nested, used, query, tags, label, ext, untagged, ids}",
        LIB,
    ),
    f("library.get", "id: number", "table?", "One asset by id", LIB),
    f("library.summary", "", "table", "Counts by kind, extension, bin and use, and the bin tree", LIB),
    f(
        "library.set",
        "ids: number | {number}, fields: table",
        "table",
        "Set {folder, tags, add_tags, remove_tags, label, description} on assets",
        LIB,
    ),
    f("library.update", "filter: table?, fields: table", "table", "Set fields on every asset matching a filter", LIB),
    f("library.move", "ids: number | {number}, bin: string", "table", "Move assets into a bin", LIB),
    f("library.tag", "ids: number | {number}, tags: {string}", "table", "Add tags to assets", LIB),
    f("library.bins", "", "table", "The bins, as a tree", LIB),
    f("library.create_bin", "bin: string", "table", "Make a bin (a path such as \"Footage/Day 1\")", LIB),
    f("library.rename_bin", "bin: string, name: string", "table", "Rename a bin", LIB),
    f("library.move_bin", "bin: string, parent: string?", "table", "Move a bin under another (or to the top)", LIB),
    f("library.delete_bin", "bin: string", "table", "Delete a bin (its assets move to its parent)", LIB),
    f("library.linked", "", "table", "The linked folders", LIB),
    f("library.linked_files", "bin: string, sub: string?", "table", "The files of a linked folder (read only)", LIB),
    f("library.smart_bins", "", "table", "The smart bins", LIB),
    // ---- project ----
    t("project", "The open project: timeline, clips, markers", LIB),
    f("project.get", "", "table", "The whole project", LIB),
    f("project.summary", "", "table", "Duration, tracks, clip counts and settings", LIB),
    f("project.timeline", "", "table", "The timeline being edited, track by track", LIB),
    f("project.tracks", "", "table", "The tracks", LIB),
    f("project.clips", "", "table", "Every clip of the timeline, with its track index and track_name", LIB),
    f("project.markers", "", "table", "The markers", LIB),
    f("project.subtitles", "", "table", "The subtitle cues", LIB),
    f("project.sequences", "", "table", "The sequences (timelines)", LIB),
    f("project.spans", "", "table", "The animated spans", LIB),
    f("project.blocks", "clip_id: number", "table", "A clip's key blocks", LIB),
    // ---- query ----
    t("query", "List helpers: filter, sort, group", LIB),
    f(
        "query.filter",
        "list: table, p: table | function",
        "table",
        "The items matching a predicate or a {field = value} table",
        LIB,
    ),
    f("query.find", "list: table, p: table | function", "any", "The first item matching", LIB),
    f("query.map", "list: table, fn: function", "table", "fn applied to each item", LIB),
    f("query.pluck", "list: table, key: string | function", "table", "One field of each item", LIB),
    f("query.ids", "list: table", "table", "The id of each item", LIB),
    f(
        "query.sort",
        "list: table, key: string | function?, desc: boolean?",
        "table",
        "A sorted copy (by id unless a key is given)",
        LIB,
    ),
    f("query.group_by", "list: table, key: string | function", "table", "Items grouped into lists by a field", LIB),
    f(
        "query.count_by",
        "list: table, key: string | function",
        "table",
        "How many items have each value of a field",
        LIB,
    ),
    f("query.sum", "list: table, key: string | function", "number", "The sum of a field", LIB),
    f("query.keys", "t: table", "table", "A table's keys, sorted", LIB),
    f("query.has", "s: any, words: string | {string}", "boolean", "s contains any of the words (ignoring case)", LIB),
    // ---- plugins (engine::plugins::runtime BOOT, panel.rs) ----
    t("plugin", "This plugin's id, name and version", PLUGIN),
    v("plugin.id", "string", "This plugin's id", PLUGIN),
    v("plugin.name", "string", "This plugin's name", PLUGIN),
    v("plugin.version", "string", "This plugin's version", PLUGIN),
    t("net", "Web requests (the network permission)", PLUGIN),
    f(
        "net.get",
        "url: string",
        "table?",
        "Fetch a URL in the background: nil until done, then {ok, status, body} or {ok = false, error}",
        PLUGIN,
    ),
    t("fs", "Files in the granted folders (the files permission)", PLUGIN),
    f("fs.read", "path: string", "string", "A text file's contents (up to 16 MB)", PLUGIN),
    f("fs.write", "path: string, text: string", "", "Write a text file (folders are made)", PLUGIN),
    f("fs.list", "path: string?", "table", "A folder's names, sorted; folders end in /", PLUGIN),
    f("fs.exists", "path: string", "boolean", "Whether a file or folder exists", PLUGIN),
    t("programs", "Start programs (the run_programs permission)", PLUGIN),
    f("programs.run", "exe: string, args: {string}?", "number", "Start a program and return its process id", PLUGIN),
    t("ui", "A panel's controls (inside draw() only)", PLUGIN),
    f("ui.label", "text: any", "", "A line of text", PLUGIN),
    f("ui.heading", "text: any", "", "A heading", PLUGIN),
    f("ui.button", "label: string", "boolean", "A button; true on the frame it was clicked", PLUGIN),
    f(
        "ui.slider",
        "label: string, value: number, min: number?, max: number?",
        "number",
        "A slider (0 to 1 unless given); the current value",
        PLUGIN,
    ),
    f(
        "ui.text",
        "label: string, value: string?, multiline: boolean?",
        "string",
        "A text field; the current text",
        PLUGIN,
    ),
    f("ui.checkbox", "label: string, value: boolean?", "boolean", "A checkbox; whether it is ticked", PLUGIN),
    f(
        "ui.dropdown",
        "label: string, value: string?, options: {string}",
        "string",
        "A drop-down list; the picked option",
        PLUGIN,
    ),
    f("ui.row", "fn: function", "", "Lay the controls fn draws side by side", PLUGIN),
    f("ui.column", "fn: function", "", "Lay the controls fn draws one under another", PLUGIN),
    f("ui.separator", "", "", "A dividing line", PLUGIN),
    f("ui.space", "px: number?", "", "Empty space (8 px unless given)", PLUGIN),
    f("ui.image", "file: string, w: number?, h: number?", "", "A .png from the plugin's folder", PLUGIN),
    // ---- ws:visualizer-helper ----
    f("ui.tip", "text: any", "", "Hover text for the widget drawn just before it (labels stay plain, explanations go here)", PLUGIN),
    // ---- std:begin (Luau's standard library, as the sandboxes keep it) ----
    v("_G", "table", "The global environment", ALL),
    v("_VERSION", "string", "The Luau version", ALL),
    f(
        "assert",
        "value: any, message: string?",
        "any",
        "Raise an error when value is false or nil; otherwise return it",
        ALL,
    ),
    f("error", "message: any, level: number?", "never", "Raise an error", ALL),
    f("collectgarbage", "option: string?", "number", "\"count\": the memory in use, in KB", OWN | AGENT),
    f("gcinfo", "", "number", "The memory in use, in KB", ALL),
    f("getfenv", "target: any?", "table", "A function's environment (deprecated)", OWN),
    f("setfenv", "target: any, env: table", "any", "Set a function's environment (deprecated)", OWN),
    f("getmetatable", "t: any", "table?", "A value's metatable", ALL),
    f("setmetatable", "t: table, mt: table?", "table", "Set a table's metatable and return the table", ALL),
    f("ipairs", "t: table", "function, table, number", "Iterate over t[1], t[2], ... until a nil", ALL),
    f("pairs", "t: table", "function, table, nil", "Iterate over every key and value of a table", ALL),
    f("next", "t: table, key: any?", "any, any", "The key and value after key in a table", ALL),
    f("newproxy", "mt: boolean?", "userdata", "An empty userdata, with a metatable if asked", ALL),
    f(
        "pcall",
        "fn: function, ...: any",
        "boolean, ...any",
        "Call fn and catch its error: true and its results, or false and the error",
        ALL,
    ),
    f(
        "xpcall",
        "fn: function, handler: function, ...: any",
        "boolean, ...any",
        "Call fn; an error goes through handler",
        ALL,
    ),
    f("rawequal", "a: any, b: any", "boolean", "a == b without metamethods", ALL),
    f("rawget", "t: table, key: any", "any", "t[key] without metamethods", ALL),
    f("rawset", "t: table, key: any, value: any", "table", "t[key] = value without metamethods", ALL),
    f("rawlen", "t: table | string", "number", "The length without metamethods", ALL),
    f("require", "path: string", "any", "Load a module file", OWN),
    f(
        "select",
        "index: number | string, ...: any",
        "any",
        "The arguments from index on, or their count with \"#\"",
        ALL,
    ),
    f("tonumber", "value: any, base: number?", "number?", "The value as a number, or nil", ALL),
    f("tostring", "value: any", "string", "The value as text", ALL),
    f(
        "type",
        "value: any",
        "string",
        "The basic type name (nil, boolean, number, string, table, function, thread, userdata)",
        ALL,
    ),
    f("typeof", "value: any", "string", "The type name, including userdata types", ALL),
    f("unpack", "t: table, i: number?, j: number?", "...any", "t[i] to t[j] as separate values", ALL),
    t("package", "Module loading (require)", OWN),
    v("package.path", "string", "Where require looks for modules", OWN),
    v("package.loaded", "table", "The modules loaded so far", OWN),
    v("package.loaders", "table", "The functions require tries in turn", OWN),
    t("debug", "Stack inspection", OWN),
    f(
        "debug.info",
        "level: number | function, what: string",
        "...any",
        "Facts about a stack level or function (\"s\" source, \"l\" line, \"n\" name, \"a\" arity, \"f\" function)",
        OWN,
    ),
    f("debug.traceback", "message: string?, level: number?", "string", "The call stack as text", OWN),
    t("bit32", "Bitwise operations on 32-bit integers", ALL),
    f("bit32.arshift", "n: number, i: number", "number", "Arithmetic shift right", ALL),
    f("bit32.band", "...: number", "number", "Bitwise and", ALL),
    f("bit32.bnot", "n: number", "number", "Bitwise not", ALL),
    f("bit32.bor", "...: number", "number", "Bitwise or", ALL),
    f("bit32.btest", "...: number", "boolean", "Whether the bitwise and is not zero", ALL),
    f("bit32.bxor", "...: number", "number", "Bitwise exclusive or", ALL),
    f("bit32.byteswap", "n: number", "number", "Reverse the byte order", ALL),
    f("bit32.countlz", "n: number", "number", "Count the leading zero bits", ALL),
    f("bit32.countrz", "n: number", "number", "Count the trailing zero bits", ALL),
    f(
        "bit32.extract",
        "n: number, field: number, width: number?",
        "number",
        "The bits field to field + width - 1",
        ALL,
    ),
    f("bit32.lrotate", "n: number, i: number", "number", "Rotate left", ALL),
    f("bit32.lshift", "n: number, i: number", "number", "Shift left", ALL),
    f(
        "bit32.replace",
        "n: number, v: number, field: number, width: number?",
        "number",
        "n with the bits field to field + width - 1 set from v",
        ALL,
    ),
    f("bit32.rrotate", "n: number, i: number", "number", "Rotate right", ALL),
    f("bit32.rshift", "n: number, i: number", "number", "Logical shift right", ALL),
    t("buffer", "Fixed-size binary data", ALL),
    f("buffer.create", "size: number", "buffer", "A zero-filled buffer of size bytes", ALL),
    f("buffer.fromstring", "s: string", "buffer", "A buffer holding a string's bytes", ALL),
    f("buffer.tostring", "b: buffer", "string", "A buffer's bytes as a string", ALL),
    f("buffer.len", "b: buffer", "number", "A buffer's size in bytes", ALL),
    f(
        "buffer.copy",
        "target: buffer, offset: number, source: buffer, sourceOffset: number?, count: number?",
        "",
        "Copy bytes between buffers",
        ALL,
    ),
    f("buffer.fill", "b: buffer, offset: number, value: number, count: number?", "", "Set bytes to a value", ALL),
    f("buffer.readbits", "b: buffer, bitOffset: number, bitCount: number", "number", "Read up to 32 bits", ALL),
    f(
        "buffer.writebits",
        "b: buffer, bitOffset: number, bitCount: number, value: number",
        "",
        "Write up to 32 bits",
        ALL,
    ),
    f("buffer.readi8", "b: buffer, offset: number", "number", "Read a signed 8-bit integer", ALL),
    f("buffer.readu8", "b: buffer, offset: number", "number", "Read an unsigned 8-bit integer", ALL),
    f("buffer.readi16", "b: buffer, offset: number", "number", "Read a signed 16-bit integer", ALL),
    f("buffer.readu16", "b: buffer, offset: number", "number", "Read an unsigned 16-bit integer", ALL),
    f("buffer.readi32", "b: buffer, offset: number", "number", "Read a signed 32-bit integer", ALL),
    f("buffer.readu32", "b: buffer, offset: number", "number", "Read an unsigned 32-bit integer", ALL),
    f("buffer.readf32", "b: buffer, offset: number", "number", "Read a 32-bit float", ALL),
    f("buffer.readf64", "b: buffer, offset: number", "number", "Read a 64-bit float", ALL),
    f("buffer.readstring", "b: buffer, offset: number, count: number", "string", "Read count bytes as a string", ALL),
    f("buffer.writei8", "b: buffer, offset: number, value: number", "", "Write a signed 8-bit integer", ALL),
    f("buffer.writeu8", "b: buffer, offset: number, value: number", "", "Write an unsigned 8-bit integer", ALL),
    f("buffer.writei16", "b: buffer, offset: number, value: number", "", "Write a signed 16-bit integer", ALL),
    f("buffer.writeu16", "b: buffer, offset: number, value: number", "", "Write an unsigned 16-bit integer", ALL),
    f("buffer.writei32", "b: buffer, offset: number, value: number", "", "Write a signed 32-bit integer", ALL),
    f("buffer.writeu32", "b: buffer, offset: number, value: number", "", "Write an unsigned 32-bit integer", ALL),
    f("buffer.writef32", "b: buffer, offset: number, value: number", "", "Write a 32-bit float", ALL),
    f("buffer.writef64", "b: buffer, offset: number, value: number", "", "Write a 64-bit float", ALL),
    f("buffer.writestring", "b: buffer, offset: number, s: string, count: number?", "", "Write a string's bytes", ALL),
    t("coroutine", "Coroutines", ALL),
    f("coroutine.create", "fn: function", "thread", "A coroutine that runs fn", ALL),
    f("coroutine.resume", "co: thread, ...: any", "boolean, ...any", "Start or continue a coroutine", ALL),
    f("coroutine.yield", "...: any", "...any", "Pause the running coroutine", ALL),
    f("coroutine.status", "co: thread", "string", "\"running\", \"suspended\", \"normal\" or \"dead\"", ALL),
    f("coroutine.running", "", "thread?", "The running coroutine", ALL),
    f("coroutine.isyieldable", "", "boolean", "Whether the running code may yield", ALL),
    f("coroutine.wrap", "fn: function", "function", "A function that resumes a new coroutine each call", ALL),
    f("coroutine.close", "co: thread", "boolean, any", "Close a suspended coroutine", ALL),
    t("math", "Mathematical functions", ALL),
    f("math.abs", "x: number", "number", "The absolute value", ALL),
    f("math.acos", "x: number", "number", "The arc cosine, in radians", ALL),
    f("math.asin", "x: number", "number", "The arc sine, in radians", ALL),
    f("math.atan", "x: number", "number", "The arc tangent, in radians", ALL),
    f("math.atan2", "y: number, x: number", "number", "The angle of the point (x, y), in radians", ALL),
    f("math.ceil", "x: number", "number", "Round up to a whole number", ALL),
    f("math.clamp", "x: number, min: number, max: number", "number", "x kept between min and max", ALL),
    f("math.cos", "x: number", "number", "The cosine of an angle in radians", ALL),
    f("math.cosh", "x: number", "number", "The hyperbolic cosine", ALL),
    f("math.deg", "x: number", "number", "Radians to degrees", ALL),
    f("math.exp", "x: number", "number", "e to the power x", ALL),
    f("math.floor", "x: number", "number", "Round down to a whole number", ALL),
    f("math.fmod", "x: number, y: number", "number", "The remainder of x / y, with the sign of x", ALL),
    f("math.frexp", "x: number", "number, number", "The mantissa and exponent of x", ALL),
    f("math.ldexp", "m: number, e: number", "number", "m times 2 to the power e", ALL),
    f("math.lerp", "a: number, b: number, t: number", "number", "The value t of the way from a to b", ALL),
    f("math.log", "x: number, base: number?", "number", "The logarithm (natural unless a base is given)", ALL),
    f("math.log10", "x: number", "number", "The base-10 logarithm", ALL),
    f(
        "math.map",
        "x: number, inMin: number, inMax: number, outMin: number, outMax: number",
        "number",
        "x moved from one range to another",
        ALL,
    ),
    f("math.max", "x: number, ...: number", "number", "The largest argument", ALL),
    f("math.min", "x: number, ...: number", "number", "The smallest argument", ALL),
    f("math.modf", "x: number", "number, number", "The whole and fractional parts", ALL),
    f("math.noise", "x: number, y: number?, z: number?", "number", "Perlin noise between -1 and 1", ALL),
    f("math.pow", "x: number, y: number", "number", "x to the power y", ALL),
    f("math.rad", "x: number", "number", "Degrees to radians", ALL),
    f("math.random", "m: number?, n: number?", "number", "A random number: 0 to 1, 1 to m, or m to n", ALL),
    f("math.randomseed", "seed: number", "", "Seed the random numbers", ALL),
    f("math.round", "x: number", "number", "Round to the nearest whole number", ALL),
    f("math.sign", "x: number", "number", "-1, 0 or 1", ALL),
    f("math.sin", "x: number", "number", "The sine of an angle in radians", ALL),
    f("math.sinh", "x: number", "number", "The hyperbolic sine", ALL),
    f("math.sqrt", "x: number", "number", "The square root", ALL),
    f("math.tan", "x: number", "number", "The tangent of an angle in radians", ALL),
    f("math.tanh", "x: number", "number", "The hyperbolic tangent", ALL),
    v("math.pi", "number", "The ratio of a circle's circumference to its diameter", ALL),
    v("math.huge", "number", "Infinity", ALL),
    t("os", "The clock and dates", ALL),
    f("os.clock", "", "number", "Seconds of processor time, for timing code", ALL),
    f("os.date", "format: string?, time: number?", "string | table", "A date as text, or as a table with \"*t\"", ALL),
    f("os.difftime", "t2: number, t1: number", "number", "Seconds from t1 to t2", ALL),
    f("os.time", "date: table?", "number", "The time now (or of a date table) in seconds", ALL),
    t("string", "Text functions (also as methods: s:upper())", ALL),
    f("string.byte", "s: string, i: number?, j: number?", "...number", "The byte values of characters i to j", ALL),
    f("string.char", "...: number", "string", "A string of the given byte values", ALL),
    f(
        "string.find",
        "s: string, pattern: string, init: number?, plain: boolean?",
        "number?, number?",
        "Where a pattern first matches: start and end",
        ALL,
    ),
    f("string.format", "fmt: string, ...: any", "string", "Text with %d, %s, %.2f... filled in", ALL),
    f("string.gmatch", "s: string, pattern: string", "function", "Iterate over a pattern's matches", ALL),
    f(
        "string.gsub",
        "s: string, pattern: string, repl: string | table | function, n: number?",
        "string, number",
        "Replace a pattern's matches",
        ALL,
    ),
    f("string.len", "s: string", "number", "The length in bytes", ALL),
    f("string.lower", "s: string", "string", "In lower case", ALL),
    f(
        "string.match",
        "s: string, pattern: string, init: number?",
        "...string",
        "A pattern's first match (or its captures)",
        ALL,
    ),
    f("string.pack", "fmt: string, ...: any", "string", "Values packed into binary", ALL),
    f("string.packsize", "fmt: string", "number", "The size string.pack makes for a format", ALL),
    f("string.rep", "s: string, n: number, sep: string?", "string", "s repeated n times", ALL),
    f("string.reverse", "s: string", "string", "Reversed", ALL),
    f("string.split", "s: string, sep: string?", "{string}", "Split at a separator (a comma unless given)", ALL),
    f(
        "string.sub",
        "s: string, i: number, j: number?",
        "string",
        "Characters i to j (negative counts from the end)",
        ALL,
    ),
    f("string.unpack", "fmt: string, s: string, pos: number?", "...any", "Values unpacked from binary", ALL),
    f("string.upper", "s: string", "string", "In upper case", ALL),
    t("table", "List and table functions", ALL),
    f("table.clear", "t: table", "", "Remove every key, keeping the memory", ALL),
    f("table.clone", "t: table", "table", "A shallow copy", ALL),
    f(
        "table.concat",
        "t: table, sep: string?, i: number?, j: number?",
        "string",
        "The list's items joined into text",
        ALL,
    ),
    f("table.create", "n: number, value: any?", "table", "A list of n items, filled with value", ALL),
    f("table.find", "t: table, value: any, init: number?", "number?", "The index of a value in a list", ALL),
    f("table.foreach", "t: table, fn: function", "any", "Call fn(key, value) for each pair (deprecated)", ALL),
    f("table.foreachi", "t: table, fn: function", "any", "Call fn(index, value) for each item (deprecated)", ALL),
    f("table.freeze", "t: table", "table", "Make a table read-only", ALL),
    f("table.isfrozen", "t: table", "boolean", "Whether a table is read-only", ALL),
    f("table.getn", "t: table", "number", "The list's length (deprecated: use #t)", ALL),
    f("table.insert", "t: table, pos: number | any, value: any?", "", "Add an item at the end, or at pos", ALL),
    f("table.maxn", "t: table", "number", "The largest number key", ALL),
    f("table.move", "a: table, f: number, e: number, t: number, b: table?", "table", "Copy a[f..e] to b[t..]", ALL),
    f("table.pack", "...: any", "table", "The arguments as a list with n", ALL),
    f("table.remove", "t: table, pos: number?", "any", "Remove and return the last item, or the one at pos", ALL),
    f("table.sort", "t: table, less: function?", "", "Sort a list in place", ALL),
    f("table.unpack", "t: table, i: number?, j: number?", "...any", "The list's items as separate values", ALL),
    t("utf8", "UTF-8 text", ALL),
    f("utf8.char", "...: number", "string", "A string of the given code points", ALL),
    v("utf8.charpattern", "string", "A pattern matching one UTF-8 character", ALL),
    f("utf8.codepoint", "s: string, i: number?, j: number?", "...number", "The code points of characters i to j", ALL),
    f("utf8.codes", "s: string", "function", "Iterate over byte positions and code points", ALL),
    f("utf8.len", "s: string, i: number?, j: number?", "number?", "The number of characters", ALL),
    f("utf8.offset", "s: string, n: number, i: number?", "number?", "The byte position of the n-th character", ALL),
    t("vector", "3D vectors", ALL),
    f("vector.create", "x: number, y: number, z: number", "vector", "A vector", ALL),
    v("vector.zero", "vector", "The vector (0, 0, 0)", ALL),
    v("vector.one", "vector", "The vector (1, 1, 1)", ALL),
    f("vector.magnitude", "v: vector", "number", "The length", ALL),
    f("vector.normalize", "v: vector", "vector", "The same direction, length 1", ALL),
    f("vector.cross", "a: vector, b: vector", "vector", "The cross product", ALL),
    f("vector.dot", "a: vector, b: vector", "number", "The dot product", ALL),
    f(
        "vector.angle",
        "a: vector, b: vector, axis: vector?",
        "number",
        "The angle between two vectors, in radians",
        ALL,
    ),
    f("vector.abs", "v: vector", "vector", "Each part's absolute value", ALL),
    f("vector.ceil", "v: vector", "vector", "Each part rounded up", ALL),
    f("vector.floor", "v: vector", "vector", "Each part rounded down", ALL),
    f("vector.sign", "v: vector", "vector", "Each part's sign", ALL),
    f("vector.clamp", "v: vector, min: vector, max: vector", "vector", "Each part kept between min and max", ALL),
    f("vector.max", "...: vector", "vector", "The largest of each part", ALL),
    f("vector.min", "...: vector", "vector", "The smallest of each part", ALL),
    // ---- std:end ----
];

/// The rows that exist in `ctx`.
pub fn items(ctx: Ctx) -> impl Iterator<Item = &'static ApiItem> {
    API.iter().filter(move |i| i.ctx & ctx != 0)
}

/// The row for `name` in `ctx`.
pub fn find(name: &str, ctx: Ctx) -> Option<&'static ApiItem> {
    items(ctx).find(|i| i.name == name)
}

/// The members of table `parent` in `ctx` (`library` -> assets, get, summary...).
pub fn members(parent: &str, ctx: Ctx) -> impl Iterator<Item = &'static ApiItem> + '_ {
    items(ctx).filter(move |i| i.parent() == Some(parent))
}

/// The globals of `ctx` (names without a dot).
pub fn globals(ctx: Ctx) -> impl Iterator<Item = &'static ApiItem> {
    items(ctx).filter(|i| i.parent().is_none())
}

#[cfg_attr(not(test), allow(dead_code))]
fn ctx_names(c: Ctx) -> String {
    if c == ALL {
        return "all".into();
    }
    CONTEXTS.iter().filter(|(b, _)| c & b != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(", ")
}

/// `docs/scripting-api.md`: the whole description as a reference page.
#[cfg_attr(not(test), allow(dead_code))]
pub fn markdown() -> String {
    let mut s = String::from(
        "# Luau scripting API\n\n\
         <!-- Generated from `src/scripting_api.rs` (`scripting_api::markdown`). Do not edit by hand: run\n\
         `SE_WRITE_API_DOCS=1 cargo test api_docs_are_current` after changing the API. -->\n\n\
         What a Simple Editor script can call. The Script Editor's completion, signature help and hover read the same\n\
         table. The *Where* column says which scripts have a name:\n\n\
         - **Script**: a script in the Scripts folder (Run, a hotkey, `scripts.run`).\n\
         - **Hook**: the same script fired by a `-- @on <event>` header (adds `event`).\n\
         - **Assistant script**: `scripts.eval`, the Assistant's runner (adds `library`, `project`, `query`, `json`, `purpose`).\n\
         - **Plugin**: a plugin's script or panel (`docs/plugins.md`).\n\n",
    );
    let mut tables: Vec<&str> = Vec::new();
    for i in API {
        let top = i.name.split('.').next().unwrap_or(i.name);
        let group = if i.parent().is_none() && i.kind != ItemKind::Table { "" } else { top };
        if !tables.contains(&group) {
            tables.push(group);
        }
    }
    for g in tables {
        let title = if g.is_empty() { "Globals".to_string() } else { format!("`{g}`") };
        s.push_str(&format!("## {title}\n\n| Name | Where | Description |\n|---|---|---|\n"));
        for i in API.iter().filter(|i| {
            let top = i.name.split('.').next().unwrap_or(i.name);
            if g.is_empty() {
                i.parent().is_none() && i.kind != ItemKind::Table
            } else {
                top == g
            }
        }) {
            let sig = i.signature().replace('|', "\\|");
            s.push_str(&format!("| `{sig}` | {} | {} |\n", ctx_names(i.ctx), i.doc.replace('|', "\\|")));
        }
        s.push('\n');
    }
    // ---- ws:visualizer-helper ----
    s.push_str("## Tools for plugins\n\nApp tools made for a plugin (the Visualizer Helper add-on). Call them with `tool(name, args)`.\n\n| Tool | Arguments | Description |\n|---|---|---|\n");
    for (name, args, doc) in PLUGIN_TOOLS {
        s.push_str(&format!("| `{name}` | `{}` | {} |\n", args.replace('|', "\\|"), doc.replace('|', "\\|")));
    }
    s.push('\n');
    s.push_str("## Keywords\n\n");
    s.push_str(&KEYWORDS.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(" "));
    s.push_str("\n\n## Types\n\n");
    s.push_str(&TYPES.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(" "));
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    // ---- ws:visualizer-helper ----
    #[test]
    fn plugin_tools_are_real_tools() {
        for (name, _, _) in PLUGIN_TOOLS {
            assert!(crate::mcp::tools::find(name).is_some(), "{name} is not a tool");
        }
    }

    /// Lists every global of the VM it runs in and every string key of each global table. `pairs(_G)` sees
    /// the frozen globals; what a runner adds after `sandbox(true)` lives in the script's own environment,
    /// which a sandboxed script cannot list, so each candidate name is also read directly.
    fn walker(candidates: &BTreeSet<String>) -> String {
        let mut s = String::from(
            r#"
local out, seen = {}, {}
local function add(name) if not seen[name] then seen[name] = true; table.insert(out, name) end end
local function walk(k, v)
  add(k)
  -- `event` is a hook's payload: its fields are the event's, not the API's
  if type(v) == "table" and k ~= "_G" and k ~= "event" then
    for k2 in pairs(v) do if type(k2) == "string" then add(k .. "." .. k2) end end
  end
end
for k, v in pairs(_G) do if type(k) == "string" then walk(k, v) end end
"#,
        );
        for c in candidates {
            s.push_str(&format!("do local v = {c}; if v ~= nil then walk(\"{c}\", v) end end\n"));
        }
        s.push_str("table.sort(out)\nreturn table.concat(out, \" \")\n");
        s
    }

    /// The globals the registration code names: `globals().set("x"`, and `x = ` / `function x.` at the start
    /// of a line of the Luau the runners load (the prelude, the plugin boot code), plus every described one.
    fn candidates() -> BTreeSet<String> {
        let mut c: BTreeSet<String> = API.iter().filter(|i| i.parent().is_none()).map(|i| i.name.to_string()).collect();
        let sources = [
            include_str!("scripting.rs"),
            include_str!("engine/plugins/runtime.rs"),
            include_str!("engine/plugins/panel.rs"),
        ];
        let ident = |s: &str| s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>();
        for src in sources {
            for part in src.split("globals().set(\"").skip(1) {
                c.insert(ident(part));
            }
            for line in src.lines() {
                let l = line.strip_prefix("function ").unwrap_or(line);
                let name = ident(l);
                let rest = &l[name.len()..];
                if !name.is_empty()
                    && !KEYWORDS.contains(&name.as_str())
                    && (rest.starts_with(" = ") || rest.starts_with('.'))
                {
                    c.insert(name);
                }
            }
        }
        c.retain(|n| {
            !n.is_empty() && !n.starts_with(|ch: char| ch.is_ascii_digit()) && !KEYWORDS.contains(&n.as_str())
        });
        c
    }

    fn set(s: &str) -> BTreeSet<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    fn ok_call(_: &str, _: &serde_json::Value) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }

    /// What each context's VM really registers, started the way the app starts it.
    pub(crate) fn registered(ctx: Ctx) -> BTreeSet<String> {
        let walk = walker(&candidates());
        let as_log = format!("print((function() {walk} end)())");
        match ctx {
            SCRIPT => {
                let mut logs = Vec::new();
                crate::scripting::run(&as_log, "walk", &mut ok_call, &mut logs).unwrap();
                set(&logs[0])
            }
            HOOK => {
                let mut logs = Vec::new();
                let ev = serde_json::json!({"clip_ids": []});
                crate::scripting::run_hook(
                    &as_log,
                    "walk",
                    &ev,
                    std::time::Duration::from_secs(5),
                    &mut ok_call,
                    &mut logs,
                )
                .unwrap();
                set(&logs[0])
            }
            AGENT => {
                let r = crate::scripting::run_api(&walk, "walk", "", &Default::default(), &|_| true, &mut ok_call);
                assert!(r.error.is_none(), "{:?}", r.error);
                set(r.result.as_ref().and_then(|v| v.as_str()).unwrap())
            }
            PLUGIN => {
                use crate::engine::plugins::runtime::{Ctx as PCtx, Vm};
                let c = PCtx {
                    id: "probe".into(),
                    name: "Probe".into(),
                    version: "1".into(),
                    dir: std::env::temp_dir(),
                    granted: Default::default(),
                    declared: Default::default(),
                    folders: Vec::new(),
                    net: Default::default(),
                };
                let vm = Vm::new(&c).unwrap();
                set(&vm.lua().load(&walk).eval::<String>().unwrap())
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn api_matches_the_registered_names() {
        for (c, label) in CONTEXTS {
            // `editor` is the hidden alias table (tool / tools / log / event / json / purpose) that keeps
            // saved scripts, Assistant tasks and plugins written before those were globals running. It is
            // registered on purpose but must stay out of the API (no docs, no completion), so it is not
            // compared here.
            let real: BTreeSet<String> =
                registered(c).into_iter().filter(|n| n != "editor" && !n.starts_with("editor.")).collect();
            let described: BTreeSet<String> = items(c).map(|i| i.name.to_string()).collect();
            let missing: Vec<_> = real.difference(&described).collect();
            let extra: Vec<_> = described.difference(&real).collect();
            assert!(missing.is_empty(), "{label}: registered but not in scripting_api::API: {missing:?}");
            assert!(extra.is_empty(), "{label}: in scripting_api::API but not registered: {extra:?}");
        }
    }

    #[test]
    fn rows_are_well_formed() {
        for (k, a) in API.iter().enumerate() {
            assert!(!a.doc.is_empty() && !a.doc.ends_with('.'), "{}: a one-line doc without a full stop", a.name);
            assert!(a.ctx != 0 && a.ctx & !ALL == 0, "{}", a.name);
            for b in &API[k + 1..] {
                assert!(a.name != b.name || a.ctx & b.ctx == 0, "{} is described twice for one context", a.name);
            }
            if let Some(p) = a.parent() {
                // a member's table is described for every context the member is in
                for (c, _) in CONTEXTS.iter().filter(|(c, _)| a.ctx & c != 0) {
                    assert!(find(p, *c).is_some_and(|t| t.kind == ItemKind::Table), "{}: no table {p} in {c}", a.name);
                }
            }
            for (n, ty) in a.param_list() {
                assert!(!n.is_empty() && !ty.is_empty(), "{}: parameter `{n}: {ty}`", a.name);
            }
        }
    }

    #[test]
    fn api_docs_are_current() {
        // next to src/ (the tests run from the package root)
        let src = std::path::Path::new(file!());
        let path =
            src.parent().and_then(|p| p.parent()).unwrap_or(std::path::Path::new("")).join("docs/scripting-api.md");
        let want = markdown();
        if std::env::var_os("SE_WRITE_API_DOCS").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            have == want,
            "docs/scripting-api.md is out of date: run SE_WRITE_API_DOCS=1 cargo test api_docs_are_current"
        );
    }
}
