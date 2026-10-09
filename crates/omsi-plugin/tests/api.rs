//! The API registry and its runtime: the entries are well formed, the manifest and the docs
//! follow them, and the runtime's events, timers, watches, messages, budget and sandbox
//! work as documented.
mod common;

use common::{dir, run, Game};
use omsi_plugin::api::{self, json, Value};
use omsi_plugin::{HostConfig, Plugins};
use std::path::Path;

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

const LUA_KEYWORDS: &[&str] = &["and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"];

#[test]
fn every_entry_is_well_formed() {
    let all = api::registry();
    let mut names: Vec<&str> = all.iter().map(|f| f.name).collect();
    names.dedup();
    assert_eq!(names.len(), all.len(), "a name is there twice");
    let groups: Vec<&str> = api::docs::GROUPS.iter().map(|g| g.0).collect();
    for f in all {
        assert!(groups.contains(&f.group), "{}: group {} is not in the docs' list", f.name, f.group);
        assert!(f.doc.ends_with('.') || f.doc.ends_with(')') || f.doc.ends_with('`'), "{}: the doc is a sentence", f.name);
        assert!(!f.returns.is_empty() && !f.since.is_empty(), "{}", f.name);
        for part in f.name.split('.') {
            assert!(!LUA_KEYWORDS.contains(&part), "{}: `{part}` is a Lua keyword", f.name);
        }
        for p in f.params {
            assert!(["number", "integer", "string", "bool", "table", "function", "any"].contains(&p.ty), "{}: {} {}", f.name, p.name, p.ty);
        }
        // (a function is not also a table of functions: `omsi.x` and `omsi.x.y` cannot both be)
        assert!(api::find(f.name).is_some());
        assert!(!all.iter().any(|g| g.name.starts_with(&format!("{}.", f.name))), "{} is a function and a table", f.name);
    }
    let mut events: Vec<&str> = api::events::EVENTS.iter().map(|e| e.name).collect();
    events.sort();
    events.dedup();
    assert_eq!(events.len(), api::events::EVENTS.len(), "an event is there twice");
}

/// docs/plugin-api.json is the registry's manifest, and docs/PLUGINS.md's reference its
/// tables (`OMSI_API_BLESS=1` writes both).
#[test]
fn api_manifest() {
    let manifest = json::encode_pretty(&api::manifest()) + "\n";
    let path = repo().join("docs/plugin-api.json");
    let doc_path = repo().join("docs/PLUGINS.md");
    let doc = std::fs::read_to_string(&doc_path).unwrap();
    let spliced = api::docs::splice(&doc).expect("docs/PLUGINS.md has the api:begin / api:end markers");
    if std::env::var_os("OMSI_API_BLESS").is_some() {
        std::fs::write(&path, &manifest).unwrap();
        std::fs::write(&doc_path, &spliced).unwrap();
        return;
    }
    // (a Windows checkout may have turned the files' line ends into CRLF)
    let have = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    let doc = doc.replace("\r\n", "\n");
    let spliced = spliced.replace("\r\n", "\n");
    assert!(have == manifest, "docs/plugin-api.json is not the registry's: run OMSI_API_BLESS=1 cargo test -p omsi-plugin api_manifest");
    assert!(doc == spliced, "docs/PLUGINS.md's function tables are not the registry's: run OMSI_API_BLESS=1 cargo test -p omsi-plugin api_manifest");
    // (the manifest reads back)
    let back = json::decode(&manifest, true).unwrap();
    assert_eq!(back.get("abi"), Some(&Value::Int(api::ABI)));
    assert!(back.get("functions").unwrap().items().len() >= 200);
    assert!(back.get("events").unwrap().items().len() >= 40);
}

#[test]
fn events_timers_watches_and_cancel() {
    let mut g = Game::new();
    let src = r#"
        local log = {}
        local function say(s) log[#log + 1] = s end
        omsi.on("tick", function(a, b) say("on " .. a .. b) end)
        function on_tick(a) say("global " .. a) end
        local h = function() say("off-me") end
        omsi.on("tick", h); omsi.off("tick", h)
        omsi.emit("tick", 1, 2)
        local once = omsi.after(1, function() say("after") end)
        local n = 0
        local ev = omsi.every(0.5, function() n = n + 1; if n == 3 then say("every3") end end)
        omsi.after(0.5, function() say("cancelled?") end)
        omsi.cancel(omsi.after(0.5, function() say("never") end))
        omsi.watch("var", "Velocity", function(v, old) say("watch " .. tostring(old) .. ">" .. v) end)
        omsi.watch("info", "line", function(v) say("line " .. tostring(v)) end)
        assert(not pcall(omsi.every, 0, function() end))
        function on_frame()
          if omsi.time() == 2 then omsi.message(table.concat(log, ",")) end
        end
    "#;
    g.vars.insert("Velocity".into(), 10.0);
    let mut p = run("timers", src, &mut g, 2);
    g.vars.insert("Velocity".into(), 20.0);
    g.info = vec![("line", omsi_plugin::InfoValue::Text("136".into()))];
    p.frame(&mut g);
    p.frame(&mut g);
    assert_eq!(g.messages, ["on 12,global 1,cancelled?,watch nil>10.0,after,every3,watch 10.0>20.0,line 136"]);
}

#[test]
fn messages_between_plugins() {
    let d = dir("mail");
    std::fs::write(d.join("a.lua"), r#"
        local sent = false
        function on_frame()
          if sent then return end
          sent = true
          omsi.plugin.send("b", "hello", { n = 42 })
          omsi.plugin.broadcast("all", "x")
        end
        function on_message(from, topic, data) omsi.message("a got " .. topic .. " from " .. from) end
    "#).unwrap();
    std::fs::write(d.join("b.lua"), r#"
        function on_message(from, topic, data)
          omsi.message("b got " .. topic .. " " .. tostring(type(data) == "table" and data.n or data) .. " from " .. from)
          if topic == "hello" then omsi.plugin.send(from, "thanks") end
        end
        assert(#omsi.plugin.list() >= 1)
    "#).unwrap();
    let mut p = Plugins::load(&[d], &HostConfig::default());
    let mut g = Game::new();
    p.frame(&mut g);
    p.frame(&mut g);
    assert_eq!(g.messages, ["b got hello 42 from a", "b got all x from a", "a got thanks from b"]);
}

#[test]
fn a_runaway_call_is_stopped_at_its_budget_and_the_plugin_switched_off() {
    let mut g = Game::new();
    let p = run("runaway", "function on_frame() local t = os.clock() while os.clock() - t < 5 do end end", &mut g, 1);
    assert!(p.lua[0].disabled, "{:?}", g.messages);
    assert!(g.messages.iter().any(|m| m.contains("longer than 50 ms")), "{:?}", g.messages);
    // a plugin catching the error is stopped all the same
    let mut g = Game::new();
    let t = std::time::Instant::now();
    let p = run("runaway2", "function on_frame() pcall(function() while true do end end) while true do end end", &mut g, 3);
    assert!(p.lua[0].disabled);
    assert!(t.elapsed().as_secs_f32() < 2.0);
}

#[test]
fn the_sandbox_holds() {
    let mut g = Game::new();
    let src = r##"
        local done = false
        function on_frame()
        if done then return end
        done = true
        local r = {}
        r[#r + 1] = load("return 1 + 1")() == 2
        r[#r + 1] = load(string.dump(function() return 1 end)) == nil
        local f, err = load("\27Lua\84\0\25\147\13\10\26\10")
        r[#r + 1] = f == nil and err ~= nil
        r[#r + 1] = select("#", load(function() return nil end)) >= 1
        r[#r + 1] = io == nil and dofile == nil and loadfile == nil and require("os").execute == nil
        r[#r + 1] = not pcall(require, "../../etc/passwd")
        r[#r + 1] = not pcall(require, "nonexistent_module")
        r[#r + 1] = package.loadlib == nil and package.cpath == ""
        local ok = omsi.files.write("../escape.txt", "x")
        r[#r + 1] = ok == false
        r[#r + 1] = omsi.files.write("/abs.txt", "x") == false
        r[#r + 1] = select(2, omsi.plugin.read("../../secret")) ~= nil
        local s = {}
        for i, v in ipairs(r) do s[#s + 1] = tostring(v) end
        omsi.message(table.concat(s, " "))
        end
    "##;
    run("sandbox", src, &mut g, 2);
    assert_eq!(g.messages, ["true true true true true true true true true true true"]);
}

#[test]
fn storage_and_files_stay_in_the_data_folder() {
    let d = dir("storage");
    std::fs::write(d.join("store.lua"), r#"
        omsi.storage.set("runs", (omsi.storage.get("runs") or 0) + 1)
        omsi.storage.set("table", { a = 1, b = { "x", "y" } })
        assert(not pcall(omsi.storage.set, "f", function() end))
        assert(omsi.files.write("log/trips.csv", "a,b\n"))
        assert(omsi.files.append("log/trips.csv", "1,2\n"))
        assert(omsi.files.read("log/trips.csv") == "a,b\n1,2\n")
        assert(omsi.files.exists("log") and #omsi.files.list("log") == 1)
        local v, why = omsi.files.read("nope.txt")
        assert(v == nil and why)
        local t = omsi.json.decode(omsi.json.encode({ x = 1, list = { 1, 2, 3 } }))
        assert(t.x == 1 and #t.list == 3)
    "#).unwrap();
    for runs in 1..=2 {
        let mut p = Plugins::load(std::slice::from_ref(&d), &HostConfig::default());
        let mut g = Game::new();
        p.frame(&mut g);
        assert!(g.messages.is_empty(), "{:?}", g.messages);
        p.finalize();
        let saved = std::fs::read_to_string(d.join("store.data/storage.json")).unwrap();
        let v = json::decode(&saved, false).unwrap();
        assert_eq!(v.get("runs"), Some(&Value::Int(runs)));
    }
    assert!(d.join("store.data/log/trips.csv").is_file());
}

#[test]
fn settings_make_a_panel_and_tell_their_changes() {
    let mut g = Game::new();
    let src = r#"
        local s = omsi.plugin.settings({
          { key = "volume", type = "number", label = "Volume", default = 50, min = 0, max = 100, step = 10 },
          { key = "rain", type = "bool", default = true },
          { key = "mode", type = "choice", choices = { "a", "b" }, default = "b" },
        }, "My plugin")
        assert(s.volume == 50 and s.rain == true and s.mode == "b")
        assert(omsi.plugin.show_settings())
        function on_setting(k, v) omsi.message(k .. "=" .. tostring(v)) end
    "#;
    let mut p = run("settings", src, &mut g, 1);
    let owner = p.ui.borrow().panels()[0].owner;
    p.ui.borrow_mut().click(owner, "__settings", Some("rain"));
    p.ui.borrow_mut().set_control(owner, "__settings", "volume", 0.71);
    p.frame(&mut g);
    assert_eq!(g.messages, ["rain=false", "volume=70.0"]);
    p.ui.borrow_mut().click(owner, "__settings", Some("__close"));
    p.frame(&mut g);
    assert!(p.ui.borrow().panels().is_empty());
}

#[test]
fn errors_name_the_function_and_argument() {
    let mut g = Game::new();
    run("errs", r#"
        function on_frame()
          local ok, e = pcall(omsi.var, {})
          omsi.message(tostring(e):match("omsi%.var: argument 1 %(name%) must be a string") and "yes" or tostring(e))
        end
    "#, &mut g, 1);
    assert_eq!(g.messages, ["yes"]);
}
