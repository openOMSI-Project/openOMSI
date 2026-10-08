//! Lua plugins driven as the game drives them: a fake bus, a few frames.
use omsi_plugin::{GameEvent, HostConfig, InfoValue, PluginIo, Plugins};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default)]
struct Bus {
    vars: HashMap<String, f32>,
    strings: HashMap<String, String>,
    fired: Vec<(String, bool)>,
    messages: Vec<String>,
    vehicle: bool,
    events: Vec<GameEvent>,
    info: Vec<(&'static str, InfoValue)>,
}

impl PluginIo for Bus {
    fn system(&mut self, name: &str) -> Option<f32> {
        (name == "Time").then_some(43200.0)
    }
    fn set_system(&mut self, _: &str, _: f32) {}
    fn has_vehicle(&self) -> bool {
        self.vehicle
    }
    fn var(&mut self, name: &str) -> Option<f32> {
        self.vars.get(name).copied()
    }
    fn set_var(&mut self, name: &str, v: f32) {
        self.vars.insert(name.into(), v);
    }
    fn string(&mut self, name: &str) -> Option<String> {
        self.strings.get(name).cloned()
    }
    fn set_string(&mut self, name: &str, s: &str) {
        self.strings.insert(name.into(), s.into());
    }
    fn fire(&mut self, t: &str, down: bool) {
        self.fired.push((t.into(), down));
    }
    fn dt(&self) -> f32 {
        0.5
    }
    fn vehicle_name(&self) -> Option<String> {
        Some("MAN SD202".into())
    }
    fn vehicle_manufacturer_model(&self) -> Option<(String, String)> {
        Some(("MAN".into(), "SD202".into()))
    }
    fn message(&mut self, text: &str, _: f32) {
        self.messages.push(text.into());
    }
    fn events(&self) -> Vec<GameEvent> {
        self.events.clone()
    }
    fn info(&self) -> Vec<(&'static str, InfoValue)> {
        self.info.clone()
    }
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omsi-lua-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn events_vars_timers_and_data() {
    let d = dir("main");
    std::fs::create_dir_all(d.join("Speedo")).unwrap();
    std::fs::write(d.join("Speedo/util.lua"), "return { double = function(x) return x * 2 end }").unwrap();
    std::fs::write(
        d.join("Speedo/main.lua"),
        r#"
        local util = require("util")
        omsi.data.runs = (omsi.data.runs or 0) + 1
        local ticks = 0
        omsi.on("vehicle", function(name) omsi.set_str("bus_name", name) end)
        omsi.every(1, function() ticks = ticks + 1; omsi.set_var("ticks", ticks) end)
        omsi.watch("Velocity", function(v, old) if old then omsi.message("v " .. v) end end)
        function on_frame(dt)
          omsi.set_var("doubled", util.double(omsi.var("Velocity") or 0))
          omsi.set_var("time", omsi.sys("Time"))
          if omsi.var("Velocity") == 50 then omsi.trigger("bus_horn") end
          assert(io == nil and os.execute == nil and dofile == nil)
          -- (#1715: the module table must not hand out the whole os library)
          local shut = require("os").execute == nil and require("os").remove == nil and require("os").clock ~= nil
            and package.loaded.io == nil and package.loaded.debug == nil
            and load(string.dump(function() return 1 end)) == nil
          omsi.set_var("sandbox", shut and 1 or 0)
        end
        "#,
    )
    .unwrap();
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    assert_eq!(plugins.lua.len(), 1);
    assert_eq!(plugins.lua[0].name, "Speedo");
    let mut bus = Bus { vehicle: true, ..Default::default() };
    for k in ["Velocity", "doubled", "ticks", "time", "sandbox"] {
        bus.vars.insert(k.into(), 0.0);
    }
    bus.strings.insert("bus_name".into(), String::new());
    bus.vars.insert("Velocity".into(), 20.0);
    for _ in 0..4 {
        plugins.frame(&mut bus);
    }
    assert_eq!(bus.strings["bus_name"], "MAN SD202");
    assert_eq!(bus.vars["doubled"], 40.0);
    assert_eq!(bus.vars["time"], 43200.0);
    assert_eq!(bus.vars["ticks"], 2.0);
    assert_eq!(bus.vars["sandbox"], 1.0, "require(\"os\") or a binary chunk got out of the sandbox");
    bus.vars.insert("Velocity".into(), 50.0);
    plugins.frame(&mut bus);
    assert_eq!(bus.fired, [("bus_horn".to_string(), true), ("bus_horn".to_string(), false)]);
    assert_eq!(bus.messages, ["v 50.0"]);
    plugins.finalize();
    let saved = std::fs::read_to_string(d.join("Speedo/data.save.lua")).unwrap();
    assert!(saved.contains("runs = 1"), "{saved}");
    // the next session reads it back
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    plugins.finalize();
    assert!(std::fs::read_to_string(d.join("Speedo/data.save.lua")).unwrap().contains("runs = 2"));
}

#[test]
fn errors_and_runaway_loops_are_contained() {
    let d = dir("bad");
    std::fs::write(d.join("loop.lua"), "function on_frame() while true do end end").unwrap();
    std::fs::write(d.join("broken.lua"), "function on_frame() error('boom') end").unwrap();
    std::fs::write(d.join("syntax.lua"), "this is not lua").unwrap();
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    assert_eq!(plugins.lua.len(), 2, "the file that does not compile is left out");
    let mut bus = Bus { vehicle: true, ..Default::default() };
    let t = std::time::Instant::now();
    plugins.frame(&mut bus);
    assert!(t.elapsed().as_secs_f32() < 3.0);
    for _ in 0..12 {
        plugins.frame(&mut bus);
    }
    assert!(plugins.lua.iter().all(|p| p.disabled));
    assert!(bus.messages.iter().any(|m| m.contains("boom")));
}


#[test]
fn game_events_reach_every_plugin_once() {
    let d = dir("events");
    let plugin = r#"
        omsi.on("crash", function(kj, kmh) omsi.set_var("kj", omsi.var("kj") + kj); omsi.set_var("kmh", kmh) end)
        function on_pedestrian(n) omsi.set_var("hurt", omsi.var("hurt") + n) end
        omsi.on("stops_skipped", function(n, from, to) omsi.message(string.format("%d %d %d", n, from, to)) end)
    "#;
    std::fs::write(d.join("a.lua"), plugin).unwrap();
    std::fs::write(d.join("b.lua"), plugin).unwrap();
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    assert_eq!(plugins.lua.len(), 2);
    let mut bus = Bus { vehicle: true, ..Default::default() };
    for k in ["kj", "kmh", "hurt"] {
        bus.vars.insert(k.into(), 0.0);
    }
    bus.events = vec![
        GameEvent { name: "crash", args: vec![InfoValue::Num(136.0), InfoValue::Num(42.0)] },
        GameEvent { name: "crash", args: vec![InfoValue::Num(136.0), InfoValue::Num(40.0)] },
        GameEvent { name: "pedestrian", args: vec![InfoValue::Num(1.0)] },
        GameEvent { name: "stops_skipped", args: vec![InfoValue::Num(7.0), InfoValue::Num(2.0), InfoValue::Num(9.0)] },
    ];
    plugins.frame(&mut bus);
    // (both plugins write the same variables: each adds its own)
    assert_eq!(bus.vars["kj"], 2.0 * 272.0, "two crashes of the same energy are two events");
    assert_eq!(bus.vars["kmh"], 40.0);
    assert_eq!(bus.vars["hurt"], 2.0);
    assert_eq!(bus.messages, ["7 2 9", "7 2 9"]);
    bus.events.clear();
    plugins.frame(&mut bus);
    assert_eq!(bus.vars["hurt"], 2.0);
}

#[test]
fn saved_data_of_a_one_file_plugin_is_no_plugin() {
    let d = dir("save");
    std::fs::write(d.join("counter.lua"), "omsi.data.n = (omsi.data.n or 0) + 1").unwrap();
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    plugins.finalize();
    assert!(d.join("counter.save.lua").is_file());
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    assert_eq!(plugins.lua.len(), 1, "counter.save.lua is not started");
    plugins.finalize();
    assert!(std::fs::read_to_string(d.join("counter.save.lua")).unwrap().contains("n = 2"));
}

#[test]
fn next_stop_fires_for_a_stop_of_the_same_name() {
    let d = dir("next-stop");
    std::fs::write(d.join("stops.lua"), r#"function on_next_stop(new, old) omsi.message(string.format("%s %d", new, omsi.info().next_stop_number)) end"#).unwrap();
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    let mut bus = Bus { vehicle: true, ..Default::default() };
    // Grundorf's 76: stops 1 and 2 are both Bauernhof, one either side of the road
    for (name, number) in [("Bauernhof", 1.0), ("Bauernhof", 1.0), ("Bauernhof", 2.0), ("Nordspitze", 3.0)] {
        bus.info = vec![("next_stop", InfoValue::Text(name.into())), ("next_stop_number", InfoValue::Num(number))];
        plugins.frame(&mut bus);
    }
    assert_eq!(bus.messages, ["Bauernhof 1", "Bauernhof 2", "Nordspitze 3"]);
}

#[test]
fn vehicle_manufacturer_and_model_apart() {
    let d = dir("vehicle");
    std::fs::write(
        d.join("names.lua"),
        r##"function on_frame() omsi.message(string.format("%s|%s|%s|%d", omsi.vehicle(), omsi.vehicle_manufacturer(), omsi.vehicle_model(), select("#", omsi.vehicle()))) end"##,
    )
    .unwrap();
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    let mut bus = Bus { vehicle: true, ..Default::default() };
    plugins.frame(&mut bus);
    bus.vehicle = false;
    plugins.frame(&mut bus);
    // (omsi.vehicle() gives the name alone, as ever)
    assert_eq!(bus.messages, ["MAN SD202|MAN|SD202|1", "nil|nil|nil|1"]);
}

#[test]
fn send_reaches_a_program_on_this_computer_only() {
    let listener = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    listener.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
    let port = listener.local_addr().unwrap().port();
    let d = dir("send");
    std::fs::write(
        d.join("sender.lua"),
        format!(
            r#"
            local done = false
            function on_frame()
              if done then return end
              done = true
              assert(omsi.send({port}, "hello"))
              local ok, why = omsi.send(80, "x")
              assert(not ok and why:find("1024"), why)
              -- (the game's own multiplayer ports are not for plugins)
              ok, why = omsi.send(27016, "x")
              assert(not ok and why:find("multiplayer"), why)
              -- 8 KB goes (macOS takes datagrams of at most 9 KB), more does not
              assert(omsi.send({port}, string.rep("x", 8 * 1024)))
              ok, why = omsi.send({port}, string.rep("x", 8 * 1024 + 1))
              assert(not ok and why:find("8 KB"), why)
              local sent = 2
              for _ = 1, 150 do
                if omsi.send({port}, "tick") then sent = sent + 1 end
              end
              omsi.message("sent " .. sent)
            end
            "#
        ),
    )
    .unwrap();
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    let mut bus = Bus { vehicle: true, ..Default::default() };
    plugins.frame(&mut bus);
    assert_eq!(bus.messages, ["sent 100"], "at most 100 messages in a second");
    let mut buf = vec![0u8; 64 * 1024];
    let (n, from) = listener.recv_from(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"hello");
    assert!(from.ip().is_loopback());
    assert_eq!(listener.recv(&mut buf).unwrap(), 8 * 1024, "the large message whole");
}

#[test]
fn panels_clicks_and_focus_through_omsi_ui() {
    let d = dir("ui");
    let plugin = r#"
        assert(omsi.ui.version >= 1)
        local ok, why = omsi.ui.set("trip", { anchor = "top_right", children = {
            { type = "text", text = "Linie 42" },
            { type = "button", id = "pause", text = "Pause" },
        } })
        assert(ok and why == nil)
        -- a mistake comes back as false and the reason, not as an error
        local bad, reason = omsi.ui.set("bad", { width = "wide" })
        assert(bad == false and reason == "width: a number is expected", reason)
        assert(omsi.ui.toast("Hallo", { seconds = 2, icon = "payments" }))
        assert(omsi.ui.toast({}) == false)
        omsi.ui.set("gone", {})
        assert(omsi.ui.remove("gone") and not omsi.ui.remove("gone"))
        local w, h, scale = omsi.ui.screen()
        assert(w > 0 and h > 0 and scale > 0)
        omsi.on("ui_click", function(panel, element) omsi.message("click " .. panel .. " " .. tostring(element)) end)
        function on_ui_focus(on) omsi.message("focus " .. tostring(on)) end
        omsi.on("key", function(key, down)
          if key == "F10" and down then omsi.message("focus asked " .. tostring(omsi.ui.focus(true))) end
        end)
    "#;
    std::fs::write(d.join("career.lua"), plugin).unwrap();
    let mut plugins = Plugins::load(std::slice::from_ref(&d), &HostConfig::default());
    let ui = plugins.ui.clone();
    assert_eq!(
        ui.borrow().panels().len(),
        1,
        "a panel set by the top level is there before the first frame"
    );
    assert_eq!(ui.borrow().toasts().len(), 1);
    let owner = ui.borrow().panels()[0].owner;
    let mut bus = Bus {
        vehicle: true,
        ..Default::default()
    };
    let mut keys = vec![("F10".to_string(), true)];
    struct Keyed<'a>(&'a mut Bus, &'a mut Vec<(String, bool)>);
    impl PluginIo for Keyed<'_> {
        fn system(&mut self, n: &str) -> Option<f32> {
            self.0.system(n)
        }
        fn set_system(&mut self, _: &str, _: f32) {}
        fn has_vehicle(&self) -> bool {
            true
        }
        fn var(&mut self, n: &str) -> Option<f32> {
            self.0.var(n)
        }
        fn set_var(&mut self, n: &str, v: f32) {
            self.0.set_var(n, v)
        }
        fn string(&mut self, n: &str) -> Option<String> {
            self.0.string(n)
        }
        fn set_string(&mut self, n: &str, s: &str) {
            self.0.set_string(n, s)
        }
        fn fire(&mut self, t: &str, down: bool) {
            self.0.fire(t, down)
        }
        fn message(&mut self, text: &str, s: f32) {
            self.0.message(text, s)
        }
        fn keys(&self) -> Vec<(String, bool)> {
            self.1.clone()
        }
    }
    plugins.frame(&mut Keyed(&mut bus, &mut keys));
    assert_eq!(bus.messages, ["focus asked true"]);
    assert!(ui.borrow().focused());
    // the game hands the plugin a click on its button; the plugin hears that it has the mouse
    ui.borrow_mut().click(owner, "trip", Some("pause"));
    keys.clear();
    plugins.frame(&mut Keyed(&mut bus, &mut keys));
    assert_eq!(bus.messages[1..], ["focus true", "click trip pause"]);
    // Esc: the game takes the mouse back
    ui.borrow_mut().release_focus();
    plugins.frame(&mut Keyed(&mut bus, &mut keys));
    assert_eq!(bus.messages.last().unwrap(), "focus false");
    // the plugin is loaded again after a change: its old panels go, it shows its new ones
    std::fs::write(d.join("career.lua"), "omsi.ui.set('again', {})").unwrap();
    let ids = |ui: &omsi_plugin::ui::SharedUi| -> Vec<String> {
        ui.borrow().panels().iter().map(|p| p.id.clone()).collect()
    };
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while ids(&ui) == ["trip"] && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(100));
        plugins.frame(&mut Keyed(&mut bus, &mut keys));
    }
    assert_eq!(ids(&ui), ["again"]);
    // and when it stops (its notifications run their time)
    plugins.finalize();
    assert!(ui.borrow().panels().is_empty());
    assert_eq!(ui.borrow().toasts().len(), 1);
}

/// The example of docs/PLUGINS.md shows its panel.
#[test]
fn the_trip_panel_example_shows_its_panel() {
    let d = dir("trip-panel");
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples/plugins/trip_panel.lua");
    std::fs::copy(example, d.join("trip_panel.lua")).unwrap();
    let mut plugins = Plugins::load(std::slice::from_ref(&d), &HostConfig::default());
    let mut bus = Bus {
        vehicle: true,
        ..Default::default()
    };
    bus.info = vec![
        ("line", InfoValue::Text("42".into())),
        ("delay", InfoValue::Num(95.0)),
        ("next_stop", InfoValue::Text("Zoo".into())),
        ("next_stop_number", InfoValue::Num(3.0)),
        ("stops", InfoValue::Num(9.0)),
        ("speed", InfoValue::Num(38.0)),
    ];
    plugins.frame(&mut bus);
    assert!(bus.messages.is_empty(), "{:?}", bus.messages);
    let ui = plugins.ui.borrow();
    let [trip] = ui.panels() else {
        panic!("{:?}", ui.panels())
    };
    assert_eq!((trip.id.as_str(), trip.panel.children.len()), ("trip", 5));
    assert_eq!(ui.toasts().len(), 1, "the next stop's notification");
}
