//! WebAssembly plugins: the host side of the ABI, with small modules written in WAT.

use omsi_plugin::wasm::{Dispatch, WasmPlugin};

#[derive(Default)]
struct Game {
    calls: Vec<(String, String)>,
    logs: Vec<(i32, String)>,
}

impl Dispatch for Game {
    fn call(&mut self, name: &str, args: &str) -> Result<String, String> {
        self.calls.push((name.to_string(), args.to_string()));
        match name {
            "speed" => Ok("42.5".into()),
            "message" => Ok("null".into()),
            "on" => Ok("true".into()),
            _ => Err(format!("{name}: no such function")),
        }
    }
    fn log(&mut self, level: i32, text: &str) {
        self.logs.push((level, text.to_string()));
    }
}

/// A guest with a bump allocator; `oop_start` calls `speed` and passes the result to `message`,
/// asks for an unknown function and logs its error, and registers callback 7 for `frame`.
/// `oop_callback` logs the arguments it gets.
const GUEST: &str = r#"
(module
  (import "openomsi" "call" (func $call (param i32 i32 i32 i32) (result i64)))
  (import "openomsi" "last_error" (func $last_error (result i64)))
  (import "openomsi" "log" (func $log (param i32 i32 i32)))
  (memory (export "memory") 2)
  (global $next (mut i32) (i32.const 4096))
  (data (i32.const 0) "speed")
  (data (i32.const 16) "[]")
  (data (i32.const 32) "message")
  (data (i32.const 48) "nope")
  (data (i32.const 64) "on")
  (data (i32.const 80) "[\"frame\",{\"$cb\":7}]")
  (func (export "oop_abi") (result i32) (i32.const 1))
  (func $alloc (export "oop_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $p))
  (func (export "oop_free") (param i32 i32))
  (func $log_packed (param $level i32) (param $packed i64)
    (call $log (local.get $level)
      (i32.wrap_i64 (i64.shr_u (local.get $packed) (i64.const 32)))
      (i32.wrap_i64 (i64.and (local.get $packed) (i64.const 0xffffffff)))))
  (func (export "oop_start")
    (local $r i64)
    ;; speed() -> "42.5", logged
    (call $log_packed (i32.const 1) (call $call (i32.const 0) (i32.const 5) (i32.const 16) (i32.const 2)))
    ;; nope() -> -1, then the error is logged
    (local.set $r (call $call (i32.const 48) (i32.const 4) (i32.const 16) (i32.const 2)))
    (if (i64.lt_s (local.get $r) (i64.const 0))
      (then (call $log_packed (i32.const 3) (call $last_error))))
    ;; on("frame", cb 7)
    (drop (call $call (i32.const 64) (i32.const 2) (i32.const 80) (i32.const 19))))
  (func (export "oop_callback") (param $id i64) (param $ptr i32) (param $len i32)
    (call $log (i32.wrap_i64 (local.get $id)) (local.get $ptr) (local.get $len))))
"#;

#[test]
fn a_module_calls_the_api_and_is_called_back() {
    let mut game = Game::default();
    let mut p = WasmPlugin::load("guest", GUEST.as_bytes()).expect("load");
    p.start(&mut game);
    assert_eq!(p.disabled, None);
    assert_eq!(game.calls[0], ("speed".into(), "[]".into()));
    assert_eq!(game.calls[2], ("on".into(), r#"["frame",{"$cb":7}]"#.into()));
    assert_eq!(game.logs[0], (1, "42.5".into()));
    assert_eq!(game.logs[1], (3, "nope: no such function".into()));
    p.callback(&mut game, 7, "[0.016]");
    assert_eq!(game.logs[2], (7, "[0.016]".into()));
}

#[test]
fn an_endless_loop_stops_the_plugin_not_the_game() {
    let wat = r#"(module
      (memory (export "memory") 1)
      (func (export "oop_abi") (result i32) (i32.const 1))
      (func (export "oop_alloc") (param i32) (result i32) (i32.const 0))
      (func (export "oop_free") (param i32 i32))
      (func (export "oop_start") (loop $l (br $l))))"#;
    let mut game = Game::default();
    let mut p = WasmPlugin::load("looper", wat.as_bytes()).expect("load");
    let t = std::time::Instant::now();
    p.start(&mut game);
    assert!(p.disabled.as_deref().is_some_and(|w| w.contains("ran too long")), "{:?}", p.disabled);
    assert!(t.elapsed().as_secs_f32() < 2.0, "{:?}", t.elapsed());
    // and it is called no more
    p.callback(&mut game, 1, "[]");
    assert!(game.calls.is_empty());
}

#[test]
fn modules_of_another_abi_or_without_one_are_refused() {
    let other = r#"(module (memory (export "memory") 1)
      (func (export "oop_abi") (result i32) (i32.const 2))
      (func (export "oop_alloc") (param i32) (result i32) (i32.const 0))
      (func (export "oop_free") (param i32 i32)))"#;
    let e = WasmPlugin::load("future", other.as_bytes()).err().expect("refused");
    assert!(e.contains("ABI 2") && e.contains("update openOMSI"), "{e}");
    let none = r#"(module (memory (export "memory") 1))"#;
    let e = WasmPlugin::load("plain", none.as_bytes()).err().expect("refused");
    assert!(e.contains("oop_abi"), "{e}");
    assert!(WasmPlugin::load("junk", b"\0asm junk").is_err());
}

#[test]
fn memory_is_held_to_its_limit() {
    // 5000 pages = 312 MB, over the 256 MB limit: memory.grow answers -1
    let wat = r#"(module
      (import "openomsi" "log" (func $log (param i32 i32 i32)))
      (memory (export "memory") 1)
      (data (i32.const 0) "refused")
      (func (export "oop_abi") (result i32) (i32.const 1))
      (func (export "oop_alloc") (param i32) (result i32) (i32.const 0))
      (func (export "oop_free") (param i32 i32))
      (func (export "oop_start")
        (if (i32.eq (memory.grow (i32.const 5000)) (i32.const -1))
          (then (call $log (i32.const 1) (i32.const 0) (i32.const 7))))))"#;
    let mut game = Game::default();
    let mut p = WasmPlugin::load("hungry", wat.as_bytes()).expect("load");
    p.start(&mut game);
    assert_eq!(game.logs, vec![(1, "refused".into())]);
}

#[test]
fn bad_pointers_trap_instead_of_reading_the_host() {
    let wat = r#"(module
      (import "openomsi" "log" (func $log (param i32 i32 i32)))
      (memory (export "memory") 1)
      (func (export "oop_abi") (result i32) (i32.const 1))
      (func (export "oop_alloc") (param i32) (result i32) (i32.const 0))
      (func (export "oop_free") (param i32 i32))
      (func (export "oop_start") (call $log (i32.const 1) (i32.const 65530) (i32.const 100))))"#;
    let mut game = Game::default();
    let mut p = WasmPlugin::load("wild", wat.as_bytes()).expect("load");
    p.start(&mut game);
    assert!(p.disabled.as_deref().is_some_and(|w| w.contains("outside")), "{:?}", p.disabled);
    assert!(game.logs.is_empty());
}
