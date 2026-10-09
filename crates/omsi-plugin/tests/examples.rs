//! The example plugins of docs/examples/plugins run against the fake game: they load, show
//! their panels, answer the player and the game's events, and say nothing went wrong.
mod common;

use common::{dir, Game};
use omsi_plugin::{HostConfig, Light, Plugins};
use std::path::Path;

fn examples() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/examples/plugins"))
}

/// A plugins folder holding one example (a file or a folder).
fn load(tag: &str, example: &str) -> Plugins {
    let d = dir(tag);
    let from = examples().join(example);
    if from.is_dir() {
        std::fs::create_dir_all(d.join(example)).unwrap();
        for f in std::fs::read_dir(&from).unwrap().flatten() {
            std::fs::copy(f.path(), d.join(example).join(f.file_name())).unwrap();
        }
    } else {
        std::fs::copy(&from, d.join(example)).unwrap();
    }
    let p = Plugins::load(&[d], &HostConfig::default());
    assert_eq!(p.lua.len(), 1, "{example} did not load");
    p
}

fn no_errors(g: &Game) {
    let errors: Vec<&String> = g.messages.iter().filter(|m| m.starts_with("Lua plugin")).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

fn panel_ids(p: &Plugins) -> Vec<String> {
    p.ui.borrow().panels().iter().map(|e| e.id.clone()).collect()
}

#[test]
fn the_dispatcher_runs_a_career() {
    let mut p = load("ex-dispatcher", "dispatcher");
    let mut g = Game::new();
    for _ in 0..3 {
        p.frame(&mut g);
    }
    no_errors(&g);
    assert_eq!(panel_ids(&p), ["main"]);
    let owner = p.ui.borrow().panels()[0].owner;
    // a stop served late, a trip done, a crash
    g.events_ex = vec![];
    g.duty.as_mut().unwrap().at_stop = true;
    p.frame(&mut g);
    g.duty.as_mut().unwrap().at_stop = false;
    g.duty.as_mut().unwrap().delay = 200.0;
    g.events = vec![omsi_plugin::GameEvent { name: "trip_done", args: vec![omsi_plugin::InfoValue::Num(1.0), omsi_plugin::InfoValue::Text("arrived".into()), omsi_plugin::InfoValue::Num(90.0), omsi_plugin::InfoValue::Num(100.0), omsi_plugin::InfoValue::Num(80.0)] }];
    p.frame(&mut g);
    g.events.clear();
    // the Career tab, then the Offer tab and a duty taken
    p.ui.borrow_mut().set_control(owner, "main", "tab", 0.5);
    p.frame(&mut g);
    p.ui.borrow_mut().set_control(owner, "main", "tab", 0.9);
    p.frame(&mut g);
    p.ui.borrow_mut().click(owner, "main", Some("take1"));
    p.frame(&mut g);
    no_errors(&g);
    assert!(g.log.borrow().iter().any(|l| l == "duty 136/3 0 0"), "{:?}", g.log.borrow());
    assert!(p.ui.borrow().toasts().iter().any(|t| t.text.contains("Trip 1 arrived")));
    // Ctrl+D hides it
    g.held = vec!["ControlLeft".into()];
    g.keys = vec![("KeyD".into(), true)];
    p.frame(&mut g);
    assert!(panel_ids(&p).is_empty());
    p.finalize();
    let saved = std::fs::read_to_string(dir_of(&p, "ex-dispatcher").join("dispatcher/data/storage.json")).unwrap();
    assert!(saved.contains("\"stops\": 1"), "{saved}");
}

fn dir_of(_: &Plugins, tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("omsi-api-test-{tag}-{}", std::process::id()))
}

#[test]
fn the_telemetry_hud_shows_and_sends() {
    let listener = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    listener.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut p = load("ex-hud", "telemetry_hud.lua");
    let mut g = Game::new();
    // (a running engine: its rpm is no whole number)
    g.engine = true;
    g.vars.insert("Velocity".into(), 42.0);
    p.frame(&mut g);
    // (a light comes into reach, showing red)
    g.light = Some(Light { aspect: "red", change_in: 5.0, distance: 40.0 });
    let owner = p.ui.borrow().panels().first().map(|e| e.owner);
    for _ in 0..3 {
        p.frame(&mut g);
    }
    no_errors(&g);
    assert_eq!(panel_ids(&p), ["hud"]);
    assert!(p.ui.borrow().toasts().iter().any(|t| t.text == "Red light ahead"));
    // the feed switched on in its settings (Ctrl+T shows them)
    let o = owner.unwrap();
    g.held = vec!["ControlLeft".into()];
    g.keys = vec![("KeyT".into(), true)];
    p.frame(&mut g);
    g.keys.clear();
    g.held.clear();
    assert!(panel_ids(&p).contains(&"__settings".to_string()));
    p.ui.borrow_mut().click(o, "__settings", Some("send"));
    // the port: a slider of 1024..65535, set to the listener's
    p.ui.borrow_mut().set_control(o, "__settings", "port", (port as f32 - 1024.0) / (65535.0 - 1024.0));
    p.frame(&mut g);
    for _ in 0..2 {
        p.frame(&mut g);
    }
    no_errors(&g);
    let mut buf = [0u8; 2048];
    let n = listener.recv(&mut buf).expect("a datagram");
    let text = std::str::from_utf8(&buf[..n]).unwrap();
    assert!(text.contains("\"speed\":42") && text.contains("\"line\":\"136\""), "{text}");
}

#[test]
fn the_weather_controller_changes_the_weather_and_the_clock() {
    let mut p = load("ex-weather", "weather_controller.lua");
    let mut g = Game::new();
    p.frame(&mut g);
    g.held = vec!["ControlLeft".into()];
    g.keys = vec![("KeyW".into(), true)];
    p.frame(&mut g);
    g.keys.clear();
    g.held.clear();
    assert_eq!(panel_ids(&p), ["weather"]);
    let owner = p.ui.borrow().panels()[0].owner;
    {
        let mut ui = p.ui.borrow_mut();
        ui.set_control(owner, "weather", "temperature", 0.0);
        ui.set_control(owner, "weather", "precipitation", 0.9);
        ui.click(owner, "weather", Some("morning"));
        ui.click(owner, "weather", Some("cycle"));
    }
    p.frame(&mut g);
    no_errors(&g);
    assert_eq!(g.weather.temperature, -20.0);
    assert_eq!(g.weather.precip_kind, 2);
    assert_eq!(g.clock.time, 7.0 * 3600.0);
    // the day cycle: at five the fog comes
    g.clock.time = 5.0 * 3600.0;
    p.frame(&mut g);
    p.frame(&mut g);
    no_errors(&g);
    assert_eq!(g.weather.precip_kind, 0);
    p.ui.borrow_mut().click(owner, "weather", Some("close"));
    p.frame(&mut g);
    assert!(panel_ids(&p).is_empty());
}

#[test]
fn the_trip_panel_runs_too() {
    let mut p = load("ex-trip", "trip_panel.lua");
    let mut g = Game::new();
    for _ in 0..3 {
        p.frame(&mut g);
    }
    no_errors(&g);
    assert_eq!(panel_ids(&p), ["trip"]);
}
