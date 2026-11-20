//! ---- feat/script-intellisense ----
//! What the Script Editor knows about the Luau it shows, as plain functions over the text (no egui):
//! a small lexer (`lex`), a scan of the file's own declarations (`Scan`), completion (`complete`, prefix and
//! fuzzy ranked), signature help (`call_at` / `signature`), hover docs (`hover`) and diagnostics (`check`:
//! Luau's own compiler for syntax, plus unknown globals). The API rows all come from
//! `scripting_api::API`. The pane (`ui::script_editor`) draws the results and runs `check` off the UI thread.

use crate::scripting_api::{self as api, ApiItem, Ctx, ItemKind};
use crate::ui::script_editor::{long_close, long_open};
use std::collections::HashSet;
use std::ops::Range;

// ---------------------------------------------------------------------------------------------------
// the lexer

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum T {
    Name,
    Keyword,
    Str,
    Num,
    Comment,
    Op,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tok {
    pub t: T,
    pub r: Range<usize>,
}

const OPS3: &[&str] = &["...", "..=", "//="];
const OPS2: &[&str] = &["..", "==", "~=", "<=", ">=", "->", "::", "//", "+=", "-=", "*=", "/=", "%=", "^="];

/// Every token of a Luau source, comments included, in order (white space is not listed).
pub fn lex(text: &str) -> Vec<Tok> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let s = i;
        let t = if c == b'-' && b.get(i + 1) == Some(&b'-') {
            i = match long_open(b, i + 2) {
                Some(level) => long_close(text, i + 2 + level + 2, level),
                None => text[i..].find('\n').map_or(n, |k| i + k),
            };
            T::Comment
        } else if c == b'"' || c == b'\'' || c == b'`' {
            i += 1;
            while i < n && b[i] != c && b[i] != b'\n' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(n);
            T::Str
        } else if let Some(level) = long_open(b, i) {
            i = long_close(text, i + level + 2, level);
            T::Str
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            i += 1;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                if (b[i] == b'e' || b[i] == b'E')
                    && matches!(b.get(i + 1), Some(b'+' | b'-'))
                    && !text[s..i].starts_with("0x")
                {
                    i += 1;
                }
                i += 1;
            }
            T::Num
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if api::KEYWORDS.contains(&&text[s..i]) {
                T::Keyword
            } else {
                T::Name
            }
        } else {
            let rest = &text[i..];
            let len = OPS3
                .iter()
                .chain(OPS2)
                .find(|o| rest.starts_with(**o))
                .map_or_else(|| rest.chars().next().map_or(1, char::len_utf8), |o| o.len());
            i += len;
            T::Op
        };
        out.push(Tok { t, r: s..i.min(n) });
    }
    out
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Whether byte `c` is inside token `k` (an unclosed string or a line comment also holds its end).
fn inside(text: &str, k: &Tok, c: usize) -> bool {
    if !(k.r.start < c && c <= k.r.end) {
        return false;
    }
    if c < k.r.end {
        return true;
    }
    let s = &text[k.r.clone()];
    match k.t {
        T::Str => {
            let q = s.as_bytes()[0];
            if q == b'[' {
                return !s.ends_with(']');
            }
            s.len() < 2 || s.as_bytes()[s.len() - 1] != q || s.ends_with(&format!("\\{}", q as char))
        }
        T::Comment => !(s.starts_with("--[") && long_open(s.as_bytes(), 2).is_some() && s.ends_with(']')),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------------
// the file's own declarations

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SymKind {
    Keyword,
    Type,
    Table,
    Function,
    Value,
    Local,
    LocalFunction,
    Param,
    Global,
    Tool,
}

impl SymKind {
    /// The letter in the completion row's icon.
    pub fn letter(self) -> &'static str {
        match self {
            SymKind::Keyword => "k",
            SymKind::Type => "T",
            SymKind::Table => "{}",
            SymKind::Function | SymKind::LocalFunction => "ƒ",
            SymKind::Value => "v",
            SymKind::Local | SymKind::Global => "x",
            SymKind::Param => "p",
            SymKind::Tool => "t",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SymKind::Keyword => "keyword",
            SymKind::Type => "type",
            SymKind::Table => "table",
            SymKind::Function => "function",
            SymKind::Value => "value",
            SymKind::Local => "local",
            SymKind::LocalFunction => "local function",
            SymKind::Param => "parameter",
            SymKind::Global => "global",
            SymKind::Tool => "tool",
        }
    }

    fn of(i: &ApiItem) -> SymKind {
        match i.kind {
            ItemKind::Table => SymKind::Table,
            ItemKind::Function => SymKind::Function,
            ItemKind::Value => SymKind::Value,
        }
    }
}

/// A name the file declares: `local x`, `local function f(a)`, a parameter, `function g()`, `y = 1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymKind,
    /// Where the declaration is (byte).
    pub at: usize,
    /// `f(a, b)` for a function, else empty.
    pub params: String,
}

#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub symbols: Vec<Symbol>,
    /// `(table, member)` of `function M.f()` / `M.x = 1`.
    pub members: Vec<(String, Symbol)>,
    /// Every name declared anywhere in the file (for the unknown-global check).
    pub declared: HashSet<String>,
}

/// The text and its tokens (comments dropped) and declarations, made once per edit.
pub struct Analysis<'a> {
    pub text: &'a str,
    /// Comments included (completion needs to see them).
    pub all: Vec<Tok>,
    /// Without comments.
    pub toks: Vec<Tok>,
    pub scan: Scan,
}

impl<'a> Analysis<'a> {
    pub fn new(text: &'a str) -> Analysis<'a> {
        let all = lex(text);
        let toks: Vec<Tok> = all.iter().filter(|k| k.t != T::Comment).cloned().collect();
        let scan = scan(text, &toks);
        Analysis { text, all, toks, scan }
    }

    fn s(&self, k: &Tok) -> &'a str {
        &self.text[k.r.clone()]
    }
}

fn op(text: &str, k: Option<&Tok>, o: &str) -> bool {
    k.is_some_and(|k| k.t == T::Op && &text[k.r.clone()] == o)
}

fn kw(text: &str, k: Option<&Tok>, w: &str) -> bool {
    k.is_some_and(|k| k.t == T::Keyword && &text[k.r.clone()] == w)
}

/// Step over a type annotation starting at token `i` (after its `:`); the index after it.
fn skip_type(text: &str, toks: &[Tok], mut i: usize) -> usize {
    let mut operand = true;
    while let Some(k) = toks.get(i) {
        let s = &text[k.r.clone()];
        if operand {
            match (k.t, s) {
                (T::Name, _) => {
                    i += 1;
                    while op(text, toks.get(i), ".") && toks.get(i + 1).is_some_and(|k| k.t == T::Name) {
                        i += 2;
                    }
                    if op(text, toks.get(i), "(") && s == "typeof" {
                        i = skip_group(text, toks, i);
                    }
                }
                (T::Keyword, "nil" | "true" | "false") | (T::Str, _) => i += 1,
                (T::Op, "{" | "(") => i = skip_group(text, toks, i),
                _ => break,
            }
            operand = false;
        } else {
            match (k.t, s) {
                (T::Op, "?") => i += 1,
                (T::Op, "|" | "&" | "->") => {
                    i += 1;
                    operand = true;
                }
                (T::Op, "<") => {
                    let mut d = 0;
                    while let Some(k) = toks.get(i) {
                        let s = &text[k.r.clone()];
                        i += 1;
                        d += (s == "<") as i32 - (s == ">") as i32;
                        if d <= 0 {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }
    i
}

/// Index after the bracket group opening at `i`.
fn skip_group(text: &str, toks: &[Tok], mut i: usize) -> usize {
    let mut d = 0;
    while let Some(k) = toks.get(i) {
        i += 1;
        if k.t == T::Op {
            match &text[k.r.clone()] {
                "(" | "{" | "[" => d += 1,
                ")" | "}" | "]" => d -= 1,
                _ => {}
            }
        }
        if d <= 0 {
            break;
        }
    }
    i
}

/// The parameter list at `(` token `i`: names with their positions, the text inside, and the index after `)`.
fn params_at(text: &str, toks: &[Tok], i: usize) -> (Vec<(String, usize)>, String, usize) {
    let end = skip_group(text, toks, i);
    let mut names = Vec::new();
    let mut j = i + 1;
    while j + 1 < end {
        let k = &toks[j];
        if k.t == T::Name || op(text, Some(k), "...") {
            names.push((text[k.r.clone()].to_string(), k.r.start));
            j += 1;
            if op(text, toks.get(j), ":") {
                j = skip_type(text, toks, j + 1);
            }
            continue;
        }
        j += 1;
    }
    let inner = names.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ");
    (names, inner, end)
}

/// Every declaration in `toks` (a comment-free token list of `text`).
pub fn scan(text: &str, toks: &[Tok]) -> Scan {
    let mut sc = Scan::default();
    let add = |sc: &mut Scan, name: &str, kind: SymKind, at: usize, params: String| {
        sc.declared.insert(name.to_string());
        sc.symbols.push(Symbol { name: name.into(), kind, at, params });
    };
    let mut braces = 0i32;
    let mut i = 0;
    while i < toks.len() {
        let k = &toks[i];
        let s = &text[k.r.clone()];
        match (k.t, s) {
            (T::Op, "{") => braces += 1,
            (T::Op, "}") => braces -= 1,
            (T::Keyword, "local") => {
                if kw(text, toks.get(i + 1), "function") {
                    if let Some(n) = toks.get(i + 2).filter(|n| n.t == T::Name) {
                        let name = &text[n.r.clone()];
                        if op(text, toks.get(i + 3), "(") {
                            let (ps, inner, end) = params_at(text, toks, i + 3);
                            add(&mut sc, name, SymKind::LocalFunction, n.r.start, format!("{name}({inner})"));
                            for (p, at) in ps {
                                add(&mut sc, &p, SymKind::Param, at, String::new());
                            }
                            i = end;
                            continue;
                        }
                        add(&mut sc, name, SymKind::LocalFunction, n.r.start, String::new());
                    }
                    i += 3;
                    continue;
                }
                // local a: T, b <const> = ...
                let mut j = i + 1;
                let mut names = Vec::new();
                while let Some(n) = toks.get(j).filter(|n| n.t == T::Name) {
                    names.push((text[n.r.clone()].to_string(), n.r.start));
                    j += 1;
                    if op(text, toks.get(j), ":") {
                        j = skip_type(text, toks, j + 1);
                    }
                    if op(text, toks.get(j), "<") {
                        j = skip_type(text, toks, j).max(j + 1);
                        while toks.get(j).is_some_and(|k| !op(text, Some(k), ">")) && j < i + 8 {
                            j += 1;
                        }
                        j += 1;
                    }
                    if !op(text, toks.get(j), ",") {
                        break;
                    }
                    j += 1;
                }
                // `local f = function(a)` reads as a function
                let func = op(text, toks.get(j), "=") && kw(text, toks.get(j + 1), "function") && names.len() == 1;
                for (n, at) in names {
                    if func && op(text, toks.get(j + 2), "(") {
                        let (_, inner, _) = params_at(text, toks, j + 2);
                        let p = format!("{n}({inner})");
                        add(&mut sc, &n, SymKind::LocalFunction, at, p);
                    } else {
                        add(&mut sc, &n, SymKind::Local, at, String::new());
                    }
                }
                i = j;
                continue;
            }
            (T::Keyword, "function") => {
                // function a.b:c(...) / function g(...) / function(...)
                let mut j = i + 1;
                let mut chain: Vec<&Tok> = Vec::new();
                let mut method = false;
                while let Some(n) = toks.get(j).filter(|n| n.t == T::Name) {
                    chain.push(n);
                    j += 1;
                    if op(text, toks.get(j), ".") || op(text, toks.get(j), ":") {
                        method = op(text, toks.get(j), ":");
                        j += 1;
                        continue;
                    }
                    break;
                }
                if op(text, toks.get(j), "<") {
                    while toks.get(j).is_some_and(|k| !op(text, Some(k), ">")) {
                        j += 1;
                    }
                    j += 1;
                }
                if op(text, toks.get(j), "(") {
                    let (ps, inner, end) = params_at(text, toks, j);
                    match chain.as_slice() {
                        [] => {}
                        [n] => {
                            let name = &text[n.r.clone()];
                            add(&mut sc, name, SymKind::Function, n.r.start, format!("{name}({inner})"));
                        }
                        [.., last] => {
                            let parent: Vec<&str> =
                                chain[..chain.len() - 1].iter().map(|k| &text[k.r.clone()]).collect();
                            let parent = parent.join(".");
                            let name = &text[last.r.clone()];
                            let sym = Symbol {
                                name: name.into(),
                                kind: SymKind::Function,
                                at: last.r.start,
                                params: format!("{parent}{}{name}({inner})", if method { ":" } else { "." }),
                            };
                            sc.members.push((parent, sym));
                        }
                    }
                    if method {
                        add(&mut sc, "self", SymKind::Param, toks[j].r.start, String::new());
                    }
                    for (p, at) in ps {
                        add(&mut sc, &p, SymKind::Param, at, String::new());
                    }
                    i = end;
                    continue;
                }
                i = j;
                continue;
            }
            (T::Keyword, "for") => {
                let mut j = i + 1;
                while let Some(n) = toks.get(j).filter(|n| n.t == T::Name) {
                    add(&mut sc, &text[n.r.clone()], SymKind::Local, n.r.start, String::new());
                    j += 1;
                    if op(text, toks.get(j), ":") {
                        j = skip_type(text, toks, j + 1);
                    }
                    if !op(text, toks.get(j), ",") {
                        break;
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
            (T::Keyword, "type") if toks.get(i + 1).is_some_and(|n| n.t == T::Name) => {
                let n = &toks[i + 1];
                if matches!(toks.get(i + 2).map(|k| &text[k.r.clone()]), Some("=" | "<")) {
                    add(&mut sc, &text[n.r.clone()], SymKind::Type, n.r.start, String::new());
                }
            }
            (T::Name, name) => {
                let prev = i.checked_sub(1).and_then(|p| toks.get(p));
                let after_dot = op(text, prev, ".") || op(text, prev, ":");
                if !after_dot {
                    // M.x = ... (one level)
                    if op(text, toks.get(i + 1), ".")
                        && toks.get(i + 2).is_some_and(|m| m.t == T::Name)
                        && op(text, toks.get(i + 3), "=")
                    {
                        let m = &toks[i + 2];
                        let sym = Symbol {
                            name: text[m.r.clone()].into(),
                            kind: SymKind::Value,
                            at: m.r.start,
                            params: String::new(),
                        };
                        if !sc.members.iter().any(|(p, s)| p == name && s.name == sym.name) {
                            sc.members.push((name.to_string(), sym));
                        }
                    } else if braces == 0 && (op(text, toks.get(i + 1), "=") || compound_assign(text, toks, i)) {
                        // y = 1 or a, b = ... at statement level declares a global
                        if !sc.declared.contains(name) {
                            add(&mut sc, name, SymKind::Global, k.r.start, String::new());
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    sc
}

/// `a, b = ...`: token `i` is one of the names before the `=`.
fn compound_assign(text: &str, toks: &[Tok], i: usize) -> bool {
    let mut j = i + 1;
    while op(text, toks.get(j), ",") && toks.get(j + 1).is_some_and(|k| k.t == T::Name) {
        j += 2;
    }
    j > i + 1 && op(text, toks.get(j), "=")
}

// ---------------------------------------------------------------------------------------------------
// completion

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: SymKind,
    /// The signature (`editor.tool(name: string, args: table?) -> any`), or the kind's name.
    pub detail: String,
    pub doc: String,
    score: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    /// Byte where the word being completed starts (accepting replaces start..cursor).
    pub start: usize,
    pub items: Vec<Item>,
}

/// How well `label` matches what was typed: a prefix beats a case-blind prefix beats a match at a word
/// start (`_x`) beats letters in order (fuzzy). `None` = no match.
pub fn score(label: &str, typed: &str) -> Option<i32> {
    if typed.is_empty() {
        return Some(0);
    }
    let len = label.len() as i32;
    if label.starts_with(typed) {
        return Some(3000 - len);
    }
    let (l, t) = (label.to_ascii_lowercase(), typed.to_ascii_lowercase());
    if l.starts_with(&t) {
        return Some(2000 - len);
    }
    if let Some(k) = l.find(&t).filter(|&k| k > 0 && matches!(l.as_bytes()[k - 1], b'_' | b'.')) {
        return Some(1500 - k as i32 - len);
    }
    // letters in order; the first must match the first (fuzzy, not anywhere)
    let lb = l.as_bytes();
    if lb.first() != t.as_bytes().first() {
        return None;
    }
    let (mut at, mut gaps) = (0usize, 0i32);
    for c in t.bytes() {
        let k = lb[at..].iter().position(|&x| x == c)?;
        gaps += k as i32;
        at += k + 1;
    }
    Some(1000 - gaps * 10 - len)
}

fn api_item(i: &ApiItem) -> Item {
    Item { label: i.short().to_string(), kind: SymKind::of(i), detail: i.signature(), doc: i.doc.to_string(), score: 0 }
}

fn sym_item(s: &Symbol) -> Item {
    let detail = if s.params.is_empty() { s.kind.name().to_string() } else { s.params.clone() };
    Item {
        label: s.name.clone(),
        kind: s.kind,
        detail,
        doc: format!("{}, declared in this file", s.kind.name()),
        score: 0,
    }
}

/// Keep the items matching `typed`, best first, one row per label.
fn rank(items: Vec<Item>, typed: &str, force: bool) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for mut it in items {
        let Some(s) = score(&it.label, typed) else { continue };
        // a word already typed out in full needs no list (Enter must stay a new line)
        if !force && it.label == typed {
            return Vec::new();
        }
        // the file's own names first among equals, keywords last
        let bias = match it.kind {
            SymKind::Local | SymKind::LocalFunction | SymKind::Param => 3,
            SymKind::Keyword => -3,
            _ => 0,
        };
        it.score = s + bias;
        if let Some(o) = out.iter_mut().find(|o| o.label == it.label) {
            if it.score > o.score {
                *o = it;
            }
            continue;
        }
        out.push(it);
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.label.cmp(&b.label)));
    out
}

/// The dotted name ending at token `i` (`editor`, `a.b`), and the index of its first token.
fn chain_before(an: &Analysis, i: usize) -> Option<(String, usize)> {
    let toks = &an.toks;
    let k = toks.get(i).filter(|k| k.t == T::Name)?;
    let mut parts = vec![an.s(k)];
    let mut j = i;
    while j >= 2 && op(an.text, toks.get(j - 1), ".") && toks[j - 2].t == T::Name {
        j -= 2;
        parts.push(an.s(&toks[j]));
    }
    parts.reverse();
    Some((parts.join("."), j))
}

/// What a tool string (`editor.tool("…`) completes to.
fn tool_items() -> Vec<Item> {
    crate::mcp::tools::all()
        .map(|d| Item {
            label: d.name.to_string(),
            kind: SymKind::Tool,
            detail: d.args.join("; "),
            doc: d.desc.to_string(),
            score: 0,
        })
        .collect()
}

/// The list at byte `cursor` of `an.text`: members after `x.` / `x:`, the names in scope while typing a
/// word, tool names inside `editor.tool("…`. `force` is Ctrl+Space (a list even before anything is typed).
pub fn complete(an: &Analysis, cursor: usize, force: bool, ctx: Ctx) -> Option<Completion> {
    let text = an.text;
    let cursor = cursor.min(text.len());
    if !text.is_char_boundary(cursor) {
        return None;
    }
    // in a string or a comment: only a tool name in editor.tool("…
    if let Some(k) = an.all.iter().find(|k| inside(text, k, cursor)) {
        if k.t != T::Str || !matches!(text.as_bytes()[k.r.start], b'"' | b'\'') {
            return None;
        }
        let call = call_at(an, k.r.start)?;
        if call.callee != "editor.tool" || call.arg != 0 {
            return None;
        }
        let start = k.r.start + 1;
        let typed = &text[start..cursor];
        let items = rank(tool_items(), typed, true);
        return (!items.is_empty()).then_some(Completion { start, items });
    }
    let b = text.as_bytes();
    let mut start = cursor;
    while start > 0 && is_ident(b[start - 1]) {
        start -= 1;
    }
    let typed = &text[start..cursor];
    if typed.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let before = text[..start].trim_end_matches([' ', '\t']);
    let sep = before.as_bytes().last().copied();
    // `..` is concatenation, not a member
    let member = matches!(sep, Some(b'.') if !before.ends_with("..")) || sep == Some(b':') && !before.ends_with("::");
    let items = if member {
        // the token just before the separator
        let sep_at = before.len() - 1;
        let i = an.toks.iter().rposition(|k| k.r.end <= sep_at)?;
        let (recv, first) = chain_before(an, i)?;
        let mut items: Vec<Item> = api::members(&recv, ctx).map(api_item).collect();
        items.extend(an.scan.members.iter().filter(|(p, _)| *p == recv).map(|(_, s)| sym_item(s)));
        if sep == Some(b':') && items.is_empty() {
            let declared = first.checked_sub(1).is_some_and(|p| kw(text, an.toks.get(p), "local"))
                || an.scan.symbols.iter().any(|s| s.kind == SymKind::Param && s.at == an.toks[first].r.start);
            if declared || (first > 0 && op(text, an.toks.get(first - 1), ")")) {
                // `local x: ` / `(a: ` / `): ` is a type annotation
                items = types(an);
            } else {
                // `s:upper()` on a string
                items = api::members("string", ctx).map(api_item).collect();
            }
        }
        items
    } else {
        if typed.is_empty() && !force {
            return None;
        }
        let mut items: Vec<Item> = api::KEYWORDS
            .iter()
            .map(|k| Item {
                label: k.to_string(),
                kind: SymKind::Keyword,
                detail: "keyword".into(),
                doc: String::new(),
                score: 0,
            })
            .collect();
        items.extend(api::globals(ctx).map(api_item));
        // the file's names declared before the cursor, and its global functions anywhere
        items.extend(
            an.scan
                .symbols
                .iter()
                .filter(|s| s.at < start || matches!(s.kind, SymKind::Function | SymKind::Global))
                .filter(|s| s.kind != SymKind::Type)
                .map(sym_item),
        );
        items
    };
    let items = rank(items, typed, force || member);
    (!items.is_empty()).then_some(Completion { start, items })
}

fn types(an: &Analysis) -> Vec<Item> {
    let mut v: Vec<Item> = api::TYPES
        .iter()
        .map(|t| Item {
            label: t.to_string(),
            kind: SymKind::Type,
            detail: "type".into(),
            doc: String::new(),
            score: 0,
        })
        .collect();
    v.extend(an.scan.symbols.iter().filter(|s| s.kind == SymKind::Type).map(sym_item));
    v
}

// ---------------------------------------------------------------------------------------------------
// signature help

/// The call whose parentheses hold byte `cursor`: the callee (`editor.tool`, `s:sub`), which argument the
/// cursor is in (0-based), and where its `(` is.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub callee: String,
    pub arg: usize,
    pub open: usize,
    pub method: bool,
}

/// Find the open call around `cursor`: commas count only at the call's own depth (not inside a nested
/// call, a table constructor or a string), and a statement keyword in between means there is none.
pub fn call_at(an: &Analysis, cursor: usize) -> Option<Call> {
    let text = an.text;
    let toks = &an.toks;
    let end = toks.iter().position(|k| k.r.start >= cursor).unwrap_or(toks.len());
    let mut depth = 0usize;
    let mut arg = 0usize;
    for i in (0..end).rev() {
        let k = &toks[i];
        let s = an.s(k);
        match k.t {
            T::Op => match s {
                ")" | "]" | "}" => depth += 1,
                "(" | "[" | "{" if depth > 0 => depth -= 1,
                "(" => {
                    // a definition's parameter list is not a call
                    let prev = i.checked_sub(1).and_then(|p| toks.get(p));
                    if kw(text, prev, "function") {
                        return None;
                    }
                    if prev.is_some_and(|p| p.t == T::Name) {
                        let (mut callee, first) = chain_before(an, i - 1)?;
                        if first >= 2 && kw(text, toks.get(first - 2), "function") {
                            return None;
                        }
                        let mut method = false;
                        // a:b( - the receiver before the colon
                        if first >= 2 && op(text, toks.get(first - 1), ":") {
                            method = true;
                            let recv = chain_before(an, first - 2).map_or(String::new(), |(r, _)| r);
                            callee = format!("{recv}:{callee}");
                        }
                        if first >= 1 && kw(text, toks.get(first - 1), "function") {
                            return None;
                        }
                        return Some(Call { callee, arg, open: k.r.start, method });
                    }
                    arg = 0;
                }
                "[" | "{" => arg = 0,
                "," if depth == 0 => arg += 1,
                _ => {}
            },
            T::Keyword if depth == 0 => {
                if matches!(
                    s,
                    "do" | "end"
                        | "local"
                        | "return"
                        | "function"
                        | "repeat"
                        | "until"
                        | "while"
                        | "for"
                        | "break"
                        | "continue"
                ) {
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

/// What signature help shows for a call: the label, the parameter list and the doc.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub name: String,
    pub params: Vec<String>,
    pub ret: String,
    pub doc: String,
}

/// The signature of `call` (from the API, or a function this file declares).
pub fn signature(an: &Analysis, call: &Call, ctx: Ctx) -> Option<Signature> {
    let from_api = |i: &ApiItem, drop_self: bool| Signature {
        name: i.name.to_string(),
        params: i.param_list().iter().skip(drop_self as usize).map(|(n, t)| format!("{n}: {t}")).collect(),
        ret: i.ret.to_string(),
        doc: i.doc.to_string(),
    };
    if call.method {
        let (recv, m) = call.callee.rsplit_once(':')?;
        if let Some((_, s)) = an.scan.members.iter().find(|(p, s)| p == recv && s.name == m) {
            return sym_sig(s, false);
        }
        return api::find(&format!("string.{m}"), ctx)
            .filter(|i| i.kind == ItemKind::Function)
            .map(|i| from_api(i, true));
    }
    if let Some(i) = api::find(&call.callee, ctx).filter(|i| i.kind == ItemKind::Function) {
        return Some(from_api(i, false));
    }
    if let Some((recv, m)) = call.callee.rsplit_once('.') {
        let (_, s) = an.scan.members.iter().find(|(p, s)| p == recv && s.name == m)?;
        return sym_sig(s, false);
    }
    let s = an.scan.symbols.iter().filter(|s| s.name == call.callee && !s.params.is_empty()).last()?;
    sym_sig(s, false)
}

fn sym_sig(s: &Symbol, _: bool) -> Option<Signature> {
    let (name, rest) = s.params.split_once('(')?;
    let inner = rest.strip_suffix(')').unwrap_or(rest);
    let params = if inner.is_empty() { Vec::new() } else { inner.split(", ").map(str::to_string).collect() };
    Some(Signature {
        name: name.to_string(),
        params,
        ret: String::new(),
        doc: format!("{}, declared in this file", s.kind.name()),
    })
}

// ---------------------------------------------------------------------------------------------------
// hover

/// The hover text for the name at byte `at`: its signature and doc. `None` off a known name.
pub fn hover(an: &Analysis, at: usize, ctx: Ctx) -> Option<(String, String)> {
    let i = an.all.iter().position(|k| k.r.start <= at && at < k.r.end)?;
    let k = &an.all[i];
    if k.t == T::Str {
        // a tool name inside editor.tool("…")
        let call = call_at(an, k.r.start)?;
        let name = an.text[k.r.clone()].trim_matches(['"', '\'']);
        if call.callee == "editor.tool" && call.arg == 0 {
            let d = crate::mcp::tools::find(name)?;
            return Some((d.name.to_string(), d.desc.to_string()));
        }
        return None;
    }
    if k.t == T::Keyword {
        return None;
    }
    if k.t != T::Name {
        return None;
    }
    let j = an.toks.iter().position(|t| t.r == k.r)?;
    let (chain, first) = chain_before(an, j)?;
    if first >= 2 && op(an.text, an.toks.get(first - 1), ":") {
        let m = an.s(k);
        let i = api::find(&format!("string.{m}"), ctx)?;
        return Some((i.signature(), i.doc.to_string()));
    }
    if let Some(i) = api::find(&chain, ctx) {
        return Some((i.signature(), i.doc.to_string()));
    }
    None
}

// ---------------------------------------------------------------------------------------------------
// diagnostics

#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    /// The bytes underlined.
    pub range: Range<usize>,
    /// 1-based line.
    pub line: usize,
    pub msg: String,
    /// An error (red); otherwise a warning.
    pub error: bool,
}

/// Byte range of 1-based `line`, without its indent and trailing space (the whole line if blank).
fn line_range(text: &str, line: usize) -> Range<usize> {
    let mut start = 0;
    for _ in 1..line {
        match text[start..].find('\n') {
            Some(k) => start += k + 1,
            None => break,
        }
    }
    let end = text[start..].find('\n').map_or(text.len(), |k| start + k);
    let l = &text[start..end];
    let lead = l.len() - l.trim_start().len();
    let r = start + lead..start + l.trim_end().len();
    if r.is_empty() {
        start..end.max(start)
    } else {
        r
    }
}

fn line_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

/// Luau's compile error for `text` (compiled only, never run), at the line Luau reports.
pub fn syntax(text: &str) -> Option<Diag> {
    const NAME: &str = "script";
    let e = crate::scripting::check(text, NAME).err()?;
    let first = e.lines().next().unwrap_or(&e);
    // "...script:12: Expected ..." -> 12, "Expected ..."
    let at = first.find(&format!("{NAME}:"))?;
    let rest = &first[at + NAME.len() + 1..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let line: usize = digits.parse().ok()?;
    let msg = rest[digits.len()..].trim_start_matches(':').trim().to_string();
    let lines = text.lines().count().max(1);
    let line = line.clamp(1, lines);
    Some(Diag { range: line_range(text, line), line, msg, error: true })
}

/// Names read as globals that neither the API (for `ctx`) nor the file declares.
pub fn unknown_globals(an: &Analysis, ctx: Ctx) -> Vec<Diag> {
    let text = an.text;
    let toks = &an.toks;
    let known: HashSet<&str> = api::globals(ctx).map(|i| i.name).collect();
    let mut out = Vec::new();
    let mut braces = 0i32;
    let mut i = 0;
    while i < toks.len() {
        let k = &toks[i];
        let s = an.s(k);
        match (k.t, s) {
            (T::Op, "{") => braces += 1,
            (T::Op, "}") => braces -= 1,
            // `x: T`, `): T`, `:: T` - type annotations; `a:b(...)` - a method (skipped as a member below)
            (T::Op, ":" | "::" | "->") => {
                let method = s == ":"
                    && toks.get(i + 1).is_some_and(|n| n.t == T::Name)
                    && matches!(toks.get(i + 2).map(|k| (k.t, an.s(k))), Some((T::Op, "(" | "{")) | Some((T::Str, _)));
                if !method {
                    i = skip_type(text, toks, i + 1);
                    continue;
                }
            }
            // type X = ... / export type X = ...: the rest of the declaration is types
            (T::Keyword, "type") if toks.get(i + 1).is_some_and(|n| n.t == T::Name) => {
                let line = line_of(text, k.r.start);
                let mut d = 0i32;
                i += 1;
                while let Some(t) = toks.get(i) {
                    if d <= 0 && line_of(text, t.r.start) > line && !matches!(an.s(t), "}" | "|" | "&") {
                        break;
                    }
                    if t.t == T::Op {
                        d += matches!(an.s(t), "{" | "(") as i32 - matches!(an.s(t), "}" | ")") as i32;
                    }
                    i += 1;
                }
                continue;
            }
            (T::Name, name) => {
                let prev = i.checked_sub(1).and_then(|p| toks.get(p));
                let member = op(text, prev, ".") || op(text, prev, ":");
                let key = braces > 0 && op(text, toks.get(i + 1), "=");
                if !member && !key && !known.contains(name) && !an.scan.declared.contains(name) {
                    out.push(Diag {
                        range: k.r.clone(),
                        line: line_of(text, k.r.start),
                        msg: format!("Unknown global '{name}'"),
                        error: false,
                    });
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The whole check the pane runs after typing stops: the syntax error (if any) and the unknown globals.
pub fn check(text: &str, ctx: Ctx) -> Vec<Diag> {
    let an = Analysis::new(text);
    let mut v: Vec<Diag> = syntax(text).into_iter().collect();
    v.extend(unknown_globals(&an, ctx));
    v
}

/// The context a file's text runs in: a plugin's sandbox, or a user script (and a hook when it has an
/// `-- @on` header line).
pub fn context_of(text: &str, plugin: bool) -> Ctx {
    if plugin {
        return api::PLUGIN;
    }
    let hook = text
        .lines()
        .take_while(|l| l.trim_start().starts_with("--"))
        .any(|l| l.trim_start().trim_start_matches('-').trim_start().starts_with("@on"));
    if hook {
        api::SCRIPT | api::HOOK
    } else {
        api::SCRIPT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting_api::{AGENT, PLUGIN, SCRIPT};

    fn at(src: &str) -> (String, usize) {
        let c = src.find('|').expect("a | marks the cursor");
        (src.replacen('|', "", 1), c)
    }

    fn labels(src: &str, force: bool, ctx: Ctx) -> Vec<String> {
        let (text, c) = at(src);
        let an = Analysis::new(&text);
        complete(&an, c, force, ctx).map(|c| c.items.into_iter().map(|i| i.label).collect()).unwrap_or_default()
    }

    #[test]
    fn prefix_filter_and_ranking() {
        // a prefix match beats a case-blind one beats a word-start one beats a fuzzy one
        let p = score("tostring", "to").unwrap();
        let ci = score("ToString", "to").unwrap();
        let word = score("buffer_tostring", "to").unwrap();
        let fuzzy = score("table_object", "tbo").unwrap();
        assert!(p > ci && ci > word && word > fuzzy, "{p} {ci} {word} {fuzzy}");
        assert_eq!(score("pairs", "zz"), None);
        assert_eq!(score("select", "lct"), None, "a fuzzy match starts at the first letter");
        assert!(score("setmetatable", "smt").is_some(), "letters in order match");
        // shorter wins among prefixes
        assert!(score("type", "ty").unwrap() > score("typeof", "ty").unwrap());

        let l = labels("local x = pa|", false, SCRIPT);
        assert_eq!(l.first().map(String::as_str), Some("pairs"), "{l:?}");
        assert!(
            !l.contains(&"print".to_string())
                || l.iter().position(|x| x == "pairs") < l.iter().position(|x| x == "print")
        );
        // fuzzy: `gmt` finds getmetatable
        assert!(labels("gmt|", false, SCRIPT).contains(&"getmetatable".to_string()));
        // keywords are offered, after a same-scored global
        assert!(labels("loc|", false, SCRIPT).contains(&"local".to_string()));
        // nothing typed: nothing unless asked (Ctrl+Space)
        assert!(labels("x = |", false, SCRIPT).is_empty());
        assert!(labels("x = |", true, SCRIPT).contains(&"editor".to_string()));
        // a word typed out in full closes the list, so Enter stays a new line
        assert!(labels("if x then\nend|", false, SCRIPT).is_empty());
        // not in strings, comments or numbers
        assert!(labels("local s = \"pa|\"", false, SCRIPT).is_empty());
        assert!(labels("-- pa|", false, SCRIPT).is_empty());
        assert!(labels("x = 12|", false, SCRIPT).is_empty());
        // the context decides: library exists for the Assistant and plugins, not for a user script
        assert!(!labels("lib|", false, SCRIPT).contains(&"library".to_string()));
        assert!(labels("lib|", false, AGENT).contains(&"library".to_string()));
        assert!(labels("pro|", false, PLUGIN).contains(&"programs".to_string()));
    }

    #[test]
    fn member_completion_after_a_dot() {
        // the bridge table every script has (the API has no `video` global: `editor` is the video bridge)
        let l = labels("editor.|", false, SCRIPT);
        assert_eq!(l, vec!["log", "tool", "tools"], "{l:?}");
        let l = labels("editor.t|", false, SCRIPT | crate::scripting_api::HOOK);
        assert_eq!(l, vec!["tool", "tools"]);
        assert!(labels("editor.|", false, SCRIPT | crate::scripting_api::HOOK).contains(&"event".to_string()));
        let l = labels("local c = project.cl|", false, AGENT);
        assert_eq!(l, vec!["clips"]);
        assert!(labels("math.fl|", false, SCRIPT).contains(&"floor".to_string()));
        // a fuzzy member
        assert!(labels("string.gsb|", false, SCRIPT).contains(&"gsub".to_string()));
        // `:` on a string value: string methods; `..` is not a member access
        assert!(labels("local s = name:up|", false, SCRIPT).contains(&"upper".to_string()));
        assert!(!labels("x = a ..pa|", false, SCRIPT).is_empty());
        assert!(labels("x = a ..pa|", false, SCRIPT).contains(&"pairs".to_string()));
        // members the file adds to its own table
        let l = labels("local M = {}\nfunction M.render(a) end\nM.size = 3\nM.|", false, SCRIPT);
        assert_eq!(l, vec!["render", "size"]);
        // a type annotation after `local x:` offers types, not string methods
        assert!(labels("local n: nu|", false, SCRIPT).contains(&"number".to_string()));
        // a tool name inside editor.tool("…
        let l = labels("editor.tool(\"timeline.li|", false, SCRIPT);
        assert!(l.contains(&"timeline.list".to_string()), "{l:?}");
        assert!(labels("print(\"timeline.li|", false, AGENT).is_empty(), "only the tool's first argument");
    }

    #[test]
    fn local_symbol_scan() {
        let src = "local a, b: number = 1, 2\nlocal function helper(x, y) return x end\n\
                   function Global(p) end\nfor i, v in ipairs({}) do end\ncount = 0\n\
                   local f = function(q) end\nlocal t = { key = 1 }\ntype Point = { x: number }\n\
                   local M = {}\nfunction M:draw(w) end\n";
        let an = Analysis::new(src);
        let find = |n: &str| an.scan.symbols.iter().find(|s| s.name == n).map(|s| (s.kind, s.params.clone()));
        assert_eq!(find("a").unwrap().0, SymKind::Local);
        assert_eq!(find("b").unwrap().0, SymKind::Local);
        assert_eq!(find("helper"), Some((SymKind::LocalFunction, "helper(x, y)".into())));
        assert_eq!(find("x").unwrap().0, SymKind::Param);
        assert_eq!(find("Global"), Some((SymKind::Function, "Global(p)".into())));
        assert_eq!(find("i").unwrap().0, SymKind::Local);
        assert_eq!(find("v").unwrap().0, SymKind::Local);
        assert_eq!(find("count").unwrap().0, SymKind::Global);
        assert_eq!(find("f"), Some((SymKind::LocalFunction, "f(q)".into())));
        assert_eq!(find("Point").unwrap().0, SymKind::Type);
        assert_eq!(find("self").unwrap().0, SymKind::Param, "a : method has self");
        assert!(find("key").is_none(), "a table key is not a declaration");
        assert!(find("number").is_none(), "a type is not a declaration");
        assert!(an.scan.members.iter().any(|(p, s)| p == "M" && s.name == "draw" && s.params == "M:draw(w)"));
        // completion offers what is declared before the cursor, not after
        let l = labels("local early = 1\nea|\nlocal eager = 2", false, SCRIPT);
        assert!(l.contains(&"early".to_string()) && !l.contains(&"eager".to_string()), "{l:?}");
        // and a local outranks an API name with the same score
        let l = labels("local prints = 1\nprin|", false, AGENT);
        assert_eq!(l[0], "prints", "{l:?}");
        assert!(l.contains(&"print".to_string()));
    }

    fn call(src: &str) -> Option<(String, usize)> {
        let (text, c) = at(src);
        let an = Analysis::new(&text);
        call_at(&an, c).map(|c| (c.callee, c.arg))
    }

    #[test]
    fn signature_arg_index_for_nested_calls_and_strings() {
        assert_eq!(call("editor.tool(|"), Some(("editor.tool".into(), 0)));
        assert_eq!(call("editor.tool(\"a\", |"), Some(("editor.tool".into(), 1)));
        // a comma inside a string or a nested call does not count
        assert_eq!(call("editor.tool(\"a, b\", |"), Some(("editor.tool".into(), 1)));
        assert_eq!(call("editor.tool(\"a\", {x = f(1, 2), y = 3|"), Some(("editor.tool".into(), 1)));
        assert_eq!(call("string.sub(s, math.max(1, |"), Some(("math.max".into(), 1)));
        assert_eq!(call("string.sub(s, math.max(1, 2), |"), Some(("string.sub".into(), 2)));
        assert_eq!(call("print('(', |"), Some(("print".into(), 1)), "a ( in a string is not a call");
        assert_eq!(call("print([[,,(]], -- a, (\n |"), Some(("print".into(), 1)), "long strings and comments");
        assert_eq!(call("s:sub(1, |"), Some(("s:sub".into(), 1)));
        // closed calls, definitions and bodies are not calls
        assert_eq!(call("print(1) |"), None);
        assert_eq!(call("local function f(a, |"), None);
        assert_eq!(call("pcall(function() local x = |"), None);
        // and the signature: the parameter list with the current one known
        let (text, c) = at("string.sub(s, |");
        let an = Analysis::new(&text);
        let sig = signature(&an, &call_at(&an, c).unwrap(), SCRIPT).unwrap();
        assert_eq!(sig.name, "string.sub");
        assert_eq!(sig.params.len(), 3);
        // a method drops self
        let (text, c) = at("s:sub(|");
        let an = Analysis::new(&text);
        let sig = signature(&an, &call_at(&an, c).unwrap(), SCRIPT).unwrap();
        assert_eq!(sig.params.len(), 2, "{sig:?}");
        // the file's own function
        let (text, c) = at("local function area(w, h) return w * h end\nprint(area(2, |");
        let an = Analysis::new(&text);
        let sig = signature(&an, &call_at(&an, c).unwrap(), SCRIPT).unwrap();
        assert_eq!((sig.name.as_str(), sig.params.clone()), ("area", vec!["w".to_string(), "h".to_string()]));
    }

    #[test]
    fn hover_docs_on_api_names() {
        let src = "local r = editor.tool(\"timeline.list\", {})\nlocal s = math.floor(1.5)\nlocal u = s:upper()";
        let an = Analysis::new(src);
        let (sig, doc) = hover(&an, src.find("tool(").unwrap() + 1, SCRIPT).unwrap();
        assert!(sig.starts_with("editor.tool(name: string") && !doc.is_empty(), "{sig}");
        assert!(hover(&an, src.find("floor").unwrap(), SCRIPT).unwrap().0.starts_with("math.floor("));
        assert!(hover(&an, src.find("math").unwrap(), SCRIPT).unwrap().0.starts_with("math"));
        assert!(hover(&an, src.find("upper").unwrap(), SCRIPT).unwrap().0.starts_with("string.upper("));
        assert!(hover(&an, src.find("timeline.list").unwrap() + 2, SCRIPT).is_some(), "a tool name in editor.tool");
        assert_eq!(hover(&an, src.find("local").unwrap(), SCRIPT), None);
        assert_eq!(hover(&an, src.find(" r ").unwrap() + 1, SCRIPT), None);
    }

    #[test]
    fn syntax_diagnostic_line_number() {
        let d = syntax("local a = 1\nlocal b = 2\nlocal c = = 3\nprint(a)\n").expect("a syntax error");
        assert_eq!(d.line, 3, "{d:?}");
        assert!(d.error && !d.msg.is_empty() && !d.msg.contains("script:"), "{d:?}");
        assert_eq!(&"local a = 1\nlocal b = 2\nlocal c = = 3\nprint(a)\n"[d.range.clone()], "local c = = 3");
        // an unclosed block is reported where Luau says (the end of the file), and stays inside the text
        let src = "function f()\n  return 1\n";
        let d = syntax(src).unwrap();
        assert!(d.line >= 1 && d.line <= 2 && d.range.end <= src.len(), "{d:?}");
        assert_eq!(syntax("local ok = true\n"), None);
        assert_eq!(syntax(crate::scripting::TEMPLATE), None, "the template compiles");
    }

    #[test]
    fn unknown_globals_are_warnings() {
        let src = "local x: number = 1\nfunction f(a: string?): boolean return a ~= nil end\n\
                   type P = { name: string, size: number }\nlocal t = { key = 1 }\ng = 2\n\
                   print(x, t.key, g, editor.tool, mystery, s:upper())\nlocal y = (x :: any)\n";
        let an = Analysis::new(src);
        let d = unknown_globals(&an, SCRIPT);
        let names: Vec<&str> = d.iter().map(|d| &src[d.range.clone()]).collect();
        assert_eq!(names, vec!["mystery", "s"], "{d:?}");
        assert!(d.iter().all(|d| !d.error && d.line == 6));
        // the context decides what is known
        assert!(unknown_globals(&Analysis::new("library.assets()"), SCRIPT).len() == 1);
        assert!(unknown_globals(&Analysis::new("library.assets()"), AGENT).is_empty());
        assert!(
            check(crate::scripting::TEMPLATE, SCRIPT).is_empty(),
            "{:?}",
            check(crate::scripting::TEMPLATE, SCRIPT)
        );
    }

    #[test]
    fn hook_headers_pick_the_context() {
        assert_eq!(
            context_of("-- @desc x\n-- @on selection_changed\nprint(1)", false),
            SCRIPT | crate::scripting_api::HOOK
        );
        assert_eq!(context_of("print(1)\n-- @on later", false), SCRIPT);
        assert_eq!(context_of("", true), PLUGIN);
    }

    /// The check of a 2,000-line script (run on the pane's worker thread): Luau's compile plus the scan.
    #[test]
    fn check_cost_for_2000_lines() {
        let mut src = String::new();
        for i in 0..400 {
            src.push_str(&format!(
                "do local v{i} = editor.tool(\"timeline.list\", {{ n = {i} }})\nif v{i} and #v{i} > 0 then\n  \
                 editor.log(\"track \" .. tostring(v{i}[1]) .. string.format(\"%d\", {i}))\nend end\n\n"
            ));
        }
        assert_eq!(src.lines().count(), 2000);
        let _ = check(&src, SCRIPT); // warm
        let t = std::time::Instant::now();
        let n = 5;
        for _ in 0..n {
            let d = check(&src, SCRIPT);
            assert!(d.is_empty(), "{:?}", &d[..d.len().min(3)]);
        }
        let all = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = Analysis::new(&src);
        }
        let scan = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("check of 2000 lines: {all:.2} ms (lex + scan {scan:.2} ms, the rest is Luau's compile)");
        // it runs off the UI thread; this only catches an accidental blow-up
        assert!(all < 500.0, "{all} ms");
    }
}
