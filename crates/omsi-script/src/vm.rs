//! The virtual machine.

use crate::{compile::Program, BlockId, NameId, Op, SysVar};

/// The 8-slot float stack, the register file (ten cells, `l0`…`l9` / `s0`…`s9`, like the
/// original's `TCache`) and the 8-slot string stack.
#[derive(Debug, Clone)]
pub struct Stacks {
    pub st: [f32; 8],
    pub reg: [f32; 10],
    pub sst: [String; 8],
}

impl Default for Stacks {
    fn default() -> Self {
        Self { st: [0.0; 8], reg: [0.0; 10], sst: Default::default() }
    }
}

impl Stacks {
    #[inline]
    pub fn push(&mut self, v: f32) {
        self.st.copy_within(0..7, 1);
        self.st[0] = v;
    }
    /// Remove and return `st[0]`; the stack shifts up and `st[7]` becomes 0 (an empty slot
    /// reads as 0, so underflow is harmless like in the original).
    #[inline]
    pub fn pop(&mut self) -> f32 {
        let v = self.st[0];
        self.st.copy_within(1..8, 0);
        self.st[7] = 0.0;
        v
    }
    #[inline]
    pub fn push_str(&mut self, s: String) {
        for i in (1..8).rev() {
            self.sst[i] = std::mem::take(&mut self.sst[i - 1]);
        }
        self.sst[0] = s;
    }
    #[inline]
    pub fn pop_str(&mut self) -> String {
        let v = std::mem::take(&mut self.sst[0]);
        for i in 0..7 {
            self.sst[i] = std::mem::take(&mut self.sst[i + 1]);
        }
        v
    }
    /// Empty stacks, as every block of a script starts with (the strings keep their
    /// buffers).
    pub fn clear(&mut self) {
        self.st = [0.0; 8];
        self.reg = [0.0; 10];
        for s in &mut self.sst {
            s.clear();
        }
    }

    #[inline]
    pub fn top(&self) -> f32 {
        self.st[0]
    }
    #[inline]
    pub fn top_str(&self) -> &str {
        &self.sst[0]
    }
}

/// Per-instance variable storage.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub vars: Vec<f32>,
    pub str_vars: Vec<String>,
}

impl State {
    pub fn new(p: &Program) -> Self {
        Self { vars: vec![0.0; p.var_names.len()], str_vars: vec![String::new(); p.str_var_names.len()] }
    }
    #[inline]
    pub fn get(&self, id: crate::VarId) -> f32 {
        self.vars[id as usize]
    }
    #[inline]
    pub fn set(&mut self, id: crate::VarId, v: f32) {
        self.vars[id as usize] = v;
    }
}

/// What the object's environment provides to running scripts.
pub trait Host {
    fn sys_var(&mut self, v: SysVar) -> f32;
    fn set_sys_var(&mut self, _v: SysVar, _value: f32) {}

    /// `(M.V.name)`. Arguments are on the stacks; results are pushed by the callback.
    fn callback(&mut self, name: &str, id: NameId, stacks: &mut Stacks, state: &mut State);
    fn sound_trigger(&mut self, name: &str, id: NameId);
    /// `(T.L.name)` with the variables as they stand at that point of the script: Omsi.exe
    /// starts the sounds of a trigger right there (0x74f2e8 runs their update at once), so
    /// their volume curves read the values of that moment, not of the frame's end.
    fn sound_trigger_vars(&mut self, name: &str, id: NameId, _vars: &[f32]) {
        self.sound_trigger(name, id)
    }
    /// `(T.F.name)`: trigger `name` with a sound file chosen by the script.
    fn sound_trigger_file(&mut self, _name: &str, _file: &str) {}
    fn message(&mut self, text: &str) {
        log::info!("$msg: {text}");
    }
    fn stack_dump(&mut self, stacks: &Stacks) {
        log::info!("Stack Dump: {:?} regs {:?}", stacks.st, stacks.reg);
    }
}

/// A host that provides nothing; useful for tests and offline validation.
#[derive(Default)]
pub struct NullHost;

impl Host for NullHost {
    fn sys_var(&mut self, _v: SysVar) -> f32 {
        0.0
    }
    fn callback(&mut self, _name: &str, _id: NameId, _stacks: &mut Stacks, _state: &mut State) {}
    fn sound_trigger(&mut self, _name: &str, _id: NameId) {}
}

/// Executes blocks of a [`Program`].
#[derive(Clone)]
pub struct Vm {
    pub stacks: Stacks,
    rng: u64,
    /// Macro recursion guard.
    depth: u32,
}

/// The state every `random` starts from while no session seed is set (tests and tools
/// get the same numbers on every run).
const FIXED_RNG: u64 = 0x9E37_79B9_7F4A_7C15;
/// The session's seed (0 = none, see [`set_session_seed`]) and how many machines drew
/// their own seed from it so far.
static SESSION_SEED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SEEDED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Seed the `random` of every machine created from now on: each one gets its own stream
/// from `seed` and its creation number, as OMSI seeds its random generator once per
/// session. Without it every vehicle drew the same numbers in every session - the stock
/// buses always started with the same (low) air pressure from bremse_init's 4-9 bar.
pub fn set_session_seed(seed: u64) {
    SESSION_SEED.store(seed.max(1), std::sync::atomic::Ordering::Relaxed);
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Default for Vm {
    fn default() -> Self {
        let session = SESSION_SEED.load(std::sync::atomic::Ordering::Relaxed);
        let rng = if session == 0 { FIXED_RNG } else { splitmix64(session ^ splitmix64(SEEDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed))) | 1 };
        Self { stacks: Stacks::default(), rng, depth: 0 }
    }
}

const MAX_DEPTH: u32 = 64;

impl Vm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&mut self, seed: u64) {
        self.rng = seed | 1;
    }

    #[inline]
    fn rand01(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        (r as f32) / ((1u64 << 24) as f32)
    }

    /// Run the `{init}` blocks.
    pub fn run_init(&mut self, p: &Program, state: &mut State, host: &mut dyn Host) {
        for &b in &p.init {
            self.run_top(p, b, state, host);
        }
    }

    /// Run the `{frame}` blocks.
    pub fn run_frame(&mut self, p: &Program, state: &mut State, host: &mut dyn Host) {
        for &b in &p.frame {
            self.run_top(p, b, state, host);
        }
    }

    /// Run the `{frame_ai}` blocks (falls back to nothing when absent).
    pub fn run_frame_ai(&mut self, p: &Program, state: &mut State, host: &mut dyn Host) {
        for &b in &p.frame_ai {
            self.run_top(p, b, state, host);
        }
    }

    /// Fire a `{trigger:name}` block. Returns false when the trigger does not exist.
    pub fn run_trigger(&mut self, p: &Program, name: &str, state: &mut State, host: &mut dyn Host) -> bool {
        match p.trigger(name) {
            Some(b) => {
                // OMSI passes the key state to a trigger: 1 on press and 0 on release.
                // Scripts guard the press body with a bare `{if}`; keeping that value on
                // the stack also lets release blocks such as `kw_m_enginestart_off` store
                // the released state in their anti-repeat variable.
                self.stacks.clear();
                let value = if name.to_ascii_lowercase().ends_with("_off") { 0.0 } else { 1.0 };
                self.stacks.push(value);
                self.run_block(p, b, state, host);
                true
            }
            None => false,
        }
    }

    /// Run a block the engine calls (not a macro) on empty stacks. The stock bus stop
    /// display builds its text with `$+` onto whatever lies below on the string stack and
    /// leaves the result there: kept from frame to frame, every frame's text was appended to
    /// the last one's.
    fn run_top(&mut self, p: &Program, block: BlockId, state: &mut State, host: &mut dyn Host) {
        self.stacks.clear();
        self.run_block(p, block, state, host);
    }

    pub fn run_block(&mut self, p: &Program, block: BlockId, state: &mut State, host: &mut dyn Host) {
        if self.depth >= MAX_DEPTH {
            log::warn!("script macro recursion too deep in {}", p.blocks[block as usize].name);
            return;
        }
        self.depth += 1;
        let ops = &p.blocks[block as usize].ops;
        let mut pc = 0usize;
        while pc < ops.len() {
            let op = &ops[pc];
            pc += 1;
            match op {
                Op::Macro(b) => {
                    let b = *b;
                    self.run_block(p, b, state, host);
                }
                Op::Random => {
                    // `Random(Abs(Round(x)))` in the exe: a whole number in [0, n). Scripts
                    // compare it (`3 random 1 =`), which a fraction never matches.
                    let r = self.rand01();
                    let s = &mut self.stacks;
                    let n = s.pop().round_ties_even().abs();
                    s.push((r * n).floor().min((n - 1.0).max(0.0)));
                }
                Op::JumpIfZero(t) => {
                    // {if} only looks at its condition; the value stays on the stack, and
                    // the stock scripts build on that: `(L.L.bremse_feststell) {if} !
                    // (S.L.bremse_feststell)` releases the SD200's parking brake with the
                    // condition itself, `cond {if} (L.L.IBIS_busstop) 0 > && {if}` (IBIS
                    // back key) and the NL202's ramp request combine it with the next test,
                    // and the chura matrix writes `x d -1 = ! {if} * (S.L.y)`. Popping it
                    // made all of these work on whatever lay below.
                    if self.stacks.top() == 0.0 {
                        pc = *t as usize;
                    }
                }
                Op::Jump(t) => pc = *t as usize,
                other => exec_op(&mut self.stacks, other, p, state, host),
            }
        }
        self.depth -= 1;
    }
}

/// Execute one instruction that only touches the stacks, the state and the host.
///
/// Values push; binary operators pop both operands and push the result; unary operators
/// replace `st[0]`. This is what the stock scripts require (`A B C || &&` evaluates
/// `A && (B || C)`).
#[inline(always)]
fn exec_op(s: &mut Stacks, op: &Op, p: &Program, state: &mut State, host: &mut dyn Host) {
    if !exec_hot(s, op, p, state, host) {
        exec_cold(s, op, p, state, host)
    }
}

#[inline(always)]
fn exec_hot(s: &mut Stacks, op: &Op, p: &Program, state: &mut State, host: &mut dyn Host) -> bool {
    macro_rules! bin {
        ($f:expr) => {{
            let a = s.pop();
            let b = s.pop();
            s.push($f(b, a));
        }};
    }
    macro_rules! un {
        ($f:expr) => {{
            let a = s.pop();
            s.push($f(a));
        }};
    }
    match op {
        Op::Push(v) => s.push(*v),
        Op::Load(id) => s.push(state.vars[*id as usize]),
        Op::Store(id) => {
            if !s.st[0].is_finite() && debug_nan() {
                log::warn!("script: {} = {} (the first number that is not one)", p.var_name(*id).unwrap_or("?"), s.st[0]);
            }
            state.vars[*id as usize] = s.st[0]
        }
        Op::LoadSys(v) => {
            let x = host.sys_var(*v);
            s.push(x)
        }
        Op::StoreSys(v) => host.set_sys_var(*v, s.st[0]),
        Op::Const(v) => s.push(*v),
        Op::Curve(id) => {
            let c = &p.curves[*id as usize];
            un!(|x| c.eval(x))
        }
        Op::LoadReg(i) => s.push(s.reg[*i as usize]),
        Op::StoreReg(i) => s.reg[*i as usize] = s.st[0],
        Op::Add => bin!(|b: f32, a: f32| b + a),
        Op::Sub => bin!(|b: f32, a: f32| b - a),
        Op::Mul => bin!(|b: f32, a: f32| b * a),
        Op::Div => {
            let a = s.pop();
            let b = s.pop();
            // division by zero gives 0 (the exe compares the divisor with 0.0 and stores 0).
            // Clearing the whole stack instead killed the LiAZ 5292: its battery charges by
            // `Timegap V_generator /`, which is 0 with the engine off, and the zeroed stack
            // wrote a flat battery every frame, so the bus never had electrics.
            s.push(if a == 0.0 { 0.0 } else { b / a })
        }
        Op::Mod => bin!(|b: f32, a: f32| if a == 0.0 { 0.0 } else { b % a }),
        Op::Eq => bin!(|b, a| b2f(b == a)),
        Op::Lt => bin!(|b, a| b2f(b < a)),
        Op::Gt => bin!(|b, a| b2f(b > a)),
        Op::Le => bin!(|b, a| b2f(b <= a)),
        Op::Ge => bin!(|b, a| b2f(b >= a)),
        Op::And => bin!(|b: f32, a: f32| b2f(b != 0.0 && a != 0.0)),
        Op::Or => bin!(|b: f32, a: f32| b2f(b != 0.0 || a != 0.0)),
        Op::Min => bin!(|b: f32, a: f32| b.min(a)),
        Op::Max => bin!(|b: f32, a: f32| b.max(a)),
        Op::Not => un!(|a: f32| b2f(a == 0.0)),
        Op::Neg => un!(|a: f32| -a),
        Op::Dup => s.push(s.st[0]),
        Op::Sin => un!(|a: f32| a.sin()),
        Op::ArcSin => un!(|a: f32| a.clamp(-1.0, 1.0).asin()),
        Op::ArcTan => un!(|a: f32| a.atan()),
        Op::Exp => un!(|a: f32| a.exp()),
        Op::Sqrt => un!(|a: f32| if a > 0.0 { a.sqrt() } else { 0.0 }),
        Op::Sqr => un!(|a: f32| a * a),
        Op::Sgn => un!(|a: f32| if a > 0.0 { 1.0 } else if a < 0.0 { -1.0 } else { 0.0 }),
        Op::Abs => un!(|a: f32| a.abs()),
        Op::Trunc => un!(|a: f32| a.trunc()),
        Op::Pi => s.push(std::f32::consts::PI),
        _ => return false,
    }
    true
}

#[inline(never)]
fn exec_cold(s: &mut Stacks, op: &Op, p: &Program, state: &mut State, host: &mut dyn Host) {
    match op {
        Op::Macro(_) | Op::Random | Op::JumpIfZero(_) | Op::Jump(_) => unreachable!("handled by run_block"),
        Op::Callback(n) => host.callback(p.name(*n), *n, s, state),
        Op::SoundTrigger(n) => host.sound_trigger_vars(p.name(*n), *n, &state.vars),
        Op::SoundTriggerFile(n) => {
            let file = s.pop_str();
            host.sound_trigger_file(p.name(*n), &file);
        }
        Op::LoadStr(id) => {
            let v = state.str_vars[*id as usize].clone();
            s.push_str(v)
        }
        Op::StoreStr(id) => state.str_vars[*id as usize] = s.sst[0].clone(),
        Op::PushStr(i) => s.push_str(p.strings[*i as usize].clone()),
        Op::StackDump => host.stack_dump(s),
        Op::StrConcat => {
            let a = s.pop_str();
            let b = s.pop_str();
            s.push_str(format!("{b}{a}"))
        }
        Op::StrEq => {
            let a = s.pop_str();
            let b = s.pop_str();
            s.push(b2f(b == a))
        }
        Op::StrLt => {
            let a = s.pop_str();
            let b = s.pop_str();
            s.push(b2f(b < a))
        }
        Op::StrGt => {
            let a = s.pop_str();
            let b = s.pop_str();
            s.push(b2f(b > a))
        }
        Op::StrLe => {
            let a = s.pop_str();
            let b = s.pop_str();
            s.push(b2f(b <= a))
        }
        Op::StrGe => {
            let a = s.pop_str();
            let b = s.pop_str();
            s.push(b2f(b >= a))
        }
        Op::StrDup => {
            let v = s.sst[0].clone();
            s.push_str(v)
        }
        Op::StrRepeat => {
            // Omsi.exe (op 0x21): the pattern repeated to fill `Round(n)` characters, cut
            // there ("ab" 3 `$*` is "aba"); 1000 or more gives "ERROR"
            let n = s.pop();
            let pat = s.pop_str();
            let v = if n >= 1000.0 {
                "ERROR".to_string()
            } else {
                pat.chars().cycle().take(if pat.is_empty() { 0 } else { omsi_round(n) }).collect()
            };
            s.push_str(v)
        }
        Op::StrLength => {
            // reads the top string and leaves it there: the chura/Krüger matrix measures
            // and stores a string and then removes it itself with `0 $* $+`
            // (`(S.$.t) $length (S.L.n) 0 $* $+`); popping it here as well ate the string
            // below, and every destination text of that display came out empty
            let n = s.top_str().chars().count();
            s.push(n as f32)
        }
        Op::StrMsg => {
            // shows the top string and leaves it on the stack (IBIS-2 builds the
            // announcement path across a `$msg`)
            let v = s.top_str().to_string();
            host.message(&v)
        }
        Op::StrRemoveSpaces => {
            // Omsi.exe 0x7ef304: tabs, line breaks, spaces and quotes off both ends only -
            // a Krueger++ bitmap "206 to jkkyz.bmp" lost its inner spaces and was not found
            let v = s
                .pop_str()
                .trim_matches(|c| matches!(c, '\t' | '\n' | '\r' | ' ' | '"'))
                .to_string();
            s.push_str(v)
        }
        Op::StrCutBegin => {
            let n = omsi_round(s.pop());
            let v: String = s.pop_str().chars().skip(n).collect();
            s.push_str(v)
        }
        Op::StrCutEnd => {
            let n = omsi_round(s.pop());
            let src = s.pop_str();
            let len = src.chars().count();
            let v: String = src.chars().take(len.saturating_sub(n)).collect();
            s.push_str(v)
        }
        Op::StrSetLengthL => {
            let n = omsi_round(s.pop());
            let v = set_length(&s.pop_str(), n, Align::Left);
            s.push_str(v)
        }
        Op::StrSetLengthR => {
            let n = omsi_round(s.pop());
            let v = set_length(&s.pop_str(), n, Align::Right);
            s.push_str(v)
        }
        Op::StrSetLengthC => {
            let n = omsi_round(s.pop());
            let v = set_length(&s.pop_str(), n, Align::Center);
            s.push_str(v)
        }
        Op::StrIntToStr => {
            let v = (s.pop().trunc() as i64).to_string();
            s.push_str(v)
        }
        Op::StrIntToStrEnh => {
            let fmt = s.pop_str();
            let v = int_to_str_enh(s.pop(), &fmt);
            s.push_str(v)
        }
        Op::StrDigitsFirst => {
            let v = digits_first(&s.pop_str());
            s.push_str(v)
        }
        Op::StrToFloat => {
            let v = str_to_float(&s.pop_str());
            s.push(v)
        }

        // (every numeric op is exec_hot's: this cannot be reached unless the program's
        // memory was damaged - it was, once, on an Exynos phone, and the panic ended the
        // game, #1812; a script that does a step less beats a game that stops)
        other => {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::error!("script: instruction {other:?} reached the string/host dispatch; skipped");
            }
        }
    }
}

/// `OMSI_DEBUG_NAN=1`: warn at every write of a NaN or an infinity into a script variable
/// (a model going wrong shows up as NaN far from where it started).
fn debug_nan() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| omsi_cfg::flags::OMSI_DEBUG_NAN.live_os().is_some())
}

/// `$StrToFloat`: a number, or -1 when the text is none. Scripts test for that: the chura
/// matrix takes `"*GOTO[12345]"` apart with `6 $cutBegin 1 $cutEnd $StrToFloat 0 >=` and
/// asks whether a line number ends in a letter with `1 $SetLengthR $StrToFloat -1 =`.
/// Reading "NORDSPITZE" as 0 sent every destination through the GOTO branch, which
/// swallowed the text.
fn str_to_float(s: &str) -> f32 {
    let t = s.trim();
    if t.is_empty() || !t.bytes().any(|b| b.is_ascii_digit()) || !t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E')) {
        return -1.0;
    }
    t.parse::<f32>().unwrap_or(-1.0)
}

#[inline]
fn b2f(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

/// Delphi's `Round` of a string function's count (ties to even, as the FPU rounds), none
/// below 0: Omsi.exe rounds the counts of `$cutBegin`, `$cutEnd`, `$SetLength*` and `$*`.
fn omsi_round(v: f32) -> usize {
    let r = (v as f64).round_ties_even();
    if r.is_finite() && r > 0.0 { r.min(1e9) as usize } else { 0 }
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Right,
    Center,
}

fn set_length(s: &str, n: usize, align: Align) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= n {
        return match align {
            Align::Left => chars[..n].iter().collect(),
            Align::Right => chars[chars.len() - n..].iter().collect(),
            Align::Center => {
                let start = (chars.len() - n) / 2;
                chars[start..start + n].iter().collect()
            }
        };
    }
    let pad = n - chars.len();
    match align {
        Align::Left => format!("{s}{}", " ".repeat(pad)),
        Align::Right => format!("{}{s}", " ".repeat(pad)),
        Align::Center => {
            let l = pad / 2;
            format!("{}{s}{}", " ".repeat(l), " ".repeat(pad - l))
        }
    }
}

/// `$IntToStrEnh` as OMSI does it (op 0x1f at 0x5d59d5): the format's first character
/// pads, the rest is the width. A format of fewer than two characters gives "ERROR", width 0
/// an empty text; the truncated number too long for the width is cut to width − 1
/// characters and a `#`; the padding always goes in front, before a minus sign too
/// (-5 in "04" is "00-5").
fn int_to_str_enh(v: f32, fmt: &str) -> String {
    let mut chars = fmt.chars();
    let (Some(pad), rest) = (chars.next(), chars.as_str()) else { return "ERROR".into() };
    if rest.is_empty() {
        return "ERROR".into();
    }
    let width = rest.trim().parse::<i64>().unwrap_or(0).max(0) as usize;
    if width == 0 {
        return String::new();
    }
    let r = (v.trunc() as i64).to_string();
    if width < r.len() {
        return format!("{}#", &r[..width - 1]);
    }
    let mut out: String = std::iter::repeat_n(pad, width - r.len()).collect();
    out.push_str(&r);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{compile, CompileInput};

    fn prog(src: &str) -> Program {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("omsi_script_test_{}_{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("t.osc");
        std::fs::write(&script, src).unwrap();
        let vl = dir.join("v.txt");
        std::fs::write(&vl, "a\nb\nresult\n").unwrap();
        let cf = dir.join("c.txt");
        std::fs::write(&cf, "[const]\nk\n2.5\n[newcurve]\ncv\n[pnt]\n0\n0\n[pnt]\n10\n100\n").unwrap();
        compile(&CompileInput { varlists: vec![vl], constfiles: vec![cf], scripts: vec![script], ..Default::default() })
    }

    #[test]
    fn session_seed_gives_every_machine_its_own_numbers() {
        let p = prog("{init}\n500000 random 400000 + (S.L.result)\n{end}\n");
        let draw = || {
            let mut vm = Vm::new();
            let mut st = State::new(&p);
            vm.run_init(&p, &mut st, &mut NullHost);
            st.get(p.var("result").unwrap())
        };
        // (the only test that sets the seed, and none of the others draws a number)
        super::set_session_seed(12345);
        let a: Vec<f32> = (0..16).map(|_| draw()).collect();
        assert!(a.iter().all(|v| (400000.0..900000.0).contains(v)), "{a:?}");
        let distinct = a.iter().map(|v| *v as i64).collect::<std::collections::BTreeSet<_>>().len();
        assert!(distinct >= 15, "{a:?}");
        assert!(a.iter().any(|v| *v > 650000.0) && a.iter().any(|v| *v < 650000.0), "{a:?}");
    }

    #[test]
    fn stack_semantics() {
        let p = prog("{macro:m}\n1 4 +\n{end}\n{init}\n(M.L.m) (S.L.result)\n{end}\n");
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(vm.stacks.st[..3], [5.0, 0.0, 0.0]);
        assert_eq!(st.get(p.var("result").unwrap()), 5.0);
    }

    #[test]
    fn unknown_curve_takes_its_argument() {
        // the O530 Facelift's converter line with two curves its constfile lacks
        let p = prog("{init}\n7 (S.L.b) 3 (L.L.b) (F.L.nocurve) (L.L.b) (F.L.cv) * max (S.L.result) (S.L.a)\n(L.L.b) (F.L.cv) (S.L.b)\n{end}\n");
        assert_eq!(p.errors.len(), 1, "{:?}", p.errors);
        assert!(p.errors[0].message.contains("functioninvalid"));
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("result").unwrap()), 3.0);
        assert_eq!(st.get(p.var("a").unwrap()), 3.0);
        assert_eq!(st.get(p.var("b").unwrap()), 70.0);
    }

    #[test]
    fn control_flow_and_strings() {
        let p = prog(
            "{init}\n(C.L.k) 2 > {if} 1 {else} 0 {endif} (S.L.a)\n5 (F.L.cv) (S.L.b)\n\"ab\" \"cd\" $+ $length (S.L.result)\n7 \"03\" $IntToStrEnh $StrToFloat\n{end}\n",
        );
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("a").unwrap()), 1.0);
        assert_eq!(st.get(p.var("b").unwrap()), 50.0);
        assert_eq!(st.get(p.var("result").unwrap()), 4.0);
        assert_eq!(vm.stacks.st[0], 7.0);
    }

    #[test]
    fn nested_logic() {
        // A && (B || C) with A false must be false
        let p = prog("{init}\n0 1 1 || && (S.L.a)\n1 0 0 || && (S.L.b)\n{end}\n");
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("a").unwrap()), 0.0);
        assert_eq!(st.get(p.var("b").unwrap()), 0.0);
    }

    #[test]
    fn frac_idiom() {
        // (L.S.GetTime) d trunc -  → fractional part
        let p = prog("{init}\n3.75 d trunc - (S.L.a)\n{end}\n");
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert!((st.get(p.var("a").unwrap()) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn if_keeps_its_condition() {
        // the SD200's parking brake release and the chura matrix's `{if} *`
        let p = prog("{init}\n1 (S.L.a) 7 (L.L.a) {if} ! (S.L.a) {endif}\n5 d -1 = ! {if} * (S.L.b) {endif}\n3 (S.L.result) 0 {if} 9 (S.L.result) {else} (S.L.result) {endif}\n{end}\n");
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("a").unwrap()), 0.0);
        assert_eq!(st.get(p.var("b").unwrap()), 5.0);
        // the else branch sees the (false) condition on top
        assert_eq!(st.get(p.var("result").unwrap()), 0.0);
    }

    #[test]
    fn length_leaves_the_string() {
        // the chura matrix's delimiter idiom: store and measure the two strings on top,
        // then drop them with `0 $* $+`, leaving the text below untouched
        let src = "{init}\n\"NORDSPITZE\" \"{{\" \"*FLIP}}\" (S.L.a) $length (S.L.b) 0 $* $+ $length (S.L.result) 0 $* $+ $length\n{end}\n";
        let p = prog(src);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("b").unwrap()), 7.0);
        assert_eq!(st.get(p.var("result").unwrap()), 2.0);
        assert_eq!(vm.stacks.top_str(), "NORDSPITZE");
        assert_eq!(vm.stacks.top(), 10.0);
    }

    #[test]
    fn triggers_that_set_a_variable() {
        let p = prog("{trigger:on}\n1 (S.L.a)\n{end}\n{trigger:off}\n0 (S.L.b) (S.L.a)\n{end}\n{macro:m}\n(L.L.a) ! (S.L.a)\n{end}\n{trigger:tog}\n(M.L.m)\n{end}\n{trigger:ai_x}\n1 (S.L.a)\n{end}\n{trigger:other}\n1 (S.L.b)\n{end}\n");
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(p.triggers_setting("a"), vec!["on".to_string(), "tog".to_string()]);
        assert_eq!(p.triggers_setting("b"), vec!["other".to_string()]);
        assert!(p.triggers_setting("nonexistent").is_empty());
        let p = prog("{init}\n\"f1\" (M.V.GetFontIndex) (S.L.a) \"x\" \"f2\" (M.V.GetFontIndex) (S.L.b)\n{end}\n");
        assert_eq!(p.literal_arguments("getfontindex"), vec!["f1".to_string(), "f2".to_string()]);
        assert_eq!(p.callbacks_used(), vec!["GetFontIndex".to_string()]);
    }

    #[test]
    fn release_triggers_receive_zero_state() {
        let mut p = Program::default();
        let a = p.declare_var("a");
        p.blocks.push(crate::compile::Block {
            ops: vec![Op::Store(a)],
            ..Default::default()
        });
        p.blocks.push(crate::compile::Block {
            ops: vec![Op::Store(a)],
            ..Default::default()
        });
        p.triggers.insert("start".into(), 0);
        p.triggers.insert("start_off".into(), 1);
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_trigger(&p, "start", &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("a").unwrap()), 1.0);
        vm.run_trigger(&p, "start_off", &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("a").unwrap()), 0.0);
    }

    /// `$` words that are no string operator compile to nothing and leave both stacks alone,
    /// silently: the O530's odometer (`" " $ (S.$.x)`), its matrix (`$=>`), the Ahlheim C2's
    /// `$++` and `30 $SetLengthM`, and `$CutEnd` (the operator is `$cutEnd`).
    #[test]
    fn unknown_string_words_are_skipped() {
        let src = "{init}\n\"km \" \"12\" $ (S.$.s)\n\"a\" \"b\" $=> $++ 30 $SetLengthM (S.L.a)\n\"12\" 1 $CutEnd $StrToFloat (S.L.b)\n\"12\" 1 $cutEnd $StrToFloat (S.L.result)\n{end}\n";
        let dir = std::env::temp_dir().join(format!("omsi_script_words_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.osc"), src).unwrap();
        std::fs::write(dir.join("v.txt"), "a\nb\nresult\n").unwrap();
        std::fs::write(dir.join("s.txt"), "s\n").unwrap();
        let p = compile(&CompileInput { varlists: vec![dir.join("v.txt")], stringvarlists: vec![dir.join("s.txt")], scripts: vec![dir.join("t.osc")], ..Default::default() });
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.str_vars[p.str_var("s").unwrap() as usize], "12");
        assert_eq!(st.get(p.var("a").unwrap()), 30.0);
        // `$CutEnd` did nothing: the whole "12" is read, and the 1 is still on the stack
        assert_eq!(st.get(p.var("b").unwrap()), 12.0);
        assert_eq!(st.get(p.var("result").unwrap()), 1.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Operator names count as spelled; ten registers.
    #[test]
    fn exact_words_and_ten_registers() {
        let p = prog("{init}\n8 s8 s9 pop 2 l8 l9 * (S.L.a)\n{end}\n");
        assert_eq!(p.errors.len(), 1, "{:?}", p.errors); // `pop` is no operator
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("a").unwrap()), 64.0);
        assert_eq!(vm.stacks.reg[9], 8.0);
        let p = prog("{init}\n1 2 Min 4 SIN L0 (S.L.a)\n{end}\n");
        assert_eq!(p.errors.len(), 3, "{:?}", p.errors);
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(p.var("a").unwrap()), 4.0);
    }

    #[test]
    fn blocks_start_on_empty_stacks() {
        // the stock bus stop display's pattern: concatenate onto the empty bottom of the
        // string stack and leave the result there
        let p = prog("{frame}\n\"x\" \"\" $= {if} {endif} \"ab\" $+ \"@\" $+ $+\n{end}\n");
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        for _ in 0..3 {
            vm.run_frame(&p, &mut st, &mut NullHost);
        }
        assert_eq!(vm.stacks.top_str(), "ab@");
    }

    #[test]
    fn fmt() {
        assert_eq!(int_to_str_enh(7.0, "02"), "07");
        assert_eq!(int_to_str_enh(7.0, " 2"), " 7");
        assert_eq!(int_to_str_enh(123.0, "02"), "1#");
        assert_eq!(set_length("abc", 5, Align::Left), "abc  ");
        assert_eq!(set_length("abc", 5, Align::Right), "  abc");
        assert_eq!(set_length("abcdef", 3, Align::Left), "abc");
    }

    /// A stray `{endif}` closes an `{if}` early; the `{else}` after it skips to the next
    /// `{endif}` as OMSI's does, instead of running what follows it every time.
    #[test]
    fn orphan_else_skips_to_the_next_endif() {
        let p = prog("{init}\n1 (S.L.a)\n1 {if} 2 (S.L.a) {endif} {endif}\n{else} 3 (S.L.a) {endif}\n{end}\n");
        let mut st = State::new(&p);
        let mut vm = Vm::new();
        vm.run_init(&p, &mut st, &mut NullHost);
        assert_eq!(st.vars[p.var("a").unwrap() as usize], 2.0);
    }

    /// Aachen ibox / ticket-printer buttons (#744): the press trigger sets a flag, `_drag`
    /// and `_off` clear it, and the frame script is what acts. The frame must run while the
    /// flag is still set - openOMSI does that from `Player::click` via `update_scripts_only`.
    #[test]
    fn momentary_mouse_flag_needs_frame_before_drag_clear() {
        let dir = std::env::temp_dir().join(format!(
            "omsi_script_ibox_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("ibox.osc");
        std::fs::write(
            &script,
            "{trigger:ibox_taste_A1}\n\t1 (S.L.ibox_taste_A1)\n{end}\n\
             {trigger:ibox_taste_A1_drag}\n\t0 (S.L.ibox_taste_A1)\n{end}\n\
             {trigger:ibox_taste_A1_off}\n\t0 (S.L.ibox_taste_A1)\n{end}\n\
             {frame}\n\
             \t(L.L.ibox_taste_A1) 1 =\n\
             \t{if}\n\
             \t\t(L.L.hits) 1 + (S.L.hits)\n\
             \t{endif}\n\
             {end}\n",
        )
        .unwrap();
        let vl = dir.join("v.txt");
        std::fs::write(&vl, "ibox_taste_A1\nhits\n").unwrap();
        let p = compile(&CompileInput {
            varlists: vec![vl],
            scripts: vec![script],
            ..Default::default()
        });
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let taste = p.var("ibox_taste_A1").unwrap();
        let hits = p.var("hits").unwrap();

        // Wrong order (drag before frame): the flag is gone before the frame sees it.
        let mut vm = Vm::new();
        let mut st = State::new(&p);
        vm.run_trigger(&p, "ibox_taste_A1", &mut st, &mut NullHost);
        vm.run_trigger(&p, "ibox_taste_A1_drag", &mut st, &mut NullHost);
        vm.run_frame(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(taste), 0.0);
        assert_eq!(st.get(hits), 0.0, "drag before frame must lose the press");

        // Right order (frame on press, then drag): one hit, then the flag is cleared so a
        // held button does not repeat every frame.
        st.set(hits, 0.0);
        vm.run_trigger(&p, "ibox_taste_A1", &mut st, &mut NullHost);
        vm.run_frame(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(hits), 1.0, "frame on press must see the flag");
        vm.run_trigger(&p, "ibox_taste_A1_drag", &mut st, &mut NullHost);
        vm.run_frame(&p, &mut st, &mut NullHost);
        assert_eq!(st.get(hits), 1.0, "cleared flag must not hit again while held");
        assert_eq!(st.get(taste), 0.0);
    }

}

/// A line number as a display of three digit cells and a letter cell shows it: the digits
/// right-aligned and padded with zeros, the one letter behind them ("E 5 " -> "005E",
/// " E92" -> "092E", "52  " -> "052 "). Anything else (a symbol line, "BVG ", more than one
/// letter) is left as it is. See `compat::four_char_matrix`.
pub fn digits_first(text: &str) -> String {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    let others: Vec<char> = text.chars().filter(|c| !c.is_ascii_digit() && !c.is_whitespace()).collect();
    if digits.is_empty() || digits.len() > 3 || others.len() > 1 || others.iter().any(|c| !c.is_alphabetic()) {
        return text.to_string();
    }
    let letter = others.first().copied().unwrap_or(' ');
    format!("{:0>3}{}", digits, letter)
}

#[cfg(test)]
mod int_to_str_tests {
    #[test]
    fn as_omsi_pads_and_cuts() {
        use super::int_to_str_enh as f;
        assert_eq!(f(5.9, "04"), "0005");
        assert_eq!(f(-5.0, "04"), "00-5");
        assert_eq!(f(12345.0, " 3"), "12#");
        assert_eq!(f(7.0, " 0"), "");
        assert_eq!(f(7.0, "0"), "ERROR");
        assert_eq!(f(42.0, " 4"), "  42");
    }
}
