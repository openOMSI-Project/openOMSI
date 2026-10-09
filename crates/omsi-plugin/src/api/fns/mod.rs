//! The functions of the API, a file per part of the game. Each entry is written with
//! [`def!`]: name, group, parameters, what it returns, its documentation, the version it
//! came with, the permission it needs, whether its result is several values, and its body.

use super::{ApiFn, ApiError, Args, Ctx, Value};

pub(crate) mod bus;
pub(crate) mod core;
pub(crate) mod duty;
pub(crate) mod game;
pub(crate) mod map;
pub(crate) mod media;
pub(crate) mod plugin;
pub(crate) mod storage;
pub(crate) mod traffic;
pub(crate) mod util;
pub(crate) mod world;

/// Every group's functions.
pub static GROUPS: &[&[ApiFn]] = &[core::FNS, bus::FNS, duty::FNS, map::FNS, traffic::FNS, world::FNS, media::FNS, game::FNS, plugin::FNS, storage::FNS, util::FNS];

/// What a function body may give back: a value, something that becomes one, or an error.
pub trait Ret {
    fn ret(self) -> Result<Value, ApiError>;
}

impl<T: Into<Value>> Ret for T {
    fn ret(self) -> Result<Value, ApiError> {
        Ok(self.into())
    }
}

impl<T: Into<Value>> Ret for Result<T, ApiError> {
    fn ret(self) -> Result<Value, ApiError> {
        self.map(Into::into)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Value {
        v.map_or(Value::Nil, Into::into)
    }
}

impl From<()> for Value {
    fn from(_: ()) -> Value {
        Value::Nil
    }
}

/// One function: `def!("name", "group", [params], "returns", "doc", since, Perm, multi,
/// |ctx, args| body)`.
macro_rules! def {
    ($name:literal, $group:literal, [$($p:expr),* $(,)?], $ret:literal, $doc:literal, $since:expr, $perm:ident, $multi:literal, |$c:ident, $a:ident| $body:expr) => {
        $crate::api::ApiFn {
            name: $name,
            group: $group,
            params: &[$($p),*],
            returns: $ret,
            doc: $doc,
            since: $since,
            perm: $crate::api::Perm::$perm,
            multi: $multi,
            call: {
                #[allow(unused_mut, unused_variables, clippy::needless_question_mark)]
                fn h($c: &mut $crate::api::Ctx<'_>, mut $a: $crate::api::Args) -> Result<$crate::api::Value, $crate::api::ApiError> {
                    $crate::api::fns::Ret::ret($body)
                }
                h
            },
        }
    };
}
pub(crate) use def;

/// Several values (a function marked `multi`): none at all when `v` is None.
pub(crate) fn multi<const N: usize>(v: Option<[Value; N]>) -> Value {
    Value::List(v.map(Vec::from).unwrap_or_default())
}

/// Numbers as several values.
#[allow(dead_code)]
pub(crate) fn nums<const N: usize>(v: Option<[f64; N]>) -> Value {
    multi(v.map(|a| a.map(Value::Num)))
}

/// A record as a map.
pub(crate) fn rec(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// `[x, y, z]` as a map.
#[allow(dead_code)]
pub(crate) fn xyz(p: [f64; 3]) -> Vec<(&'static str, Value)> {
    vec![("x", p[0].into()), ("y", p[1].into()), ("z", p[2].into())]
}

/// Keep `Ctx`, `Args` in reach of the group files.
#[allow(unused_imports)]
pub(crate) use super::{o, p};
#[allow(dead_code)]
fn _uses(_: &mut Ctx<'_>, _: Args) {}
