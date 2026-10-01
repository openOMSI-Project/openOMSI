//! Tokenizer and compiler: `.osc` text → [`Program`].

use crate::{BlockId, ConstFile, Curve, NameId, Op, StrVarId, SysVar, VarId};
use hashbrown::HashMap;
use omsi_cfg::CfgFile;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file.display(), self.line, self.message)
    }
}

/// A compiled block of instructions.
#[derive(Debug, Clone, Default)]
pub struct Block {
    pub name: String,
    pub ops: Vec<Op>,
    pub file: PathBuf,
    pub line: usize,
}

/// A compiled script set for one object type.
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub var_names: Vec<String>,
    pub str_var_names: Vec<String>,
    pub names: Vec<String>,
    pub curves: Vec<Curve>,
    pub blocks: Vec<Block>,
    pub init: Vec<BlockId>,
    pub frame: Vec<BlockId>,
    pub frame_ai: Vec<BlockId>,
    pub macros: HashMap<String, BlockId>,
    pub triggers: HashMap<String, BlockId>,
    pub errors: Vec<ScriptError>,
    /// `[const]` values of the constfiles (lower-case names).
    consts: HashMap<String, f32>,
    var_index: HashMap<String, VarId>,
    str_var_index: HashMap<String, StrVarId>,
    name_index: HashMap<String, NameId>,
}

impl Program {
    pub fn var(&self, name: &str) -> Option<VarId> {
        self.var_index.get(&name.to_ascii_lowercase()).copied()
    }
    /// The name of a variable (lower case); a linear search, for diagnostics.
    pub fn var_name(&self, id: VarId) -> Option<&str> {
        self.var_index.iter().find(|(_, v)| **v == id).map(|(k, _)| k.as_str())
    }
    /// Every variable name the program knows (lower case).
    pub fn var_names(&self) -> Vec<String> {
        self.var_index.keys().cloned().collect()
    }

    pub fn str_var(&self, name: &str) -> Option<StrVarId> {
        self.str_var_index.get(&name.to_ascii_lowercase()).copied()
    }
    pub fn name(&self, id: NameId) -> &str {
        &self.names[id as usize]
    }
    pub fn trigger(&self, name: &str) -> Option<BlockId> {
        self.triggers.get(&name.to_ascii_lowercase()).copied()
    }
    /// Every input trigger a script set exposes, in stable order.  A vehicle mod is free to
    /// give its ignition and starter controls its own names; callers that need to discover a
    /// control (rather than assume the stock keyboard names) use this list.
    pub fn trigger_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.triggers.keys().cloned().collect();
        out.sort();
        out
    }
    pub fn macro_block(&self, name: &str) -> Option<BlockId> {
        self.macros.get(&name.to_ascii_lowercase()).copied()
    }
    /// A `[const]` of the constfiles, as `(C.L.name)` reads it.
    pub fn constant(&self, name: &str) -> Option<f32> {
        self.consts.get(&name.to_ascii_lowercase()).copied()
    }

    /// Declare a variable (idempotent), returning its id.
    pub fn declare_var(&mut self, name: &str) -> VarId {
        let key = name.trim().to_ascii_lowercase();
        if let Some(&id) = self.var_index.get(&key) {
            return id;
        }
        let id = self.var_names.len() as VarId;
        self.var_names.push(name.trim().to_string());
        self.var_index.insert(key, id);
        id
    }

    pub fn declare_str_var(&mut self, name: &str) -> StrVarId {
        let key = name.trim().to_ascii_lowercase();
        if let Some(&id) = self.str_var_index.get(&key) {
            return id;
        }
        let id = self.str_var_names.len() as StrVarId;
        self.str_var_names.push(name.trim().to_string());
        self.str_var_index.insert(key, id);
        id
    }

    /// Whether any block of the program stores into `var`.
    pub fn stores(&self, var: VarId) -> bool {
        self.blocks.iter().any(|b| b.ops.iter().any(|op| matches!(op, Op::Store(v) if *v == var)))
    }

    /// Whether any block of the program reads variable `var` (`(L.L.name)`).
    pub fn reads(&self, var: VarId) -> bool {
        self.blocks.iter().any(|b| b.ops.iter().any(|op| matches!(op, Op::Load(v) if *v == var)))
    }

    /// Whether any block of the program reads the system variable `sys` (`(L.S.name)`).
    pub fn reads_sys(&self, sys: SysVar) -> bool {
        self.blocks.iter().any(|b| b.ops.iter().any(|op| matches!(op, Op::LoadSys(v) if *v == sys)))
    }

    /// Whether running `block` (with the macros it calls) can write variable `var` with
    /// anything but a literal 0 (`0 (S.L.a) (S.L.b)` only ever clears them).
    pub fn block_sets(&self, block: BlockId, var: VarId) -> bool {
        let mut seen = hashbrown::HashSet::new();
        self.block_sets_rec(block, var, &mut seen)
    }

    fn block_sets_rec(&self, block: BlockId, var: VarId, seen: &mut hashbrown::HashSet<BlockId>) -> bool {
        if !seen.insert(block) {
            return false;
        }
        let Some(b) = self.blocks.get(block as usize) else { return false };
        b.ops.iter().enumerate().any(|(i, op)| match op {
            Op::Store(v) if *v == var => {
                // the value stored is what the instruction before the chain of stores pushed
                let value = b.ops[..i].iter().rev().find(|o| !matches!(o, Op::Store(_)));
                !matches!(value, Some(Op::Push(x)) if *x == 0.0)
            }
            Op::Macro(m) => self.block_sets_rec(*m, var, seen),
            _ => false,
        })
    }

    /// Whether running `block` (with the macros it calls) reads variable `var`.
    pub fn block_reads(&self, block: BlockId, var: VarId) -> bool {
        let mut seen = hashbrown::HashSet::new();
        self.block_reads_rec(block, var, &mut seen)
    }

    fn block_reads_rec(&self, block: BlockId, var: VarId, seen: &mut hashbrown::HashSet<BlockId>) -> bool {
        if !seen.insert(block) {
            return false;
        }
        let Some(b) = self.blocks.get(block as usize) else { return false };
        b.ops.iter().any(|op| match op {
            Op::Load(v) => *v == var,
            Op::Macro(m) => self.block_reads_rec(*m, var, seen),
            _ => false,
        })
    }

    /// Whether the vehicle's gearbox is a manual one worked through gates (`kw_s_1`,
    /// `kw_s_2` ...): it has the gates and either no automatic's `automatic_D`, or its first
    /// gate asks for the clutch pedal (`{trigger:kw_s_1} (L.L.clutch) 1 = ...`). An
    /// automatic whose scripts answer to the gate keys as well (gear hold, a dashboard's
    /// display) and read a `Clutch` of their own somewhere (a torque converter's) is no
    /// manual - taken for one, the automatic clutch of the settings worked its clutch at
    /// every stop and pull-away.
    pub fn manual_gearbox(&self) -> bool {
        let (Some(g1), true) = (self.trigger("kw_s_1").or_else(|| self.trigger("kw_s_1_fest")), self.trigger("kw_s_2").or_else(|| self.trigger("kw_s_2_fest")).is_some()) else { return false };
        // (a script that reads OMSI's `AutoClutch` works a clutch of its own: the Sprinter
        // W906 MT, whose dashboard answers to `automatic_D` as well, #279)
        if self.trigger("automatic_D").is_none() || self.reads_sys(SysVar::AutoClutch) {
            return true;
        }
        ["Clutch", "clutch_pedal"].iter().filter_map(|n| self.var(n)).any(|v| self.block_reads(g1, v))
    }

    /// Names (lower case, sorted) of the triggers that can set variable `name` (to anything
    /// but 0): which key or switch of a vehicle turns its electrics on or cranks its
    /// engine, whatever the mod called it. The engine's own `ai_*` triggers are left out.
    pub fn triggers_setting(&self, name: &str) -> Vec<String> {
        let Some(var) = self.var(name) else { return Vec::new() };
        let mut out: Vec<String> = self.triggers.iter().filter(|(n, b)| !n.starts_with("ai_") && self.block_sets(**b, var)).map(|(n, _)| n.clone()).collect();
        out.sort();
        out
    }

    /// Every callback name the scripts call (`(M.V.name)`), in the spelling first used.
    pub fn callbacks_used(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for b in &self.blocks {
            for op in &b.ops {
                if let Op::Callback(n) = op {
                    let name = self.name(*n).to_string();
                    if !out.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
                        out.push(name);
                    }
                }
            }
        }
        out
    }

    /// String literals that are handed straight to callback `callback` (`"x" (M.V.cb)`):
    /// the font names a script asks `GetFontIndex` for, the textures of `STLoadTex`.
    pub fn literal_arguments(&self, callback: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for b in &self.blocks {
            for w in b.ops.windows(2) {
                if let (Op::PushStr(s), Op::Callback(n)) = (&w[0], &w[1]) {
                    if self.name(*n).eq_ignore_ascii_case(callback) && !out.contains(s) {
                        out.push(s.clone());
                    }
                }
            }
        }
        out
    }

    fn intern(&mut self, name: &str) -> NameId {
        let key = name.to_ascii_lowercase();
        if let Some(&id) = self.name_index.get(&key) {
            return id;
        }
        let id = self.names.len() as NameId;
        self.names.push(name.to_string());
        self.name_index.insert(key, id);
        id
    }
}

/// Everything needed to compile one object type's scripts.
#[derive(Debug, Default, Clone)]
pub struct CompileInput {
    /// Variables the host provides (e.g. `Velocity`, `Throttle` for road vehicles).
    pub builtin_vars: Vec<String>,
    pub builtin_str_vars: Vec<String>,
    pub varlists: Vec<PathBuf>,
    pub stringvarlists: Vec<PathBuf>,
    pub constfiles: Vec<PathBuf>,
    pub scripts: Vec<PathBuf>,
}

fn read_list(path: &Path, errors: &mut Vec<ScriptError>) -> Vec<String> {
    match CfgFile::read(path) {
        Ok(f) => f.lines.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        Err(e) => {
            errors.push(ScriptError { file: path.to_path_buf(), line: 0, message: e.to_string() });
            Vec::new()
        }
    }
}

/// Compile a script set.
pub fn compile(input: &CompileInput) -> Program {
    let mut p = Program::default();
    for v in &input.builtin_vars {
        p.declare_var(v);
    }
    for v in &input.builtin_str_vars {
        p.declare_str_var(v);
    }
    let mut errors = Vec::new();
    for path in &input.varlists {
        for v in read_list(path, &mut errors) {
            p.declare_var(&v);
        }
    }
    for path in &input.stringvarlists {
        for v in read_list(path, &mut errors) {
            p.declare_str_var(&v);
        }
    }
    let mut consts: HashMap<String, f32> = HashMap::new();
    let mut curve_index: HashMap<String, u32> = HashMap::new();
    for path in &input.constfiles {
        match ConstFile::load(path) {
            Ok(cf) => {
                for e in cf.errors {
                    errors.push(ScriptError { file: path.clone(), line: 0, message: e });
                }
                for (n, v) in cf.consts {
                    consts.insert(n.to_ascii_lowercase(), v);
                }
                for c in cf.curves {
                    let id = p.curves.len() as u32;
                    curve_index.insert(c.name.to_ascii_lowercase(), id);
                    p.curves.push(c);
                }
            }
            Err(e) => errors.push(ScriptError { file: path.clone(), line: 0, message: format!("cannot read constfile: {e}") }),
        }
    }
    p.errors = errors;

    // Pass 1: collect macro and trigger names so forward references across files resolve.
    let mut files = Vec::new();
    for path in &input.scripts {
        match CfgFile::read(path) {
            Ok(mut f) => {
                for fix in crate::compat::patch(&mut f.lines) {
                    log::info!("script {}: {fix}", path.display());
                }
                files.push(f)
            }
            Err(e) => p.errors.push(ScriptError { file: path.clone(), line: 0, message: e.to_string() }),
        }
    }
    let mut pending: Vec<(usize, usize, String)> = Vec::new(); // (block, op index, macro name)
    let mut c = Compiler { p: &mut p, consts: &consts, curves: &curve_index, pending: &mut pending, missing_curve: None };
    for f in &files {
        c.compile_file(f);
    }
    p.consts = consts;
    // Resolve macro calls.
    for (block, op, name) in pending {
        match p.macros.get(&name).copied() {
            Some(id) => p.blocks[block].ops[op] = Op::Macro(id),
            None => {
                let (file, line) = (p.blocks[block].file.clone(), p.blocks[block].line);
                p.errors.push(ScriptError { file, line, message: format!("SC_ErrorInCommand_macroinvalid: macro \"{name}\" not found") });
                // A missing macro leaves the stack untouched in the original; emulate by
                // replacing with a no-op pair.
                p.blocks[block].ops[op] = Op::Jump((op + 1) as u32);
            }
        }
    }
    p
}

struct Compiler<'a> {
    p: &'a mut Program,
    consts: &'a HashMap<String, f32>,
    curves: &'a HashMap<String, u32>,
    pending: &'a mut Vec<(usize, usize, String)>,
    /// The empty curve an unknown `(F.L.name)` evaluates (see `compile_access`).
    missing_curve: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Paren(String),
    Brace(String),
}

/// Tokenize one script line. `'` starts a comment (outside a string literal).
fn tokenize(line: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\'' {
            break;
        }
        if c == '"' {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && chars[j] != '"' {
                j += 1;
            }
            out.push(Tok::Str(chars[start..j.min(chars.len())].iter().collect()));
            i = j + 1;
            continue;
        }
        if c == '(' {
            // The access ends at its matching bracket: mods name variables after fonts like
            // `Font_(8-1)x3`, and `(L.L.Font_(8-1)x3)` is one access, not `(L.L.Font_(8-1)`
            // followed by a stray `x3)`. Without a match the first `)` closes it.
            let start = i + 1;
            let mut depth = 1;
            let mut j = start;
            while j < chars.len() && !chars[j].is_whitespace() {
                match chars[j] {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth != 0 {
                j = start;
                while j < chars.len() && chars[j] != ')' {
                    j += 1;
                }
            }
            out.push(Tok::Paren(chars[start..j.min(chars.len())].iter().collect::<String>().trim().to_string()));
            i = j + 1;
            continue;
        }
        if c == '{' {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && chars[j] != '}' {
                j += 1;
            }
            out.push(Tok::Brace(chars[start..j.min(chars.len())].iter().collect::<String>().trim().to_string()));
            i = j + 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < chars.len() && !chars[j].is_whitespace() && chars[j] != '(' && chars[j] != '{' && chars[j] != '"' {
            j += 1;
        }
        out.push(Tok::Word(chars[start..j].iter().collect()));
        i = j.max(start + 1);
    }
    out
}

enum BlockKind {
    Init,
    Frame,
    FrameAi,
    Macro(String),
    Trigger(String),
}

impl<'a> Compiler<'a> {
    fn err(&mut self, file: &Path, line: usize, msg: String) {
        self.p.errors.push(ScriptError { file: file.to_path_buf(), line, message: msg });
    }

    fn compile_file(&mut self, f: &CfgFile) {
        let mut cur: Option<(Vec<BlockKind>, Block, Vec<(usize, bool)>)> = None; // kinds (aliases), block, if-stack: (jump op index, has_else)
        // `{else}` without an open `{if}` (a stray `{endif}` closed it early): in OMSI a
        // reached `{else}` skips to the next `{endif}`, so it jumps there here too - the
        // Procity's dashboard blanked its odometer every frame through such an `{else}`
        let mut orphan_elses: Vec<usize> = Vec::new();
        for (ln, raw) in f.lines.iter().enumerate() {
            let line_no = ln + 1;
            let toks = tokenize(raw);
            for t in toks {
                match t {
                    Tok::Brace(b) => {
                        let bl = b.to_ascii_lowercase();
                        match bl.as_str() {
                            "init" | "frame" | "frame_ai" | _ if bl == "init" || bl == "frame" || bl == "frame_ai" || bl.starts_with("macro:") || bl.starts_with("trigger:") => {
                                let (kind, name) = match bl.as_str() {
                                    "init" => (BlockKind::Init, bl.clone()),
                                    "frame" => (BlockKind::Frame, bl.clone()),
                                    "frame_ai" => (BlockKind::FrameAi, bl.clone()),
                                    _ => {
                                        let name = b[b.find(':').unwrap() + 1..].trim().to_string();
                                        let kind = if bl.starts_with("macro:") { BlockKind::Macro(name.to_ascii_lowercase()) } else { BlockKind::Trigger(name.to_ascii_lowercase()) };
                                        (kind, name)
                                    }
                                };
                                match cur.as_mut() {
                                    // A block header directly after another header (no code yet)
                                    // makes both names refer to the same body - stock scripts
                                    // stack `{trigger:a}` `{trigger:b}` this way.
                                    Some((kinds, block, _)) if block.ops.is_empty() => kinds.push(kind),
                                    Some(_) => {
                                        let (kinds, mut block, _) = cur.take().unwrap();
                                        let end = block.ops.len() as u32;
                                        for idx in orphan_elses.drain(..) {
                                            patch(&mut block.ops[idx], end);
                                        }
                                        log::debug!("{}:{}: {{{b}}} starts while block {} is open (implicit end)", f.path.display(), line_no, block.name);
                                        self.finish_block(kinds, block);
                                        cur = Some((vec![kind], Block { name, file: f.path.clone(), line: line_no, ..Default::default() }, Vec::new()));
                                    }
                                    None => cur = Some((vec![kind], Block { name, file: f.path.clone(), line: line_no, ..Default::default() }, Vec::new())),
                                }
                            }
                            "end" => {
                                if let Some((kinds, mut block, ifs)) = cur.take() {
                                    let end = block.ops.len() as u32;
                                    for idx in orphan_elses.drain(..) {
                                        patch(&mut block.ops[idx], end);
                                    }
                                    for (idx, _) in ifs {
                                        log::debug!("{}:{}: {{end}} with open {{if}}", f.path.display(), line_no);
                                        let end = block.ops.len() as u32;
                                        patch(&mut block.ops[idx], end);
                                    }
                                    self.finish_block(kinds, block);
                                } else {
                                    // Surplus {end} tokens are common in stock content; ignored.
                                    log::debug!("{}:{}: {{end}} without block", f.path.display(), line_no);
                                }
                            }
                            "if" => {
                                if let Some((_, block, ifs)) = cur.as_mut() {
                                    ifs.push((block.ops.len(), false));
                                    block.ops.push(Op::JumpIfZero(u32::MAX));
                                } else {
                                    self.err(&f.path, line_no, "{if} outside block".into());
                                }
                            }
                            "else" => {
                                if let Some((_, block, ifs)) = cur.as_mut() {
                                    if let Some((idx, has_else)) = ifs.last_mut() {
                                        if *has_else {
                                            log::debug!("{}:{}: double {{else}}", f.path.display(), line_no);
                                        }
                                        // jump over the else branch from the end of the if branch
                                        let jmp = block.ops.len();
                                        block.ops.push(Op::Jump(u32::MAX));
                                        let target = block.ops.len() as u32;
                                        patch(&mut block.ops[*idx], target);
                                        *idx = jmp;
                                        *has_else = true;
                                    } else {
                                        log::debug!("{}:{}: {{else}} without {{if}}", f.path.display(), line_no);
                                        orphan_elses.push(block.ops.len());
                                        block.ops.push(Op::Jump(u32::MAX));
                                    }
                                } else {
                                    log::debug!("{}:{}: {{else}} outside block", f.path.display(), line_no);
                                }
                            }
                            "endif" => {
                                if let Some((_, block, ifs)) = cur.as_mut() {
                                    if let Some((idx, _)) = ifs.pop() {
                                        let target = block.ops.len() as u32;
                                        patch(&mut block.ops[idx], target);
                                    } else {
                                        log::debug!("{}:{}: {{endif}} without {{if}}", f.path.display(), line_no);
                                        let target = block.ops.len() as u32;
                                        for idx in orphan_elses.drain(..) {
                                            patch(&mut block.ops[idx], target);
                                        }
                                    }
                                } else {
                                    log::debug!("{}:{}: {{endif}} outside block", f.path.display(), line_no);
                                }
                            }
                            _ => self.err(&f.path, line_no, format!("unknown block token {{{b}}}")),
                        }
                    }
                    Tok::Str(s) => {
                        if let Some((_, block, _)) = cur.as_mut() {
                            block.ops.push(Op::PushStr(s));
                        }
                    }
                    Tok::Paren(inner) => {
                        if cur.is_none() {
                            continue;
                        }
                        let op = self.compile_access(&inner, &f.path, line_no);
                        let (_, block, _) = cur.as_mut().unwrap();
                        if let Some(Op::Macro(u32::MAX)) = op.as_ref() {
                            // placeholder resolved later
                            let name = inner[4..].trim().to_ascii_lowercase();
                            let idx = block.ops.len();
                            block.ops.push(Op::Macro(u32::MAX));
                            self.pending_push(idx, name);
                        } else if let Some(op) = op {
                            block.ops.push(op);
                        }
                    }
                    Tok::Word(w) => {
                        if cur.is_none() {
                            continue;
                        }
                        let op = self.compile_word(&w, &f.path, line_no);
                        if let Some(op) = op {
                            cur.as_mut().unwrap().1.ops.push(op);
                        }
                    }
                }
            }
        }
        if let Some((kinds, mut block, _)) = cur.take() {
            let end = block.ops.len() as u32;
            for idx in orphan_elses.drain(..) {
                patch(&mut block.ops[idx], end);
            }
            log::debug!("{}: missing {{end}} at end of file", f.path.display());
            self.finish_block(kinds, block);
        }
    }

    fn pending_push(&mut self, op_idx: usize, name: String) {
        // block index is the one that will be assigned when the current block finishes
        let block_idx = self.p.blocks.len();
        self.pending.push((block_idx, op_idx, name));
    }

    fn finish_block(&mut self, kinds: Vec<BlockKind>, block: Block) {
        let id = self.p.blocks.len() as BlockId;
        self.p.blocks.push(block);
        for kind in kinds {
            self.register(kind, id);
        }
    }

    fn register(&mut self, kind: BlockKind, id: BlockId) {
        match kind {
            BlockKind::Init => self.p.init.push(id),
            BlockKind::Frame => self.p.frame.push(id),
            BlockKind::FrameAi => self.p.frame_ai.push(id),
            BlockKind::Macro(n) => {
                self.p.macros.insert(n, id);
            }
            BlockKind::Trigger(n) => {
                self.p.triggers.insert(n, id);
            }
        }
    }

    fn compile_access(&mut self, inner: &str, file: &Path, line: usize) -> Option<Op> {
        // Form: X.Y.name  (Y may be '$')
        let b = inner.as_bytes();
        if b.len() < 5 || b[1] != b'.' || b[3] != b'.' {
            self.err(file, line, format!("SC_ErrorInCommand: malformed access ({inner})"));
            return None;
        }
        let kind = b[0].to_ascii_uppercase();
        let scope = b[2].to_ascii_uppercase();
        let name = inner[4..].trim();
        let lname = name.to_ascii_lowercase();
        match (kind, scope) {
            (b'L', b'L') => match self.p.var(&lname) {
                Some(id) => Some(Op::Load(id)),
                None => {
                    self.err(file, line, format!("SC_ErrorInCommand_varinvalid: variable \"{name}\" not found"));
                    Some(Op::Push(0.0))
                }
            },
            (b'S', b'L') => match self.p.var(&lname) {
                Some(id) => Some(Op::Store(id)),
                None => {
                    self.err(file, line, format!("SC_ErrorInCommand_varinvalid: variable \"{name}\" not found"));
                    None
                }
            },
            // (the original knows only L, S and $ in the third place: anything
            // else there - (L.M.x) too - is a system variable; OMSI has no map variables)
            (b'L', b'S' | b'M') => match SysVar::from_name(name) {
                Some(v) => Some(Op::LoadSys(v)),
                None => {
                    self.err(file, line, format!("SC_ErrorInCommand_varinvalid: system variable \"{name}\" not found"));
                    Some(Op::Push(0.0))
                }
            },
            (b'S', b'S' | b'M') => match SysVar::from_name(name) {
                Some(v) => Some(Op::StoreSys(v)),
                None => {
                    self.err(file, line, format!("SC_ErrorInCommand_varinvalid: system variable \"{name}\" not found"));
                    None
                }
            },
            (b'L', b'$') => match self.p.str_var(&lname) {
                Some(id) => Some(Op::LoadStr(id)),
                None => {
                    self.err(file, line, format!("SC_ErrorInCommand_varinvalid: string variable \"{name}\" not found"));
                    Some(Op::PushStr(String::new()))
                }
            },
            (b'S', b'$') => match self.p.str_var(&lname) {
                Some(id) => Some(Op::StoreStr(id)),
                None => {
                    self.err(file, line, format!("SC_ErrorInCommand_varinvalid: string variable \"{name}\" not found"));
                    None
                }
            },
            (b'C', _) => match self.consts.get(&lname) {
                Some(&v) => Some(Op::Const(v)),
                None => {
                    self.err(file, line, format!("SC_ErrorInCommand_constantinvalid: constant \"{name}\" not found"));
                    Some(Op::Push(0.0))
                }
            },
            (b'F', _) => match self.curves.get(&lname) {
                Some(&id) => Some(Op::Curve(id)),
                None => {
                    self.err(file, line, format!("SC_ErrorInCommand_functioninvalid: curve \"{name}\" not found"));
                    // A function takes its argument off the stack and gives its value back; one
                    // the constfiles do not define gives 0 like an empty curve. Pushing a 0 on
                    // top of the argument instead shifted the rest of the line: the O530
                    // Facelift's ZF-6AP-1300/1700 lack `antrieb_wandler_idleforce` and
                    // `wandler_idle_fadeout`, and `A (L.L.engine_n) (F.L.…) (L.L.velocity)
                    // (F.L.…) * max (S.L.M_Wheel)` stored max(engine_n·0, 0) = 0 instead of
                    // max(A, 0), so the bus never moved.
                    let id = match self.missing_curve {
                        Some(id) => id,
                        None => {
                            let id = self.p.curves.len() as u32;
                            self.p.curves.push(Curve { name: String::new(), points: Vec::new() });
                            self.missing_curve = Some(id);
                            id
                        }
                    };
                    Some(Op::Curve(id))
                }
            },
            (b'M', b'V') => Some(Op::Callback(self.p.intern(name))),
            (b'M', _) => Some(Op::Macro(u32::MAX)),
            (b'T', b'F') => Some(Op::SoundTriggerFile(self.p.intern(name))),
            (b'T', _) => Some(Op::SoundTrigger(self.p.intern(name))),
            _ => {
                self.err(file, line, format!("SC_ErrorInCommand: unknown access ({inner})"));
                None
            }
        }
    }

    /// One word of a script line. The original compares words with its operator names as
    /// they stand (case counts: `$CutEnd` and `Min` are not operators).
    fn compile_word(&mut self, w: &str, file: &Path, line: usize) -> Option<Op> {
        if let Some(name) = w.strip_prefix('$') {
            // A `$` word whose rest is not a string operator compiles to nothing, without a
            // message: the original tries its list and moves on to the next word. Mods carry
            // a lone `$` (the O530's odometer), `$=>` (its destination matrix), `$++` and
            // `$SetLengthM` (the Ahlheim C2's ATRON) and `$CutEnd` (the O530's ALMEX).
            let op = match name {
                "msg" => Op::StrMsg,
                "RemoveSpaces" => Op::StrRemoveSpaces,
                "d" => Op::StrDup,
                "=" => Op::StrEq,
                "<" => Op::StrLt,
                ">" => Op::StrGt,
                "<=" => Op::StrLe,
                ">=" => Op::StrGe,
                "+" => Op::StrConcat,
                "*" => Op::StrRepeat,
                "length" => Op::StrLength,
                "cutBegin" => Op::StrCutBegin,
                "cutEnd" => Op::StrCutEnd,
                "SetLengthR" => Op::StrSetLengthR,
                "SetLengthL" => Op::StrSetLengthL,
                "SetLengthC" => Op::StrSetLengthC,
                "IntToStr" => Op::StrIntToStr,
                "IntToStrEnh" => Op::StrIntToStrEnh,
                "StrToFloat" => Op::StrToFloat,
                "__DigitsFirst" => Op::StrDigitsFirst,
                _ => {
                    log::debug!("{}:{line}: \"{w}\" is no string operator; skipped", file.display());
                    return None;
                }
            };
            return Some(op);
        }
        // a number as Delphi reads one: digits, a point, an exponent - never "inf" or "nan",
        // and nothing beyond the range of a float
        let numeric = w.trim_start_matches(['-', '+']).starts_with(|c: char| c.is_ascii_digit() || c == '.');
        if let Some(v) = w.parse::<f32>().ok().filter(|v| numeric && v.is_finite()) {
            return Some(Op::Push(v));
        }
        let op = match w {
            "+" => Op::Add,
            "-" => Op::Sub,
            "*" => Op::Mul,
            "/" => Op::Div,
            "%" => Op::Mod,
            "=" => Op::Eq,
            "<" => Op::Lt,
            ">" => Op::Gt,
            "<=" => Op::Le,
            ">=" => Op::Ge,
            "&&" => Op::And,
            "||" => Op::Or,
            "!" => Op::Not,
            "/-/" => Op::Neg,
            "d" => Op::Dup,
            "pi" => Op::Pi,
            "random" => Op::Random,
            "%stackdump%" => Op::StackDump,
            "sin" => Op::Sin,
            "arcsin" => Op::ArcSin,
            "arctan" => Op::ArcTan,
            "min" => Op::Min,
            "max" => Op::Max,
            "exp" => Op::Exp,
            "sqrt" => Op::Sqrt,
            "sqr" => Op::Sqr,
            "sgn" => Op::Sgn,
            "abs" => Op::Abs,
            "trunc" => Op::Trunc,
            _ => {
                // registers: `l0`…`l9` load, `s0`…`s9` store (the original's register file
                // has ten cells; the Ahlheim O530's gearbox uses `l8` and `l9`)
                let b = w.as_bytes();
                if b.len() == 2 && (b[0] == b'l' || b[0] == b's') && b[1].is_ascii_digit() {
                    let d = b[1] - b'0';
                    return Some(if b[0] == b'l' { Op::LoadReg(d) } else { Op::StoreReg(d) });
                }
                self.err(file, line, format!("SC_ErrorInCommand: unknown token \"{w}\""));
                return None;
            }
        };
        Some(op)
    }
}

fn patch(op: &mut Op, target: u32) {
    match op {
        Op::JumpIfZero(t) | Op::Jump(t) => *t = target,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program_of(script: &str) -> Program {
        let dir = std::env::temp_dir().join(format!("omsi_gearbox_{}_{}", std::process::id(), script.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let osc = dir.join("g.osc");
        std::fs::write(&osc, script).unwrap();
        let p = compile(&CompileInput { builtin_vars: vec!["Clutch".into()], scripts: vec![osc], ..Default::default() });
        let _ = std::fs::remove_dir_all(&dir);
        p
    }

    #[test]
    fn a_manual_gearbox_is_told_from_an_automatic_with_gate_keys() {
        // the LiAZ KPP: gates, the first one asks for the clutch
        let kpp = program_of("{trigger:kw_s_1} (L.L.Clutch) 1 = {if} 1 (S.L.g) {endif} {end}\n{trigger:kw_s_2} 2 (S.L.g) {end}\n");
        assert!(kpp.manual_gearbox());
        // an automatic with gear-hold keys and a torque converter's clutch elsewhere
        let auto = program_of("{trigger:automatic_D} 1 (S.L.d) {end}\n{trigger:kw_s_1} 1 (S.L.hold) {end}\n{trigger:kw_s_2} 2 (S.L.hold) {end}\n{macro:conv} (L.L.Clutch) (S.L.c) {end}\n");
        assert!(!auto.manual_gearbox());
        // gates and no automatic at all
        let plain = program_of("{trigger:kw_s_1} 1 (S.L.g) {end}\n{trigger:kw_s_2} 2 (S.L.g) {end}\n");
        assert!(plain.manual_gearbox());
        // a stock automatic: no gates
        assert!(!program_of("{trigger:automatic_D} 1 (S.L.d) {end}\n").manual_gearbox());
    }

    #[test]
    fn tokens() {
        let t = tokenize(r#"(L.L.a) 0.5 > {if} "Krueger 7x6" (M.V.GetFontIndex) $+ 'comment (x)"#);
        assert_eq!(
            t,
            vec![
                Tok::Paren("L.L.a".into()),
                Tok::Word("0.5".into()),
                Tok::Word(">".into()),
                Tok::Brace("if".into()),
                Tok::Str("Krueger 7x6".into()),
                Tok::Paren("M.V.GetFontIndex".into()),
                Tok::Word("$+".into()),
            ]
        );
        let t = tokenize("\"churafont CE (8-1)x3\" (M.V.GetFontIndex) (S.L.Font_(8-1)x3) (L.L.a)(L.L.b) ( L.L.c )");
        assert_eq!(
            t,
            vec![
                Tok::Str("churafont CE (8-1)x3".into()),
                Tok::Paren("M.V.GetFontIndex".into()),
                Tok::Paren("S.L.Font_(8-1)x3".into()),
                Tok::Paren("L.L.a".into()),
                Tok::Paren("L.L.b".into()),
                Tok::Paren("L.L.c".into()),
            ]
        );
    }
}
