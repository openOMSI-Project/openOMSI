//! JSON for [`Value`]s: the manifest, `json.encode`/`json.decode`, the plugins' storage and
//! the WASM boundary (see PLUGIN_SPEC section 1: `Callback` = `{"$cb": id}`, `Bytes` =
//! `{"$b64": "..."}`, a whole number as an `Int`).

use super::Value;
use std::fmt::Write as _;

/// Deepest nesting `decode` reads (a hostile text cannot run the stack out).
const MAX_DEPTH: usize = 128;

/// The value as compact JSON. A float that is no number (NaN, infinite) is `null`, as
/// JSON has nothing else for it.
pub fn encode(v: &Value) -> String {
    let mut out = String::new();
    write(&mut out, v, None, 0);
    out
}

/// The value as JSON indented by two spaces (the blessed files).
pub fn encode_pretty(v: &Value) -> String {
    let mut out = String::new();
    write(&mut out, v, Some(2), 0);
    out
}

fn write(out: &mut String, v: &Value, indent: Option<usize>, level: usize) {
    let nl = |out: &mut String, level: usize| {
        if let Some(n) = indent {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', n * level));
        }
    };
    match v {
        Value::Nil => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Num(n) => {
            if !n.is_finite() {
                out.push_str("null");
            } else if n.fract() == 0.0 && n.abs() < 1e15 {
                let _ = write!(out, "{}", *n as i64);
            } else {
                let _ = write!(out, "{n}");
            }
        }
        Value::Str(s) => string(out, s),
        Value::List(l) => {
            if l.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, x) in l.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, level + 1);
                write(out, x, indent, level + 1);
            }
            nl(out, level);
            out.push(']');
        }
        Value::Map(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, level + 1);
                string(out, k);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write(out, x, indent, level + 1);
            }
            nl(out, level);
            out.push('}');
        }
        Value::Callback(id) => {
            let _ = write!(out, "{{\"$cb\":{id}}}");
        }
        Value::Bytes(b) => {
            out.push_str("{\"$b64\":\"");
            out.push_str(&base64(b));
            out.push_str("\"}");
        }
    }
}

fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Read a JSON text. `{"$cb": n}` and `{"$b64": "..."}` read back as a callback and bytes
/// only when `special` is set (the WASM boundary); a plugin's `json.decode` keeps them maps.
pub fn decode(text: &str, special: bool) -> Result<Value, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0, special };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("text after the value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    special: bool,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        format!("JSON: {what} at byte {}", self.i)
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deep"));
        }
        match self.s.get(self.i) {
            None => Err(self.err("the text ends")),
            Some(b'n') if self.eat("null") => Ok(Value::Nil),
            Some(b't') if self.eat("true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Value::Bool(false)),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut l = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Ok(Value::List(l));
                }
                loop {
                    self.ws();
                    l.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Value::List(l));
                    }
                    return Err(self.err("',' or ']' expected"));
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut m: Vec<(String, Value)> = Vec::new();
                self.ws();
                if !self.eat("}") {
                    loop {
                        self.ws();
                        if self.s.get(self.i) != Some(&b'"') {
                            return Err(self.err("a key expected"));
                        }
                        let k = self.string()?;
                        self.ws();
                        if !self.eat(":") {
                            return Err(self.err("':' expected"));
                        }
                        self.ws();
                        let v = self.value(depth + 1)?;
                        // (a key written twice: the last one counts, as everywhere)
                        if let Some(slot) = m.iter_mut().find(|(key, _)| *key == k) {
                            slot.1 = v;
                        } else {
                            m.push((k, v));
                        }
                        self.ws();
                        if self.eat(",") {
                            continue;
                        }
                        if self.eat("}") {
                            break;
                        }
                        return Err(self.err("',' or '}' expected"));
                    }
                }
                if self.special && m.len() == 1 {
                    match (&m[0].0[..], &m[0].1) {
                        ("$cb", v) => {
                            if let Some(id) = v.as_i64().filter(|i| *i >= 0) {
                                return Ok(Value::Callback(id as u64));
                            }
                        }
                        ("$b64", Value::Str(s)) => {
                            return unbase64(s).map(Value::Bytes).ok_or_else(|| self.err("bad base64"));
                        }
                        _ => {}
                    }
                }
                Ok(Value::Map(m))
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        let t = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        let n: f64 = t.parse().map_err(|_| self.err("bad number"))?;
        if !t.contains(['.', 'e', 'E']) && n.abs() < 9_007_199_254_740_992.0 {
            if let Ok(i) = t.parse::<i64>() {
                return Ok(Value::Int(i));
            }
        }
        Ok(Value::Num(n))
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return Err(self.err("the text ends in a string")) };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return Err(self.err("the text ends in a string")) };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            char::from_u32(cp).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.err("bad escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("no UTF-8"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.s.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u32::from_str_radix(h, 16).ok());
        self.i += 4;
        h.ok_or_else(|| self.err("bad \\u escape"))
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                out.push(B64[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_order_and_kinds() {
        let v = Value::Map(vec![
            ("z".into(), Value::Int(3)),
            ("a".into(), Value::List(vec![Value::Num(1.5), Value::Nil, Value::Bool(true), Value::Str("é\"\n".into())])),
            ("cb".into(), Value::Callback(7)),
            ("raw".into(), Value::Bytes(vec![0, 1, 2, 250])),
        ]);
        let text = encode(&v);
        assert_eq!(text, r#"{"z":3,"a":[1.5,null,true,"é\"\n"],"cb":{"$cb":7},"raw":{"$b64":"AAEC+g=="}}"#);
        assert_eq!(decode(&text, true).unwrap(), v);
        // (a plugin's own JSON keeps such maps as maps)
        assert!(matches!(decode(&text, false).unwrap().get("cb"), Some(Value::Map(_))));
        assert_eq!(decode(&encode_pretty(&v), true).unwrap(), v);
    }

    #[test]
    fn numbers_and_mistakes() {
        assert_eq!(decode("12", false).unwrap(), Value::Int(12));
        assert_eq!(decode("12.0", false).unwrap(), Value::Num(12.0));
        assert_eq!(decode("-3e2", false).unwrap(), Value::Num(-300.0));
        assert_eq!(encode(&Value::Num(12.0)), "12");
        assert_eq!(encode(&Value::Num(f64::NAN)), "null");
        assert_eq!(decode(r#""😀""#, false).unwrap(), Value::Str("😀".into()));
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "tru", "\"abc", "1 2"] {
            assert!(decode(bad, false).is_err(), "{bad}");
        }
        assert!(decode(&"[".repeat(500), false).unwrap_err().contains("deep"));
    }

    #[test]
    fn base64_both_ways() {
        for b in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(unbase64(&base64(b)).unwrap(), b);
        }
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
