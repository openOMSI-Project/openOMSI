//! Compiled plugins (`.oop`): built with the `oop-format` crate as `oopc` builds them, then
//! loaded from a plugins folder and driven as the game drives them.

use oop_format::{Header, Kind, OopBuilder};
use omsi_plugin::{HostConfig, PluginIo, Plugins};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default)]
struct Game {
    vars: HashMap<String, f32>,
    messages: Vec<String>,
}

impl PluginIo for Game {
    fn system(&mut self, _: &str) -> Option<f32> {
        None
    }
    fn set_system(&mut self, _: &str, _: f32) {}
    fn has_vehicle(&self) -> bool {
        true
    }
    fn var(&mut self, name: &str) -> Option<f32> {
        self.vars.get(name).copied()
    }
    fn set_var(&mut self, name: &str, v: f32) {
        self.vars.insert(name.into(), v);
    }
    fn string(&mut self, _: &str) -> Option<String> {
        None
    }
    fn set_string(&mut self, _: &str, _: &str) {}
    fn fire(&mut self, _: &str, _: bool) {}
    fn dt(&self) -> f32 {
        0.1
    }
    fn message(&mut self, text: &str, _: f32) {
        self.messages.push(text.into());
    }
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omsi-oop-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn header(id: &str, kind: Kind, entry: &str, perms: &[&str]) -> Header {
    let mut h = Header::new(id, id, "1.0.0", kind, entry);
    h.permissions = perms.iter().map(|p| p.to_string()).collect();
    h
}

/// Lua compiled into an `.oop`: its modules come from the archive, never the disk, and a
/// function outside its permissions fails.
#[test]
fn a_lua_oop_runs_from_memory_within_its_permissions() {
    let d = dir("lua");
    let oop = OopBuilder::new(header("org.test.greeter", Kind::Lua, "main.lua", &["ui"]))
        .file("main.lua", r#"
            local u = require("util")
            local done = false
            omsi.on("frame", function()
                if done then return end
                done = true
                omsi.message("lua " .. u.x)
                local ok = pcall(omsi.set_var, "doors", 1)
                omsi.message("set_var allowed: " .. tostring(ok))
            end)
        "#)
        .file("util.lua", "return { x = 42 }")
        .file("icon.png", vec![0x89, b'P', b'N', b'G'])
        .write()
        .unwrap();
    std::fs::write(d.join("greeter.oop"), oop).unwrap();
    let mut plugins = Plugins::load(std::slice::from_ref(&d), &HostConfig::default());
    assert_eq!(plugins.lua.len(), 1, "loaded");
    let mut game = Game::default();
    plugins.frame(&mut game);
    assert_eq!(game.messages, ["lua 42", "set_var allowed: false"]);
    // no code on disk; the asset in the cache
    assert!(!d.join(".oop-cache/greeter/main.lua").exists());
    assert!(!d.join(".oop-cache/greeter/util.lua").exists());
    assert!(d.join(".oop-cache/greeter/icon.png").exists());
    assert!(game.vars.is_empty());
    plugins.finalize();
}

/// A WebAssembly plugin: calls the API by name, gets its frame callback, and its own `emit`
/// reaches its own handler (run after the call into the module returns).
const GUEST: &str = r#"
(module
  (import "openomsi" "call" (func $call (param i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 2)
  (global $next (mut i32) (i32.const 8192))
  (data (i32.const 0) "message")
  (data (i32.const 16) "[\"hi from wasm\"]")
  (data (i32.const 48) "on")
  (data (i32.const 64) "[\"frame\",{\"$cb\":1}]")
  (data (i32.const 96) "[\"ping\",{\"$cb\":2}]")
  (data (i32.const 128) "emit")
  (data (i32.const 144) "[\"ping\"]")
  (data (i32.const 160) "[\"pong\"]")
  (data (i32.const 176) "set_var")
  (data (i32.const 192) "[\"doors\",1]")
  (data (i32.const 208) "[\"set_var refused\"]")
  (func (export "oop_abi") (result i32) (i32.const 1))
  (func (export "oop_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $p))
  (func (export "oop_free") (param i32 i32))
  (func (export "oop_start")
    (drop (call $call (i32.const 0) (i32.const 7) (i32.const 16) (i32.const 16)))
    (drop (call $call (i32.const 48) (i32.const 2) (i32.const 64) (i32.const 19)))
    (drop (call $call (i32.const 48) (i32.const 2) (i32.const 96) (i32.const 18)))
    (if (i64.lt_s (call $call (i32.const 176) (i32.const 7) (i32.const 192) (i32.const 11)) (i64.const 0))
      (then (drop (call $call (i32.const 0) (i32.const 7) (i32.const 208) (i32.const 20))))))
  (func (export "oop_callback") (param $id i64) (param i32 i32)
    (if (i64.eq (local.get $id) (i64.const 1))
      (then (drop (call $call (i32.const 128) (i32.const 4) (i32.const 144) (i32.const 8)))))
    (if (i64.eq (local.get $id) (i64.const 2))
      (then (drop (call $call (i32.const 0) (i32.const 7) (i32.const 160) (i32.const 8)))))))
"#;

#[test]
fn a_wasm_oop_calls_the_api_and_hears_events() {
    let d = dir("wasm");
    // (wasmi reads the text format too; `oopc` puts a binary module here)
    let oop = OopBuilder::new(header("org.test.pinger", Kind::Wasm, "plugin.wasm", &["ui"])).file("plugin.wasm", GUEST).write().unwrap();
    std::fs::write(d.join("pinger.oop"), oop).unwrap();
    let mut plugins = Plugins::load(std::slice::from_ref(&d), &HostConfig::default());
    assert_eq!(plugins.wasm.len(), 1, "loaded");
    let mut game = Game::default();
    plugins.frame(&mut game);
    plugins.frame(&mut game);
    assert!(!plugins.wasm[0].disabled);
    // frame -> emit("ping") -> its own ping handler -> message("pong"), once a frame
    assert_eq!(game.messages.iter().filter(|m| *m == "pong").count(), 2, "{:?}", game.messages);
    assert!(game.vars.is_empty(), "set_var needs vehicle_write");
    plugins.finalize();
}

/// A file changed after it was built is refused, and nothing of it runs.
#[test]
fn a_changed_oop_is_refused() {
    let d = dir("tampered");
    let mut oop = OopBuilder::new(header("org.test.bad", Kind::Lua, "main.lua", &["ui"])).file("main.lua", "omsi.message('x')").write().unwrap();
    let n = oop.len();
    oop[n - 3] ^= 0x40;
    std::fs::write(d.join("bad.oop"), oop).unwrap();
    let plugins = Plugins::load(std::slice::from_ref(&d), &HostConfig::default());
    assert!(plugins.lua.is_empty() && plugins.wasm.is_empty());
}

/// A plain `.lua` plugin of the same name wins over an `.oop` (the one being worked on).
#[test]
fn a_lua_file_of_the_same_name_wins() {
    let d = dir("both");
    std::fs::write(d.join("same.lua"), "omsi.log('plain')").unwrap();
    let oop = OopBuilder::new(header("org.test.same", Kind::Lua, "main.lua", &[])).file("main.lua", "omsi.log('oop')").write().unwrap();
    std::fs::write(d.join("same.oop"), oop).unwrap();
    let plugins = Plugins::load(std::slice::from_ref(&d), &HostConfig::default());
    assert_eq!(plugins.lua.len(), 1);
    assert!(plugins.lua[0].path.extension().is_some_and(|x| x == "lua"));
}
