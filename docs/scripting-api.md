# Luau scripting API

<!-- Generated from `src/scripting_api.rs` (`scripting_api::markdown`). Do not edit by hand: run
`SE_WRITE_API_DOCS=1 cargo test api_docs_are_current` after changing the API. -->

What a Simple Editor script can call. The Script Editor's completion, signature help and hover read the same
table. The *Where* column says which scripts have a name:

- **Script**: a script in the Scripts folder (Run, a hotkey, `scripts.run`).
- **Hook**: the same script fired by a `-- @on <event>` header (adds `event`).
- **Assistant script**: `scripts.eval`, the Assistant's runner (adds `library`, `project`, `query`, `json`, `purpose`).
- **Plugin**: a plugin's script or panel (`docs/plugins.md`).

## Globals

| Name | Where | Description |
|---|---|---|
| `tool(name: string, args: table?) -> any` | all | Call a tool by name (the MCP catalogue) and return its result |
| `tools() -> table` | all | List the tools this script may call, as {name, description} rows |
| `print(...: any)` | Script, Hook | Show a line in the script's output and log, as a toast (tables as JSON) |
| `print(...: any)` | Assistant script, Plugin | Print a line into the script's output (tables as JSON) |
| `warn(...: any)` | all | Show a line as a warning in the script's output and log (tables as JSON) |
| `event: table` | Hook | The payload of the event that fired this -- @on hook |
| `json(value: any) -> string` | Assistant script, Plugin | The value as compact JSON (tables) or text |
| `purpose: string` | Assistant script | What the Assistant said this script is for |
| `_G: table` | all | The global environment |
| `_VERSION: string` | all | The Luau version |
| `assert(value: any, message: string?) -> any` | all | Raise an error when value is false or nil; otherwise return it |
| `error(message: any, level: number?) -> never` | all | Raise an error |
| `collectgarbage(option: string?) -> number` | Script, Hook, Assistant script | "count": the memory in use, in KB |
| `gcinfo() -> number` | all | The memory in use, in KB |
| `getfenv(target: any?) -> table` | Script, Hook | A function's environment (deprecated) |
| `setfenv(target: any, env: table) -> any` | Script, Hook | Set a function's environment (deprecated) |
| `getmetatable(t: any) -> table?` | all | A value's metatable |
| `setmetatable(t: table, mt: table?) -> table` | all | Set a table's metatable and return the table |
| `ipairs(t: table) -> function, table, number` | all | Iterate over t[1], t[2], ... until a nil |
| `pairs(t: table) -> function, table, nil` | all | Iterate over every key and value of a table |
| `next(t: table, key: any?) -> any, any` | all | The key and value after key in a table |
| `newproxy(mt: boolean?) -> userdata` | all | An empty userdata, with a metatable if asked |
| `pcall(fn: function, ...: any) -> boolean, ...any` | all | Call fn and catch its error: true and its results, or false and the error |
| `xpcall(fn: function, handler: function, ...: any) -> boolean, ...any` | all | Call fn; an error goes through handler |
| `rawequal(a: any, b: any) -> boolean` | all | a == b without metamethods |
| `rawget(t: table, key: any) -> any` | all | t[key] without metamethods |
| `rawset(t: table, key: any, value: any) -> table` | all | t[key] = value without metamethods |
| `rawlen(t: table \| string) -> number` | all | The length without metamethods |
| `require(path: string) -> any` | Script, Hook | Load a module file |
| `select(index: number \| string, ...: any) -> any` | all | The arguments from index on, or their count with "#" |
| `tonumber(value: any, base: number?) -> number?` | all | The value as a number, or nil |
| `tostring(value: any) -> string` | all | The value as text |
| `type(value: any) -> string` | all | The basic type name (nil, boolean, number, string, table, function, thread, userdata) |
| `typeof(value: any) -> string` | all | The type name, including userdata types |
| `unpack(t: table, i: number?, j: number?) -> ...any` | all | t[i] to t[j] as separate values |

## `library`

| Name | Where | Description |
|---|---|---|
| `library: table` | Assistant script, Plugin | The media library: assets, bins, tags |
| `library.assets(filter: table?) -> table` | Assistant script, Plugin | Every asset matching {kind, folder, nested, used, query, tags, label, ext, untagged, ids} |
| `library.get(id: number) -> table?` | Assistant script, Plugin | One asset by id |
| `library.summary() -> table` | Assistant script, Plugin | Counts by kind, extension, bin and use, and the bin tree |
| `library.set(ids: number \| {number}, fields: table) -> table` | Assistant script, Plugin | Set {folder, tags, add_tags, remove_tags, label, description} on assets |
| `library.update(filter: table?, fields: table) -> table` | Assistant script, Plugin | Set fields on every asset matching a filter |
| `library.move(ids: number \| {number}, bin: string) -> table` | Assistant script, Plugin | Move assets into a bin |
| `library.tag(ids: number \| {number}, tags: {string}) -> table` | Assistant script, Plugin | Add tags to assets |
| `library.bins() -> table` | Assistant script, Plugin | The bins, as a tree |
| `library.create_bin(bin: string) -> table` | Assistant script, Plugin | Make a bin (a path such as "Footage/Day 1") |
| `library.rename_bin(bin: string, name: string) -> table` | Assistant script, Plugin | Rename a bin |
| `library.move_bin(bin: string, parent: string?) -> table` | Assistant script, Plugin | Move a bin under another (or to the top) |
| `library.delete_bin(bin: string) -> table` | Assistant script, Plugin | Delete a bin (its assets move to its parent) |
| `library.linked() -> table` | Assistant script, Plugin | The linked folders |
| `library.linked_files(bin: string, sub: string?) -> table` | Assistant script, Plugin | The files of a linked folder (read only) |
| `library.smart_bins() -> table` | Assistant script, Plugin | The smart bins |

## `project`

| Name | Where | Description |
|---|---|---|
| `project: table` | Assistant script, Plugin | The open project: timeline, clips, markers |
| `project.get() -> table` | Assistant script, Plugin | The whole project |
| `project.summary() -> table` | Assistant script, Plugin | Duration, tracks, clip counts and settings |
| `project.timeline() -> table` | Assistant script, Plugin | The timeline being edited, track by track |
| `project.tracks() -> table` | Assistant script, Plugin | The tracks |
| `project.clips() -> table` | Assistant script, Plugin | Every clip of the timeline, with its track index and track_name |
| `project.markers() -> table` | Assistant script, Plugin | The markers |
| `project.subtitles() -> table` | Assistant script, Plugin | The subtitle cues |
| `project.sequences() -> table` | Assistant script, Plugin | The sequences (timelines) |
| `project.spans() -> table` | Assistant script, Plugin | The animated spans |
| `project.blocks(clip_id: number) -> table` | Assistant script, Plugin | A clip's key blocks |

## `query`

| Name | Where | Description |
|---|---|---|
| `query: table` | Assistant script, Plugin | List helpers: filter, sort, group |
| `query.filter(list: table, p: table \| function) -> table` | Assistant script, Plugin | The items matching a predicate or a {field = value} table |
| `query.find(list: table, p: table \| function) -> any` | Assistant script, Plugin | The first item matching |
| `query.map(list: table, fn: function) -> table` | Assistant script, Plugin | fn applied to each item |
| `query.pluck(list: table, key: string \| function) -> table` | Assistant script, Plugin | One field of each item |
| `query.ids(list: table) -> table` | Assistant script, Plugin | The id of each item |
| `query.sort(list: table, key: string \| function?, desc: boolean?) -> table` | Assistant script, Plugin | A sorted copy (by id unless a key is given) |
| `query.group_by(list: table, key: string \| function) -> table` | Assistant script, Plugin | Items grouped into lists by a field |
| `query.count_by(list: table, key: string \| function) -> table` | Assistant script, Plugin | How many items have each value of a field |
| `query.sum(list: table, key: string \| function) -> number` | Assistant script, Plugin | The sum of a field |
| `query.keys(t: table) -> table` | Assistant script, Plugin | A table's keys, sorted |
| `query.has(s: any, words: string \| {string}) -> boolean` | Assistant script, Plugin | s contains any of the words (ignoring case) |

## `plugin`

| Name | Where | Description |
|---|---|---|
| `plugin: table` | Plugin | This plugin's id, name and version |
| `plugin.id: string` | Plugin | This plugin's id |
| `plugin.name: string` | Plugin | This plugin's name |
| `plugin.version: string` | Plugin | This plugin's version |

## `net`

| Name | Where | Description |
|---|---|---|
| `net: table` | Plugin | Web requests (the network permission) |
| `net.get(url: string) -> table?` | Plugin | Fetch a URL in the background: nil until done, then {ok, status, body} or {ok = false, error} |

## `fs`

| Name | Where | Description |
|---|---|---|
| `fs: table` | Plugin | Files in the granted folders (the files permission) |
| `fs.read(path: string) -> string` | Plugin | A text file's contents (up to 16 MB) |
| `fs.write(path: string, text: string)` | Plugin | Write a text file (folders are made) |
| `fs.list(path: string?) -> table` | Plugin | A folder's names, sorted; folders end in / |
| `fs.exists(path: string) -> boolean` | Plugin | Whether a file or folder exists |

## `programs`

| Name | Where | Description |
|---|---|---|
| `programs: table` | Plugin | Start programs (the run_programs permission) |
| `programs.run(exe: string, args: {string}?) -> number` | Plugin | Start a program and return its process id |

## `ui`

| Name | Where | Description |
|---|---|---|
| `ui: table` | Plugin | A panel's controls (inside draw() only) |
| `ui.label(text: any)` | Plugin | A line of text |
| `ui.heading(text: any)` | Plugin | A heading |
| `ui.button(label: string) -> boolean` | Plugin | A button; true on the frame it was clicked |
| `ui.slider(label: string, value: number, min: number?, max: number?) -> number` | Plugin | A slider (0 to 1 unless given); the current value |
| `ui.text(label: string, value: string?, multiline: boolean?) -> string` | Plugin | A text field; the current text |
| `ui.checkbox(label: string, value: boolean?) -> boolean` | Plugin | A checkbox; whether it is ticked |
| `ui.dropdown(label: string, value: string?, options: {string}) -> string` | Plugin | A drop-down list; the picked option |
| `ui.row(fn: function)` | Plugin | Lay the controls fn draws side by side |
| `ui.column(fn: function)` | Plugin | Lay the controls fn draws one under another |
| `ui.separator()` | Plugin | A dividing line |
| `ui.space(px: number?)` | Plugin | Empty space (8 px unless given) |
| `ui.image(file: string, w: number?, h: number?)` | Plugin | A .png from the plugin's folder |
| `ui.tip(text: any)` | Plugin | Hover text for the widget drawn just before it (labels stay plain, explanations go here) |

## `package`

| Name | Where | Description |
|---|---|---|
| `package: table` | Script, Hook | Module loading (require) |
| `package.path: string` | Script, Hook | Where require looks for modules |
| `package.loaded: table` | Script, Hook | The modules loaded so far |
| `package.loaders: table` | Script, Hook | The functions require tries in turn |

## `debug`

| Name | Where | Description |
|---|---|---|
| `debug: table` | Script, Hook | Stack inspection |
| `debug.info(level: number \| function, what: string) -> ...any` | Script, Hook | Facts about a stack level or function ("s" source, "l" line, "n" name, "a" arity, "f" function) |
| `debug.traceback(message: string?, level: number?) -> string` | Script, Hook | The call stack as text |

## `bit32`

| Name | Where | Description |
|---|---|---|
| `bit32: table` | all | Bitwise operations on 32-bit integers |
| `bit32.arshift(n: number, i: number) -> number` | all | Arithmetic shift right |
| `bit32.band(...: number) -> number` | all | Bitwise and |
| `bit32.bnot(n: number) -> number` | all | Bitwise not |
| `bit32.bor(...: number) -> number` | all | Bitwise or |
| `bit32.btest(...: number) -> boolean` | all | Whether the bitwise and is not zero |
| `bit32.bxor(...: number) -> number` | all | Bitwise exclusive or |
| `bit32.byteswap(n: number) -> number` | all | Reverse the byte order |
| `bit32.countlz(n: number) -> number` | all | Count the leading zero bits |
| `bit32.countrz(n: number) -> number` | all | Count the trailing zero bits |
| `bit32.extract(n: number, field: number, width: number?) -> number` | all | The bits field to field + width - 1 |
| `bit32.lrotate(n: number, i: number) -> number` | all | Rotate left |
| `bit32.lshift(n: number, i: number) -> number` | all | Shift left |
| `bit32.replace(n: number, v: number, field: number, width: number?) -> number` | all | n with the bits field to field + width - 1 set from v |
| `bit32.rrotate(n: number, i: number) -> number` | all | Rotate right |
| `bit32.rshift(n: number, i: number) -> number` | all | Logical shift right |

## `buffer`

| Name | Where | Description |
|---|---|---|
| `buffer: table` | all | Fixed-size binary data |
| `buffer.create(size: number) -> buffer` | all | A zero-filled buffer of size bytes |
| `buffer.fromstring(s: string) -> buffer` | all | A buffer holding a string's bytes |
| `buffer.tostring(b: buffer) -> string` | all | A buffer's bytes as a string |
| `buffer.len(b: buffer) -> number` | all | A buffer's size in bytes |
| `buffer.copy(target: buffer, offset: number, source: buffer, sourceOffset: number?, count: number?)` | all | Copy bytes between buffers |
| `buffer.fill(b: buffer, offset: number, value: number, count: number?)` | all | Set bytes to a value |
| `buffer.readbits(b: buffer, bitOffset: number, bitCount: number) -> number` | all | Read up to 32 bits |
| `buffer.writebits(b: buffer, bitOffset: number, bitCount: number, value: number)` | all | Write up to 32 bits |
| `buffer.readi8(b: buffer, offset: number) -> number` | all | Read a signed 8-bit integer |
| `buffer.readu8(b: buffer, offset: number) -> number` | all | Read an unsigned 8-bit integer |
| `buffer.readi16(b: buffer, offset: number) -> number` | all | Read a signed 16-bit integer |
| `buffer.readu16(b: buffer, offset: number) -> number` | all | Read an unsigned 16-bit integer |
| `buffer.readi32(b: buffer, offset: number) -> number` | all | Read a signed 32-bit integer |
| `buffer.readu32(b: buffer, offset: number) -> number` | all | Read an unsigned 32-bit integer |
| `buffer.readf32(b: buffer, offset: number) -> number` | all | Read a 32-bit float |
| `buffer.readf64(b: buffer, offset: number) -> number` | all | Read a 64-bit float |
| `buffer.readstring(b: buffer, offset: number, count: number) -> string` | all | Read count bytes as a string |
| `buffer.writei8(b: buffer, offset: number, value: number)` | all | Write a signed 8-bit integer |
| `buffer.writeu8(b: buffer, offset: number, value: number)` | all | Write an unsigned 8-bit integer |
| `buffer.writei16(b: buffer, offset: number, value: number)` | all | Write a signed 16-bit integer |
| `buffer.writeu16(b: buffer, offset: number, value: number)` | all | Write an unsigned 16-bit integer |
| `buffer.writei32(b: buffer, offset: number, value: number)` | all | Write a signed 32-bit integer |
| `buffer.writeu32(b: buffer, offset: number, value: number)` | all | Write an unsigned 32-bit integer |
| `buffer.writef32(b: buffer, offset: number, value: number)` | all | Write a 32-bit float |
| `buffer.writef64(b: buffer, offset: number, value: number)` | all | Write a 64-bit float |
| `buffer.writestring(b: buffer, offset: number, s: string, count: number?)` | all | Write a string's bytes |

## `coroutine`

| Name | Where | Description |
|---|---|---|
| `coroutine: table` | all | Coroutines |
| `coroutine.create(fn: function) -> thread` | all | A coroutine that runs fn |
| `coroutine.resume(co: thread, ...: any) -> boolean, ...any` | all | Start or continue a coroutine |
| `coroutine.yield(...: any) -> ...any` | all | Pause the running coroutine |
| `coroutine.status(co: thread) -> string` | all | "running", "suspended", "normal" or "dead" |
| `coroutine.running() -> thread?` | all | The running coroutine |
| `coroutine.isyieldable() -> boolean` | all | Whether the running code may yield |
| `coroutine.wrap(fn: function) -> function` | all | A function that resumes a new coroutine each call |
| `coroutine.close(co: thread) -> boolean, any` | all | Close a suspended coroutine |

## `math`

| Name | Where | Description |
|---|---|---|
| `math: table` | all | Mathematical functions |
| `math.abs(x: number) -> number` | all | The absolute value |
| `math.acos(x: number) -> number` | all | The arc cosine, in radians |
| `math.asin(x: number) -> number` | all | The arc sine, in radians |
| `math.atan(x: number) -> number` | all | The arc tangent, in radians |
| `math.atan2(y: number, x: number) -> number` | all | The angle of the point (x, y), in radians |
| `math.ceil(x: number) -> number` | all | Round up to a whole number |
| `math.clamp(x: number, min: number, max: number) -> number` | all | x kept between min and max |
| `math.cos(x: number) -> number` | all | The cosine of an angle in radians |
| `math.cosh(x: number) -> number` | all | The hyperbolic cosine |
| `math.deg(x: number) -> number` | all | Radians to degrees |
| `math.exp(x: number) -> number` | all | e to the power x |
| `math.floor(x: number) -> number` | all | Round down to a whole number |
| `math.fmod(x: number, y: number) -> number` | all | The remainder of x / y, with the sign of x |
| `math.frexp(x: number) -> number, number` | all | The mantissa and exponent of x |
| `math.ldexp(m: number, e: number) -> number` | all | m times 2 to the power e |
| `math.lerp(a: number, b: number, t: number) -> number` | all | The value t of the way from a to b |
| `math.log(x: number, base: number?) -> number` | all | The logarithm (natural unless a base is given) |
| `math.log10(x: number) -> number` | all | The base-10 logarithm |
| `math.map(x: number, inMin: number, inMax: number, outMin: number, outMax: number) -> number` | all | x moved from one range to another |
| `math.max(x: number, ...: number) -> number` | all | The largest argument |
| `math.min(x: number, ...: number) -> number` | all | The smallest argument |
| `math.modf(x: number) -> number, number` | all | The whole and fractional parts |
| `math.noise(x: number, y: number?, z: number?) -> number` | all | Perlin noise between -1 and 1 |
| `math.pow(x: number, y: number) -> number` | all | x to the power y |
| `math.rad(x: number) -> number` | all | Degrees to radians |
| `math.random(m: number?, n: number?) -> number` | all | A random number: 0 to 1, 1 to m, or m to n |
| `math.randomseed(seed: number)` | all | Seed the random numbers |
| `math.round(x: number) -> number` | all | Round to the nearest whole number |
| `math.sign(x: number) -> number` | all | -1, 0 or 1 |
| `math.sin(x: number) -> number` | all | The sine of an angle in radians |
| `math.sinh(x: number) -> number` | all | The hyperbolic sine |
| `math.sqrt(x: number) -> number` | all | The square root |
| `math.tan(x: number) -> number` | all | The tangent of an angle in radians |
| `math.tanh(x: number) -> number` | all | The hyperbolic tangent |
| `math.pi: number` | all | The ratio of a circle's circumference to its diameter |
| `math.huge: number` | all | Infinity |

## `os`

| Name | Where | Description |
|---|---|---|
| `os: table` | all | The clock and dates |
| `os.clock() -> number` | all | Seconds of processor time, for timing code |
| `os.date(format: string?, time: number?) -> string \| table` | all | A date as text, or as a table with "*t" |
| `os.difftime(t2: number, t1: number) -> number` | all | Seconds from t1 to t2 |
| `os.time(date: table?) -> number` | all | The time now (or of a date table) in seconds |

## `string`

| Name | Where | Description |
|---|---|---|
| `string: table` | all | Text functions (also as methods: s:upper()) |
| `string.byte(s: string, i: number?, j: number?) -> ...number` | all | The byte values of characters i to j |
| `string.char(...: number) -> string` | all | A string of the given byte values |
| `string.find(s: string, pattern: string, init: number?, plain: boolean?) -> number?, number?` | all | Where a pattern first matches: start and end |
| `string.format(fmt: string, ...: any) -> string` | all | Text with %d, %s, %.2f... filled in |
| `string.gmatch(s: string, pattern: string) -> function` | all | Iterate over a pattern's matches |
| `string.gsub(s: string, pattern: string, repl: string \| table \| function, n: number?) -> string, number` | all | Replace a pattern's matches |
| `string.len(s: string) -> number` | all | The length in bytes |
| `string.lower(s: string) -> string` | all | In lower case |
| `string.match(s: string, pattern: string, init: number?) -> ...string` | all | A pattern's first match (or its captures) |
| `string.pack(fmt: string, ...: any) -> string` | all | Values packed into binary |
| `string.packsize(fmt: string) -> number` | all | The size string.pack makes for a format |
| `string.rep(s: string, n: number, sep: string?) -> string` | all | s repeated n times |
| `string.reverse(s: string) -> string` | all | Reversed |
| `string.split(s: string, sep: string?) -> {string}` | all | Split at a separator (a comma unless given) |
| `string.sub(s: string, i: number, j: number?) -> string` | all | Characters i to j (negative counts from the end) |
| `string.unpack(fmt: string, s: string, pos: number?) -> ...any` | all | Values unpacked from binary |
| `string.upper(s: string) -> string` | all | In upper case |

## `table`

| Name | Where | Description |
|---|---|---|
| `table: table` | all | List and table functions |
| `table.clear(t: table)` | all | Remove every key, keeping the memory |
| `table.clone(t: table) -> table` | all | A shallow copy |
| `table.concat(t: table, sep: string?, i: number?, j: number?) -> string` | all | The list's items joined into text |
| `table.create(n: number, value: any?) -> table` | all | A list of n items, filled with value |
| `table.find(t: table, value: any, init: number?) -> number?` | all | The index of a value in a list |
| `table.foreach(t: table, fn: function) -> any` | all | Call fn(key, value) for each pair (deprecated) |
| `table.foreachi(t: table, fn: function) -> any` | all | Call fn(index, value) for each item (deprecated) |
| `table.freeze(t: table) -> table` | all | Make a table read-only |
| `table.isfrozen(t: table) -> boolean` | all | Whether a table is read-only |
| `table.getn(t: table) -> number` | all | The list's length (deprecated: use #t) |
| `table.insert(t: table, pos: number \| any, value: any?)` | all | Add an item at the end, or at pos |
| `table.maxn(t: table) -> number` | all | The largest number key |
| `table.move(a: table, f: number, e: number, t: number, b: table?) -> table` | all | Copy a[f..e] to b[t..] |
| `table.pack(...: any) -> table` | all | The arguments as a list with n |
| `table.remove(t: table, pos: number?) -> any` | all | Remove and return the last item, or the one at pos |
| `table.sort(t: table, less: function?)` | all | Sort a list in place |
| `table.unpack(t: table, i: number?, j: number?) -> ...any` | all | The list's items as separate values |

## `utf8`

| Name | Where | Description |
|---|---|---|
| `utf8: table` | all | UTF-8 text |
| `utf8.char(...: number) -> string` | all | A string of the given code points |
| `utf8.charpattern: string` | all | A pattern matching one UTF-8 character |
| `utf8.codepoint(s: string, i: number?, j: number?) -> ...number` | all | The code points of characters i to j |
| `utf8.codes(s: string) -> function` | all | Iterate over byte positions and code points |
| `utf8.len(s: string, i: number?, j: number?) -> number?` | all | The number of characters |
| `utf8.offset(s: string, n: number, i: number?) -> number?` | all | The byte position of the n-th character |

## `vector`

| Name | Where | Description |
|---|---|---|
| `vector: table` | all | 3D vectors |
| `vector.create(x: number, y: number, z: number) -> vector` | all | A vector |
| `vector.zero: vector` | all | The vector (0, 0, 0) |
| `vector.one: vector` | all | The vector (1, 1, 1) |
| `vector.magnitude(v: vector) -> number` | all | The length |
| `vector.normalize(v: vector) -> vector` | all | The same direction, length 1 |
| `vector.cross(a: vector, b: vector) -> vector` | all | The cross product |
| `vector.dot(a: vector, b: vector) -> number` | all | The dot product |
| `vector.angle(a: vector, b: vector, axis: vector?) -> number` | all | The angle between two vectors, in radians |
| `vector.abs(v: vector) -> vector` | all | Each part's absolute value |
| `vector.ceil(v: vector) -> vector` | all | Each part rounded up |
| `vector.floor(v: vector) -> vector` | all | Each part rounded down |
| `vector.sign(v: vector) -> vector` | all | Each part's sign |
| `vector.clamp(v: vector, min: vector, max: vector) -> vector` | all | Each part kept between min and max |
| `vector.max(...: vector) -> vector` | all | The largest of each part |
| `vector.min(...: vector) -> vector` | all | The smallest of each part |

## Tools for plugins

App tools made for a plugin (the Visualizer Helper add-on). Call them with `tool(name, args)`.

| Tool | Arguments | Description |
|---|---|---|
| `audio.analyze_song` | `{clip?, action = "status" \| "start" \| "stems", tempo_min?, tempo_max?, gate?, drops?, after?}` | Analyse a song in the background (stems through the Stem Separation add-on when installed): hits, drops, the calm part, the vocal envelope, onsets and vibrato, stored on the project with markers. `status` returns the state and the summary; `stems` opens Settings ▸ Add-ons |
| `visualizer.template` | `{clip?, template?, palette?, colors?, spikes?, sensitivity?, smoothing?, harder?, ball?, wave?, bolts?, beam?}` | Give the song's visualizer a template (Olympus Storm, Chrome Waveform, Storm Lightning) keyed from the analysis; the same inputs give the same keys |
| `lyrics.karaoke` | `{lyrics, clip?, font?, size?, y?, look?, giant?}` | The exact lyrics as timed Lyric Chrome text clips, each word filling on its sung syllable |
| `visualizer.drop_fx` | `{clip?, flash?, invert?, shatter?, build?, grade?}` | Flash and invert at each drop, the shatter into the biggest drop, the background grade |
| `media.loop` | `{clip?, action = "status" \| "start", seconds?}` | A GIF, picture or clip as a seamless 60 fps ping-pong loop under the song (ffmpeg, in the background) |

## Keywords

`and` `break` `continue` `do` `else` `elseif` `end` `export` `false` `for` `function` `if` `in` `local` `nil` `not` `or` `repeat` `return` `then` `true` `type` `until` `while`

## Types

`any` `boolean` `buffer` `never` `nil` `number` `string` `table` `thread` `unknown` `userdata` `vector`
