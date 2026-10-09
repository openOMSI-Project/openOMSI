//! [`Value`]: what goes into and comes out of a plugin function, whatever the plugin's
//! language. Lua tables, WASM JSON and the game's own data all meet here.

/// One value of the plugin API.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    List(Vec<Value>),
    /// Keys in order (a JSON object keeps the order it was written in).
    Map(Vec<(String, Value)>),
    /// A function of the plugin: a Lua function or a WASM callback id, called back through
    /// the plugin's [`super::Binding`].
    Callback(u64),
    Bytes(Vec<u8>),
}

impl Value {
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// The number, for `Int` and `Num` (and a text that reads as one, as Lua converts it).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Num(n) => Some(*n),
            Value::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// A whole number: an `Int`, or a `Num` without a fraction.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Some(*n as i64),
            Value::Str(s) => s.trim().parse::<i64>().ok().or_else(|| s.trim().parse::<f64>().ok().filter(|n| n.fract() == 0.0).map(|n| n as i64)),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Truthiness as Lua has it: everything but nil and false.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    /// A map's value by key (`None` for other kinds, or no such key).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The items of a list (a map or anything else has none).
    pub fn items(&self) -> &[Value] {
        match self {
            Value::List(l) => l,
            _ => &[],
        }
    }

    /// A table of either kind: a list or a map (an empty Lua table reads as an empty list).
    pub fn is_table(&self) -> bool {
        matches!(self, Value::List(_) | Value::Map(_))
    }

    /// A number or a text written as Lua's `tostring` writes it (`12.5`, `2.0`, `1e+20`).
    pub fn to_text(&self) -> Option<String> {
        match self {
            Value::Str(s) => Some(s.clone()),
            Value::Int(i) => Some(i.to_string()),
            Value::Num(n) => Some(lua_number(*n)),
            _ => None,
        }
    }

    /// What kind of value it is, for error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::List(_) | Value::Map(_) => "table",
            Value::Callback(_) => "function",
            Value::Bytes(_) => "bytes",
        }
    }

    /// A map from pairs whose keys are static names (the game's records).
    pub fn map<const N: usize>(pairs: [(&str, Value); N]) -> Value {
        Value::Map(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// `Some(x)` as the value, `None` as nil.
    pub fn opt<T: Into<Value>>(v: Option<T>) -> Value {
        v.map_or(Value::Nil, Into::into)
    }
}

/// A number as Lua 5.4 writes it: `%.14g`, with `.0` after a float that looks whole.
pub fn lua_number(n: f64) -> String {
    if n.is_nan() {
        return if n.is_sign_negative() { "-nan".into() } else { "nan".into() };
    }
    if n.is_infinite() {
        return if n > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let s = format_g(n, 14);
    if s.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        format!("{s}.0")
    } else {
        s
    }
}

/// C's `%.<prec>g`.
fn format_g(n: f64, prec: usize) -> String {
    if n == 0.0 {
        return if n.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let exp = n.abs().log10().floor() as i32;
    // (rounding can carry into the next power of ten: 9.99999999999999e14 -> 1e15)
    let e = format!("{:.*e}", prec - 1, n);
    let (mant, ex) = e.split_once('e').unwrap_or((&e, "0"));
    let ex: i32 = ex.parse().unwrap_or(exp);
    if ex < -4 || ex >= prec as i32 {
        let mant = trim_zeros(mant);
        let sign = if ex < 0 { '-' } else { '+' };
        format!("{mant}e{sign}{:02}", ex.abs())
    } else {
        let decimals = (prec as i32 - 1 - ex).max(0) as usize;
        trim_zeros(&format!("{n:.decimals$}")).to_string()
    }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Value {
        Value::Num(n)
    }
}
impl From<f32> for Value {
    /// A game value kept as `f32`, as the number it reads as (2.1, not 2.0999999046325684).
    fn from(n: f32) -> Value {
        Value::Num(n.to_string().parse().unwrap_or(n as f64))
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Value {
        Value::Int(n)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Value {
        Value::Int(n as i64)
    }
}
impl From<u32> for Value {
    fn from(n: u32) -> Value {
        Value::Int(n as i64)
    }
}
impl From<usize> for Value {
    fn from(n: usize) -> Value {
        Value::Int(n as i64)
    }
}
impl From<u64> for Value {
    fn from(n: u64) -> Value {
        Value::Int(n.min(i64::MAX as u64) as i64)
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(s)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(s.to_string())
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Value {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}
impl From<crate::InfoValue> for Value {
    fn from(v: crate::InfoValue) -> Value {
        match v {
            crate::InfoValue::Num(n) => Value::Num(n),
            crate::InfoValue::Text(s) => Value::Str(s),
            crate::InfoValue::Bool(b) => Value::Bool(b),
            crate::InfoValue::Nil => Value::Nil,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_lua_writes_them() {
        for (n, s) in [(12.5, "12.5"), (2.0, "2.0"), (1e20, "1e+20"), (-0.5, "-0.5"), (0.1, "0.1"), (1e-5, "1e-05"), (123456789012345.0, "1.2345678901234e+14"), (100.0, "100.0")] {
            assert_eq!(lua_number(n), s, "{n}");
        }
    }

    #[test]
    fn conversions() {
        assert_eq!(Value::Str(" 5 ".into()).as_f64(), Some(5.0));
        assert_eq!(Value::Num(3.0).as_i64(), Some(3));
        assert_eq!(Value::Num(3.5).as_i64(), None);
        assert!(Value::Int(0).truthy() && !Value::Bool(false).truthy() && !Value::Nil.truthy());
        let m = Value::map([("a", 1i64.into()), ("b", "x".into())]);
        assert_eq!(m.get("b"), Some(&Value::Str("x".into())));
        assert_eq!(Value::from(2.1f32), Value::Num(2.1));
    }
}
