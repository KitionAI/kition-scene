//! Every command declares the editor state it reads (`KitCommand::reads`, and this kit's own record
//! of the mode, `kition_scene_kit::acts::reads_mode`): what the engine's code reads, found by
//! reading it, not what descriptions say. Each command's code is found in the kit's sources (the
//! match arms and `command == "…"` branches that name it, with the code before them in the same
//! function, and the `Needs` gate its spec names), and followed through the kit's functions, the
//! document's methods and the commands it runs (`run_view(doc, "edit.selectAll", …)`) to the
//! accessors of the selection and active object, of the current frame, and of the mode and the
//! edited mesh's selected elements. A command that reaches one and does not declare it, or declares
//! what it never reads, fails here (task card C4-scene). The commands are the ones the document's
//! `commands()` lists at run time. To see how the scan reached a reading, set
//! `ACTS_DEBUG=<command id>` or `ACTS_FN=<function>`; `ACTS_PRINT=1` prints the derived table.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;

/// Code that reads the object selection or the active object.
const SELECTION: &[&str] = &["editor.selected", "editor.active"];
/// Code that reads the current frame: the frame itself, and the scene rendered or exported whole as
/// it is at the current frame (`render_png`, `export_scene`); the engine calls that read it where
/// the scan cannot follow: the viewport picture a sculpt finds its surface in (`picture(doc)`, drawn
/// at the current frame), Cryptomatte's names under a point of the scene as it stands
/// (`passes::names_at`), a particle system baked at the current frame (`particles::realize(&only`,
/// `particle.convert`), and imported objects taking their values at the current frame
/// (`kition_scene_exchange::place`).
const TIME: &[&str] = &[
    "timeline.frame",
    "render_png(",
    "export_scene(",
    "picture(doc)",
    "passes::names_at(",
    "particles::realize(&only",
    "kition_scene_exchange::place(",
];
/// Code that reads the mode (object, a mesh's edit mode, an armature's edit or pose mode, weight
/// paint, a curve's, text's or lattice's edit mode, sculpt, Particle Edit) or what it selects (the
/// edited mesh's vertices and the element kind clicks select, proportional editing).
const MODE: &[&str] = &[
    "session.mode",
    "session.vertices",
    "session.select_mode",
    "session.proportional",
    "session.rig",
    "session.paint",
    "session.shape",
    "session.sculpt",
    "session.particle_edit",
];
/// The document's bookkeeping every command goes through (the undo step, putting back a failed
/// command, tidying the session after a change, the picture's revision): no command's own reading.
const BOOKKEEPING: &[&str] = &["step", "restore", "committed", "tidy_session", "touched", "view_from_replica", "replica_changed", "tidy"];
/// The dispatchers that route a command to its code (and the catalogue's gates, read through the
/// spec's `Needs`): a call to one is not the command's own reading.
const DISPATCH: &[&str] = &[
    "render",
    "with_elements",
    "execute",
    "run_view",
    "run_change",
    "change",
    "run",
    "catalogue",
    "disabled",
    "spec_of",
    "spec_enabled",
    "enabled",
    "refuse_unknown",
    "fill",
];
/// Commands whose code the kit reaches by a route that names no id, as (command, file, function): a
/// module's one view command (`if !spec.mutates { return select(doc, params) }`), the default arm of
/// a match, a constant's id (`command == crate::render::RENDER`).
const ROUTES: &[(&str, &str, &str)] = &[
    ("lattice.select", "commands_lattice", "select"),
    ("compositor.preview", "commands_compositor", "preview"),
    ("node.describe", "commands_nodes", "describe"),
    ("graph.removeModifier", "commands_nla", "modifiers"),
    // The projections `run` maps to a `Projection` (`"uv.cubeProject" => Some(Projection::Cube)`)
    // and gives the meshes `uv_targets` chooses.
    ("uv.cubeProject", "commands_uv", "uv_targets"),
    ("uv.cylinderProject", "commands_uv", "uv_targets"),
    ("uv.sphereProject", "commands_uv", "uv_targets"),
    ("uv.planarProject", "commands_uv", "uv_targets"),
    ("uv.reset", "commands_uv", "uv_targets"),
    ("file.render", "render", "render"),
];
/// The kit's modules: a call qualified by one of them (`crate::commands_model::f`, `params::f`) is to
/// the kit's own code.
const MODULES: &[&str] = &["crate", "super", "self", "params", "commands", "render", "inspect", "tools", "check", "collab", "files", "formats", "gl"];

fn sources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// `source` with every comment, string and character literal blanked (to spaces), so brackets and
/// names are only the code's; same length, byte for byte.
fn code_only(source: &str) -> Vec<u8> {
    let b = source.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for byte in out.iter_mut().take(to).skip(from) {
            *byte = b' ';
        }
    };
    while i < b.len() {
        let ident_before = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if b[i..].starts_with(b"//") {
            let end = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |n| i + n);
            blank(&mut out, i, end);
            i = end;
        } else if b[i..].starts_with(b"/*") {
            let end = source[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
            blank(&mut out, i, end);
            i = end;
        } else if !ident_before && (b[i] == b'r' || b[i..].starts_with(b"br")) && {
            let start = if b[i] == b'b' { i + 2 } else { i + 1 };
            let hashes = b[start..].iter().take_while(|&&c| c == b'#').count();
            b.get(start + hashes) == Some(&b'"')
        } {
            let start = if b[i] == b'b' { i + 2 } else { i + 1 };
            let hashes = b[start..].iter().take_while(|&&c| c == b'#').count();
            let close = format!("\"{}", "#".repeat(hashes));
            let body = start + hashes + 1;
            let end = source[body..].find(&close).map_or(b.len(), |n| body + n + close.len());
            blank(&mut out, i, end);
            i = end;
        } else if b[i] == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            blank(&mut out, i, (j + 1).min(b.len()));
            i = j + 1;
        } else if b[i] == b'\'' {
            // A character literal ('a', '\n', '{'), not a lifetime ('a).
            let rest = &source[i + 1..];
            let len = if let Some(escaped) = rest.strip_prefix('\\') {
                escaped.find('\'').map(|n| n + 2)
            } else {
                rest.chars().next().map(char::len_utf8).filter(|&n| rest[n..].starts_with('\''))
            };
            match len {
                Some(len) => {
                    blank(&mut out, i, i + len + 2);
                    i += len + 2;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The position after the bracket matching the one at `open`.
fn matching(code: &[u8], open: usize) -> usize {
    let (left, right) = match code[open] {
        b'(' => (b'(', b')'),
        b'{' => (b'{', b'}'),
        _ => (b'[', b']'),
    };
    let mut depth = 0usize;
    for (at, &c) in code.iter().enumerate().skip(open) {
        if c == left {
            depth += 1;
        } else if c == right {
            depth -= 1;
            if depth == 0 {
                return at + 1;
            }
        }
    }
    code.len()
}

/// Where the expression starting at `from` ends: at a comma, `;` or closing bracket at its own
/// depth.
fn expression_end(code: &[u8], from: usize) -> usize {
    let mut at = from;
    while at < code.len() {
        match code[at] {
            b'(' | b'[' | b'{' => at = matching(code, at),
            b',' | b';' | b')' | b']' | b'}' => return at,
            _ => at += 1,
        }
    }
    code.len()
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn ident_at(code: &[u8], at: usize) -> &str {
    let end = code[at..].iter().position(|c| !is_ident(*c)).map_or(code.len(), |n| at + n);
    std::str::from_utf8(&code[at..end]).unwrap_or("")
}

/// One source file: the source cut before its `#[cfg(test)]` module, and its code.
struct File {
    name: String,
    source: String,
    code: Vec<u8>,
}

fn files() -> Vec<File> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(sources())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        // `described.rs` holds the commands' descriptions and schemas, no code of theirs.
        .filter(|path| {
            path.extension().is_some_and(|e| e == "rs") && !path.to_string_lossy().ends_with("_tests.rs") && !path.ends_with("described.rs")
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let mut source = std::fs::read_to_string(&path).unwrap();
            if let Some(at) = source.find("#[cfg(test)]\nmod ") {
                source.truncate(at);
            }
            let code = code_only(&source);
            File { name: path.file_stem().unwrap().to_string_lossy().into_owned(), source, code }
        })
        .collect()
}

/// What a piece of code reads directly, the kit's functions it calls (with the module a qualified
/// call names), and the commands it runs.
#[derive(Default, Clone, Debug)]
struct Reach {
    reads: BTreeSet<&'static str>,
    calls: BTreeSet<(Option<String>, String)>,
    runs: BTreeSet<String>,
}

/// Whether `token` is read (not written) somewhere in `text`.
fn reads_token(text: &str, code: &[u8], token: &str) -> bool {
    text.match_indices(token).any(|(at, _)| {
        let end = at + token.len();
        let before = at > 0 && is_ident(code[at - 1]);
        // `timeline.frame` is the field, not a method `timeline.frame(t)`.
        let whole = token.ends_with('(') || !code.get(end).copied().is_some_and(|c| is_ident(c) || c == b'(');
        // Setting it (`editor.selected = ids`, `editor.selected.push(id)`) is no read.
        let rest = text[end..].trim_start();
        let written = (rest.starts_with('=') && !rest.starts_with("=="))
            || [".clear()", ".push(", ".retain(", ".extend(", ".truncate(", ".dedup(", ".insert(", ".remove("]
                .iter()
                .any(|write| rest.starts_with(write));
        !before && whole && !written
    })
}

/// What `code` (a span of a file, `source` the same span of its source) reads, calls and runs.
fn reach(code: &[u8], source: &str, commands: &BTreeSet<String>) -> Reach {
    let text = std::str::from_utf8(code).unwrap_or("");
    let mut found = Reach::default();
    for (kind, tokens) in [("selection", SELECTION), ("time", TIME), ("mode", MODE)] {
        if tokens.iter().any(|token| reads_token(text, code, token)) {
            found.reads.insert(kind);
        }
    }
    let mut at = 0;
    while at < code.len() {
        let starts = (code[at].is_ascii_alphabetic() || code[at] == b'_') && (at == 0 || !is_ident(code[at - 1]));
        if !starts {
            at += 1;
            continue;
        }
        let name = ident_at(code, at).to_owned();
        let after = at + name.len();
        let call = code.get(after) == Some(&b'(') || (code.get(after) == Some(&b':') && code.get(after + 2) == Some(&b'<'));
        if call && name != "Some" && name != "Ok" && name != "Err" {
            // `doc.f(`, `self.f(`: the document's method. `a::b::f(`: the module `b`'s, when it is the kit's.
            let method = at > 0 && code[at - 1] == b'.';
            if method {
                let receiver = text[..at - 1].trim_end();
                if receiver.ends_with("doc") || receiver.ends_with("self") {
                    found.calls.insert((Some(".".to_owned()), name.clone()));
                }
            } else if at >= 2 && &code[at - 2..at] == b"::" {
                let mut from = at - 2;
                while from > 0 && is_ident(code[from - 1]) {
                    from -= 1;
                }
                let module = ident_at(code, from).to_owned();
                let first = {
                    let mut start = from;
                    while start >= 2 && &code[start - 2..start] == b"::" {
                        let mut s = start - 2;
                        while s > 0 && is_ident(code[s - 1]) {
                            s -= 1;
                        }
                        start = s;
                    }
                    ident_at(code, start).to_owned()
                };
                if module.starts_with("commands_") || MODULES.contains(&module.as_str()) || (first == "crate" && module != "crate") {
                    found.calls.insert((Some(module), name.clone()));
                } else if module == "crate" || module == "super" || module == "self" {
                    found.calls.insert((None, name.clone()));
                }
            } else {
                found.calls.insert((None, name.clone()));
            }
            // `run_view(doc, "edit.selectAll", …)`: the command runs another, and reads what it reads.
            if code.get(after) == Some(&b'(') {
                let args = &source[after + 1..];
                let args = args.trim_start();
                if let Some(rest) = args.strip_prefix("doc,").or_else(|| args.strip_prefix("self,")) {
                    let rest = rest.trim_start();
                    if let Some(literal) = rest.strip_prefix('"')
                        && let Some(end) = literal.find('"')
                        && commands.contains(&literal[..end])
                    {
                        found.runs.insert(literal[..end].to_owned());
                    }
                }
            }
        }
        at = after;
    }
    found
}

/// A function's body.
struct Function {
    file: String,
    start: usize,
    end: usize,
    reach: Reach,
}

/// Every `fn` body, by name.
fn functions(files: &[File], commands: &BTreeSet<String>) -> BTreeMap<String, Vec<Function>> {
    let mut out: BTreeMap<String, Vec<Function>> = BTreeMap::new();
    for file in files {
        let code = &file.code;
        let text = std::str::from_utf8(code).unwrap_or("");
        for (at, _) in text.match_indices("fn ") {
            if at > 0 && is_ident(code[at - 1]) {
                continue;
            }
            let name = ident_at(code, at + 3).to_owned();
            if name.is_empty() {
                continue;
            }
            let Some(open) = code[at..].iter().position(|&c| c == b'(').map(|n| at + n) else { continue };
            let params_end = matching(code, open);
            let Some(brace) = code[params_end..].iter().position(|&c| c == b'{' || c == b';').map(|n| params_end + n) else { continue };
            if code[brace] == b';' {
                continue;
            }
            let end = matching(code, brace);
            let reach = reach(&code[brace..end], &file.source[brace..end], commands);
            out.entry(name).or_default().push(Function { file: file.name.clone(), start: brace, end, reach });
        }
    }
    out
}

/// Where the code of one command is: a match arm whose patterns name it, or the block of an
/// `if command == "…"`, or the rest of a function after `if command != "…" { return … }`.
struct Branch {
    file: String,
    /// The branch's own span.
    start: usize,
    end: usize,
    /// Where the code that runs before it ends: the opening brace of its `match`, or its `if`.
    head: usize,
    ids: Vec<String>,
}

/// The string literals (command ids) in `pattern`, when it is made of them alone (`"a" | "b"`),
/// with or without a guard (`"a" if …`).
fn literal_ids(pattern: &str, commands: &BTreeSet<String>) -> Option<Vec<String>> {
    let mut ids = Vec::new();
    let pattern = pattern.split(" if ").next().unwrap_or(pattern);
    // `("uv.clear", _)`: the id is the tuple's first element.
    let pattern = match pattern.strip_prefix('(') {
        Some(inner) => inner.split(',').next().unwrap_or(inner),
        None => pattern,
    };
    for part in pattern.split('|') {
        let part = part.trim();
        let id = part.strip_prefix('"')?.strip_suffix('"')?;
        if !commands.contains(id) {
            return None;
        }
        ids.push(id.to_owned());
    }
    (!ids.is_empty()).then_some(ids)
}

fn branches(files: &[File], commands: &BTreeSet<String>) -> Vec<Branch> {
    let mut out = Vec::new();
    for file in files {
        let (code, source) = (&file.code, &file.source);
        let text = std::str::from_utf8(code).unwrap_or("");
        // Match arms: `"a" | "b" => body`.
        for (arrow, _) in text.match_indices("=>") {
            let (mut from, mut depth) = (arrow, 0i32);
            while from > 0 && !(depth == 0 && matches!(code[from - 1], b'{' | b',' | b'}' | b';')) {
                match code[from - 1] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    _ => {}
                }
                if depth < 0 {
                    break;
                }
                from -= 1;
            }
            let pattern: String = source[from..arrow].lines().map(|line| line.split("//").next().unwrap_or("")).collect::<Vec<_>>().join(" ");
            let Some(ids) = literal_ids(pattern.trim(), commands) else { continue };
            let mut body = arrow + 2;
            while body < code.len() && code[body].is_ascii_whitespace() {
                body += 1;
            }
            let end = if code.get(body) == Some(&b'{') { matching(code, body) } else { expression_end(code, body) };
            out.push(Branch { file: file.name.clone(), start: from, end, head: enclosing_block_start(code, from), ids });
        }
        // `command == "a"` / `command != "a"` conditions.
        for operator in ["command == \"", "command != \""] {
            for (at, _) in source.match_indices(operator) {
                let literal = &source[at + operator.len()..];
                let Some(close) = literal.find('"') else { continue };
                let id = &literal[..close];
                if !commands.contains(id) {
                    continue;
                }
                let after = at + operator.len() + close + 1;
                let Some(brace) = code[after..].iter().position(|&c| c == b'{' || c == b';' || c == b'\n').map(|n| after + n) else { continue };
                if code[brace] != b'{' {
                    continue;
                }
                let block_end = matching(code, brace);
                let (start, end) = if operator.starts_with("command !=") {
                    // `if command != "a" { return None; }`: the rest of the function is a's.
                    let enclosing = enclosing_block_end(code, at);
                    (block_end, enclosing)
                } else {
                    (at, block_end)
                };
                out.push(Branch { file: file.name.clone(), start, end, head: at, ids: vec![id.to_owned()] });
            }
        }
    }
    out
}

/// The opening brace of the innermost `{…}` block holding `at`.
fn enclosing_block_start(code: &[u8], at: usize) -> usize {
    let mut depth = 0i32;
    for index in (0..at).rev() {
        match code[index] {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    return index;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    0
}

/// The end of the innermost `{…}` block holding `at`.
fn enclosing_block_end(code: &[u8], at: usize) -> usize {
    let mut depth = 0i32;
    for (index, &c) in code.iter().enumerate().skip(at) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return index;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    code.len()
}

/// The `Needs` gate each spec names, by command id and file: the `Needs::X` in the innermost
/// bracket group holding the id's literal (a `spec(…)` call or a `Spec { … }` literal).
fn gates(files: &[File], commands: &BTreeSet<String>) -> BTreeMap<String, Vec<(String, String)>> {
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for file in files {
        let (code, source) = (&file.code, &file.source);
        for (at, _) in source.match_indices('"') {
            let rest = &source[at + 1..];
            let Some(close) = rest.find('"') else { continue };
            let id = &rest[..close];
            if !commands.contains(id) || code[at] != b' ' {
                continue;
            }
            // The innermost open bracket before the literal.
            let mut depth = 0i32;
            let mut open = None;
            for index in (0..at).rev() {
                match code[index] {
                    b')' | b'}' | b']' => depth += 1,
                    b'(' | b'{' | b'[' => {
                        if depth == 0 {
                            open = Some(index);
                            break;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
            }
            let Some(open) = open else { continue };
            let group = std::str::from_utf8(&code[open..matching(code, open)]).unwrap_or("");
            let first = group.find("Needs::").map(|n| ident_at(group.as_bytes(), n + 7).to_owned());
            if let Some(needs) = first {
                out.entry(id.to_owned()).or_default().push((file.name.clone(), needs));
            }
        }
    }
    out
}

/// The scan's working set: functions, branches, and the reads found so far.
struct Scan {
    files: Vec<File>,
    functions: BTreeMap<String, Vec<Function>>,
    /// Each function's reads (to a fixed point over its calls), by name and file.
    reads: BTreeMap<(String, String), BTreeSet<&'static str>>,
}

impl Scan {
    fn new(commands: &BTreeSet<String>) -> Self {
        let files = files();
        let functions = functions(&files, commands);
        let mut scan = Scan { files, functions, reads: BTreeMap::new() };
        for (name, defs) in &scan.functions {
            for def in defs {
                scan.reads.insert((name.clone(), def.file.clone()), def.reach.reads.clone());
            }
        }
        loop {
            let mut changed = false;
            let mut next = scan.reads.clone();
            for (name, defs) in &scan.functions {
                for def in defs {
                    let mut now = next[&(name.clone(), def.file.clone())].clone();
                    for call in &def.reach.calls {
                        if &call.1 != name {
                            now.extend(scan.resolve(call, &def.file));
                        }
                    }
                    if next[&(name.clone(), def.file.clone())] != now {
                        next.insert((name.clone(), def.file.clone()), now);
                        changed = true;
                    }
                }
            }
            scan.reads = next;
            if !changed {
                break;
            }
        }
        scan
    }

    /// What a call reads: the function of its module when qualified, else the caller's file's own,
    /// else every one of that name. Bookkeeping and dispatch read nothing of their own.
    fn resolve(&self, (module, name): &(Option<String>, String), file: &str) -> BTreeSet<&'static str> {
        if BOOKKEEPING.contains(&name.as_str()) || DISPATCH.contains(&name.as_str()) {
            return BTreeSet::new();
        }
        let Some(defs) = self.functions.get(name) else { return BTreeSet::new() };
        let module = module.as_deref().filter(|module| !MODULES.contains(module) || matches!(*module, "params" | "commands" | "render" | "."));
        let chosen: Vec<&Function> = match module {
            // A method of the document, defined in any of the kit's files.
            Some(".") => defs.iter().collect(),
            Some(module) => defs.iter().filter(|def| def.file == module).collect(),
            None => {
                // An unqualified call is to the file's own function, else to one its `use` brings
                // in (a closure or a local of the same name is neither).
                let own: Vec<&Function> = defs.iter().filter(|def| def.file == file).collect();
                if !own.is_empty() {
                    own
                } else if self.imports(file, name) {
                    defs.iter().collect()
                } else {
                    Vec::new()
                }
            }
        };
        chosen.iter().flat_map(|def| self.reads.get(&(name.clone(), def.file.clone())).cloned().unwrap_or_default()).collect()
    }

    /// Whether `file`'s `use` statements name `name`.
    fn imports(&self, file: &str, name: &str) -> bool {
        let Some(file) = self.files.iter().find(|f| f.name == file) else { return false };
        file.source.split(';').filter(|statement| statement.trim_start().starts_with("use ") || statement.contains("\nuse ")).any(|statement| {
            statement.match_indices(name).any(|(at, _)| {
                let bytes = statement.as_bytes();
                !(at > 0 && is_ident(bytes[at - 1])) && !bytes.get(at + name.len()).copied().is_some_and(is_ident)
            })
        })
    }

    fn reach_of(&self, file: &File, start: usize, end: usize, commands: &BTreeSet<String>) -> (BTreeSet<&'static str>, BTreeSet<String>) {
        let found = reach(&file.code[start..end], &file.source[start..end], commands);
        let mut reads = found.reads.clone();
        for call in &found.calls {
            reads.extend(self.resolve(call, &file.name));
        }
        (reads, found.runs)
    }
}

/// What every command reads (`selection`, `time`, `mode`), through the functions and commands it
/// reaches.
fn derived(commands: &BTreeSet<String>) -> (BTreeMap<String, BTreeSet<&'static str>>, BTreeSet<String>) {
    let scan = Scan::new(commands);
    let branches = branches(&scan.files, commands);
    let gates = gates(&scan.files, commands);
    let file_of = |name: &str| scan.files.iter().find(|file| file.name == name).unwrap();
    let mut table: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    let mut runs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut found: BTreeSet<String> = BTreeSet::new();
    for branch in &branches {
        let file = file_of(&branch.file);
        let (mut reads, ran) = scan.reach_of(file, branch.start, branch.end, commands);
        // The code before the branch in its function (another command's branch left out) runs for
        // it too.
        if let Some(function) = scan.functions.values().flatten().find(|f| {
            f.file == branch.file && f.start <= branch.start && branch.end <= f.end && {
                // The innermost function.
                !scan
                    .functions
                    .values()
                    .flatten()
                    .any(|g| g.file == branch.file && g.start > f.start && g.start <= branch.start && branch.end <= g.end)
            }
        }) {
            let head = branch.head.max(function.start);
            let mut code = file.code[function.start..head].to_vec();
            for other in branches.iter().filter(|other| other.file == branch.file && other.end <= head && other.start >= function.start) {
                for byte in &mut code[other.start - function.start..other.end - function.start] {
                    *byte = b' ';
                }
            }
            let prefix = reach(&code, &file.source[function.start..head], commands);
            reads.extend(prefix.reads.iter().copied());
            for call in &prefix.calls {
                let resolved = scan.resolve(call, &file.name);
                if !resolved.is_empty() && branch.ids.iter().any(|id| std::env::var("ACTS_DEBUG").is_ok_and(|wanted| &wanted == id)) {
                    eprintln!("  prefix calls {call:?}: {resolved:?}");
                }
                reads.extend(resolved);
            }
            if !prefix.reads.is_empty() && branch.ids.iter().any(|id| std::env::var("ACTS_DEBUG").is_ok_and(|wanted| &wanted == id)) {
                eprintln!("  prefix reads {:?}", prefix.reads);
            }
        }
        for id in &branch.ids {
            found.insert(id.clone());
            table.entry(id.clone()).or_default().extend(reads.iter().copied());
            runs.entry(id.clone()).or_default().extend(ran.iter().cloned());
            if std::env::var("ACTS_DEBUG").is_ok_and(|wanted| &wanted == id) {
                eprintln!("{id}: branch in {} at {}..{}: {reads:?}", branch.file, branch.start, branch.end);
            }
        }
    }
    for (id, file, function) in ROUTES {
        let def =
            scan.functions.get(*function).into_iter().flatten().find(|def| def.file == *file).unwrap_or_else(|| panic!("no fn {function} in {file}"));
        let (reads, ran) = scan.reach_of(file_of(file), def.start, def.end, commands);
        found.insert((*id).to_owned());
        table.entry((*id).to_owned()).or_default().extend(reads);
        runs.entry((*id).to_owned()).or_default().extend(ran);
    }
    // The gate the spec names.
    for (id, named) in &gates {
        for (file, needs) in named {
            let file = file_of(file);
            let arm = format!("Needs::{needs} =>");
            let text = std::str::from_utf8(&file.code).unwrap_or("");
            for (at, _) in text.match_indices(&arm) {
                let mut body = at + arm.len();
                while body < file.code.len() && file.code[body].is_ascii_whitespace() {
                    body += 1;
                }
                let end = if file.code.get(body) == Some(&b'{') { matching(&file.code, body) } else { expression_end(&file.code, body) };
                let (reads, _) = scan.reach_of(file, body, end, commands);
                if std::env::var("ACTS_DEBUG").is_ok_and(|wanted| &wanted == id) {
                    eprintln!("{id}: gate {needs} in {}: {reads:?}", file.name);
                }
                table.entry(id.clone()).or_default().extend(reads);
            }
        }
    }
    if let Ok(wanted) = std::env::var("ACTS_FN") {
        for def in scan.functions.get(&wanted).into_iter().flatten() {
            eprintln!("fn {wanted} in {}: direct {:?}", def.file, def.reach.reads);
            for call in &def.reach.calls {
                eprintln!("  calls {call:?}: {:?}", scan.resolve(call, &def.file));
            }
        }
    }
    // Commands that run other commands read what those read.
    loop {
        let mut changed = false;
        for (id, others) in &runs {
            let mut now = table.get(id).cloned().unwrap_or_default();
            for other in others {
                now.extend(table.get(other).cloned().unwrap_or_default());
            }
            if table.get(id) != Some(&now) {
                table.insert(id.clone(), now);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    (table, found)
}

#[test]
fn every_command_declares_the_editor_state_its_code_reads() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let commands = doc.commands();
    let ids: BTreeSet<String> = commands.iter().map(|command| command.id.clone()).collect();
    let (derived, found) = derived(&ids);
    let missing: Vec<&String> = ids.iter().filter(|id| !found.contains(*id)).collect();
    assert!(missing.is_empty(), "the scan found no code for {} commands: {missing:?}", missing.len());
    if std::env::var("ACTS_PRINT").is_ok() {
        let table: BTreeMap<&String, Vec<&str>> =
            derived.iter().filter(|(_, reads)| !reads.is_empty()).map(|(id, reads)| (id, reads.iter().copied().collect())).collect();
        eprintln!("{}", serde_json::to_string_pretty(&table).unwrap());
        let targets: Vec<(&String, &str)> = commands.iter().filter_map(|command| Some((&command.id, command.targets?))).collect();
        eprintln!("{} of {} commands declare targets: {targets:?}", targets.len(), commands.len());
    }
    let mut wrong = Vec::new();
    for command in &commands {
        let mut declared: BTreeSet<&'static str> = command.reads.iter().map(|state| state.as_str()).collect();
        if kition_scene_kit::acts::reads_mode(&command.id) {
            declared.insert("mode");
        }
        let expected = derived.get(&command.id).cloned().unwrap_or_default();
        if declared != expected {
            wrong.push(format!("{}: declares {declared:?}, its code reads {expected:?}", command.id));
        }
    }
    assert!(wrong.is_empty(), "{} commands declare other editor state than they read:\n{}", wrong.len(), wrong.join("\n"));
}
