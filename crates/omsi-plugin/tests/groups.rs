//! Every group of the API through Lua, against the fake game: what each function reads and
//! what it asks the game to do.
mod common;

use common::{run, Game};
use omsi_plugin::{AiCar, InfoValue, Light};

/// Run `body` (Lua) in a plugin's first frame; it reports with `omsi.message`.
fn check(tag: &str, body: &str, g: &mut Game) -> Vec<String> {
    let src = format!("local done = false\nfunction on_frame()\nif done then return end\ndone = true\n{body}\nend");
    run(tag, &src, g, 1);
    std::mem::take(&mut g.messages)
}

#[test]
fn the_bus() {
    let mut g = Game::new();
    let out = check(
        "bus",
        r##"
        local s = omsi.bus.state()
        assert(s.speed == 0 and s.gear == 1 and s.passengers == 3 and s.handbrake == true and s.fuel == 180, "state")
        local vx, vy = omsi.bus.velocity_vector(); assert(vx == 1 and vy == 2)
        local across, along = omsi.bus.acceleration(); assert(along == -1.5)
        assert(select("#", omsi.bus.orientation()) == 3)
        assert(omsi.bus.mass() == 11500 and omsi.bus.odometer() > 1000 and omsi.bus.km_today() == 12.5)
        assert(#omsi.bus.doors() == 2 and omsi.bus.door(1) == false and omsi.bus.doors_open() == false and omsi.bus.door_count() == 2)
        assert(omsi.bus.toggle_door(1))
        assert(omsi.bus.indicator() == "off" and omsi.bus.set_indicator("left") and omsi.bus.indicator() == "left")
        assert(not pcall(omsi.bus.set_indicator, "up"))
        assert(omsi.bus.headlights() == 2 and omsi.bus.interior_light() == 1)
        assert(omsi.bus.set_interior_light(false))
        local running, rpm, elec = omsi.bus.engine(); assert(running == false and elec == true)
        assert(omsi.bus.start_up() == "Starting up" and omsi.bus.engine_running() and omsi.bus.rpm() == 750.5)
        assert(omsi.bus.shift(-1) and omsi.bus.gear() == -1)
        assert(omsi.bus.fuel() == 180 and omsi.bus.dirt() == 0.25)
        local d = omsi.bus.damage(); assert(d.crashes == 1 and d.last_impact_kj == 136 and d.repair_minutes == 30)
        local t, b, c, st = omsi.bus.controls(); assert(t == 0.5 and st == -0.25)
        assert(omsi.bus.steering_angle() == -10)
        assert(#omsi.bus.wheels() == 1 and omsi.bus.wheels()[1].radius == 0.5)
        assert(omsi.bus.horn() == false and omsi.bus.sound_horn(true) and omsi.bus.horn() == true)
        assert(omsi.bus.handbrake() == true and omsi.bus.toggle_handbrake())
        assert(omsi.bus.stop_brake() == nil and omsi.bus.kneeling() == nil)
        assert(omsi.bus.action("kw_scheinwerfer_toggle"))
        assert(omsi.bus.trailers() == 0)
        assert(#omsi.bus.destinations() == 2 and omsi.bus.destinations()[2].all_exit)
        local name, i = omsi.bus.destination(); assert(name == "Rathaus" and i == 1)
        assert(omsi.bus.set_destination(2) and not omsi.bus.set_destination(0))
        assert(omsi.bus.set_line("136"))
        assert(omsi.bus.number() == "2711" and omsi.bus.file():find("SD202"))
        local all, seated, standing = omsi.bus.passengers(); assert(all == 3 and seated == 2 and standing == 1)
        assert(omsi.bus.tickets()[1].price == 2.1)
        local tn, tp = omsi.bus.ticket_request(); assert(tn == "Einzelfahrschein")
        local sold, money = omsi.bus.sales(); assert(sold == 4 and money == 8.4)
        assert(omsi.bus.triggers()[1] == "bus_doorfront0")
        assert(omsi.bus.play_sound("ev_horn"))
        local vars = omsi.bus.get_vars({ "Velocity", "nope" }); assert(vars.Velocity == 0 and vars.nope == nil)
        assert(omsi.bus.get_strings().IBIS_terminus_name == "Rathaus")
        assert(omsi.bus.set_vars({ Velocity = 12, nope = 1 }) == 1 and omsi.var("Velocity") == 12)
        omsi.message("ok")
        "##,
        &mut g,
    );
    assert_eq!(out, ["ok"]);
    assert!(g.log.borrow().iter().any(|l| l == "door 1"));
    assert!(g.log.borrow().iter().any(|l| l == "destination 1"));
    assert!(g.log.borrow().iter().any(|l| l == "sound ev_horn"));
}

#[test]
fn duty_and_timetable() {
    let mut g = Game::new();
    let out = check(
        "duty",
        r##"
        assert(omsi.duty.active())
        local d = omsi.duty.get()
        assert(d.line == "136" and d.trip == 1 and d.trips == 4 and d.next_stop.name == "Markt" and d.previous_stop.name == "Zoo" and d.delay == 30)
        assert(omsi.duty.delay() == 30 and not omsi.duty.at_stop())
        local stops = omsi.duty.stops(); assert(#stops == 3 and stops[1].passed and not stops[2].passed and stops[3].x == 900)
        assert(omsi.duty.next_stop().id == 12)
        assert(#omsi.duty.trips() == 1 and omsi.duty.trip_stops(1)[3].name == "Rathaus" and omsi.duty.trip_stops(9) == nil)
        assert(omsi.duty.skip_stop() == "Markt" and omsi.duty.next_stop().name == "Rathaus")
        assert(omsi.duty.skip_to(1) and not omsi.duty.skip_to(10))
        assert(omsi.duty.start("136", "3", 2, 1))
        local ok, why = omsi.duty.start("999", "1"); assert(not ok and why:find("999"))
        local l = omsi.timetable.lines()[1]; assert(l.name == "136" and l.tours[1].today)
        assert(#omsi.timetable.stops("136", "3") == 2 and omsi.timetable.stops("136", "3", 2)[1].trip == 2)
        assert(omsi.timetable.stop_names()[1].id == 11)
        assert(omsi.timetable.buses()[1].delay == 60)
        assert(omsi.duty.finish() and not omsi.duty.active())
        omsi.message("ok")
        "##,
        &mut g,
    );
    assert_eq!(out, ["ok"]);
    assert!(g.log.borrow().iter().any(|l| l == "duty 136/3 1 0"));
}

#[test]
fn the_map() {
    let mut g = Game::new();
    let out = check(
        "map",
        r##"
        assert(omsi.map.name() == "Spandau" and omsi.map.info().tile_size == 300)
        assert(omsi.map.tiles()[1].loaded)
        assert(omsi.map.stops()[1].name == "Zoo")
        local x, y, z, h = omsi.map.object(11); assert(h == 90 and omsi.map.object(5) == nil)
        local near = omsi.map.objects_near(0, 0, 50); assert(#near == 2 and near[1].id == 11 and near[2].distance == 10)
        assert(omsi.map.ground(1, 1) == 2.5 and omsi.map.terrain(1, 1) == 2)
        assert(omsi.map.lane(0, 0).speed_limit == 50 and omsi.map.speed_limit() == 50)
        assert(omsi.map.entrypoints()[1].name == "Depot" and omsi.map.entrypoints()[1].index == 1)
        assert(omsi.map.teleport(10, 20, nil, 45) and omsi.position() == 10)
        assert(omsi.map.teleport_to(1) and not omsi.map.teleport_to(2))
        assert(omsi.map.place_on_road(1, 2))
        omsi.message("ok")
        "##,
        &mut g,
    );
    assert_eq!(out, ["ok"]);
}

#[test]
fn traffic_and_people() {
    let mut g = Game::new();
    let out = check(
        "traffic",
        r##"
        local all = omsi.traffic.list(); assert(#all == 2 and all[1].kind == "car" and all[2].line == "137")
        assert(#omsi.traffic.list(100) == 1 and omsi.traffic.list(100)[1].distance == 30)
        assert(omsi.traffic.get(9).kind == "timetable_bus" and omsi.traffic.get(1) == nil)
        assert(omsi.traffic.nearest().id == 7 and omsi.traffic.nearest("timetable_bus").id == 9)
        assert(omsi.traffic.ahead(100).id == 7)
        local driving, buses = omsi.traffic.counts(); assert(driving == 2 and buses == 1)
        assert(omsi.traffic.density() == 30 and omsi.traffic.set_density(50, 0.5))
        assert(omsi.traffic.remove(7) and not omsi.traffic.remove(7) and omsi.traffic.clear() == 1)
        assert(omsi.traffic.light_ahead() == nil)
        local walking, waiting, riding = omsi.people.counts(); assert(walking == 20 and riding == 3)
        assert(#omsi.people.list() == 2 and #omsi.people.list(10) == 1 and omsi.people.list(10)[1].state == "seated")
        assert(omsi.people.stops()[1].waiting == 5 and omsi.people.waiting(13) == 5 and omsi.people.waiting(1) == 0)
        assert(omsi.people.density() == 1 and omsi.people.set_density(2))
        local others = omsi.others(1000); assert(#others == 0)
        omsi.message("ok")
        "##,
        &mut g,
    );
    assert_eq!(out, ["ok"]);
    assert!(g.log.borrow().iter().any(|l| l == "density Some(50) Some(0.5)"));
}

#[test]
fn time_and_weather() {
    let mut g = Game::new();
    let out = check(
        "world",
        r##"
        assert(omsi.world.time() == 43200 and omsi.world.date().weekday == 4 and omsi.world.date().month == 10)
        assert(omsi.world.play_time() == 10)
        assert(omsi.world.set_time("07:30") and omsi.world.time() == 27000)
        assert(omsi.world.set_time(90000) and omsi.world.time() == 3600)
        assert(not pcall(omsi.world.set_time, "noon"))
        assert(omsi.world.set_date(2027, 1, 2) and omsi.world.date().year == 2027)
        assert(omsi.world.time_speed() == 1 and omsi.world.set_time_speed(100))
        assert(not omsi.world.paused())
        local folder, snow = omsi.world.season(); assert(folder == nil and snow == false)
        assert(omsi.world.sun_altitude() == 35)
        local w = omsi.weather.get(); assert(w.name == "Sunny" and w.precipitation == "none" and w.visibility == 20000)
        assert(omsi.weather.temperature() == 18 and omsi.weather.visibility() == 20000)
        assert(select("#", omsi.weather.wind()) == 2 and omsi.weather.precipitation() == "none")
        assert(omsi.weather.set({ temperature = -5, precipitation = "snow" }))
        assert(omsi.weather.temperature() == -5 and omsi.weather.precipitation() == "snow")
        assert(not pcall(omsi.weather.set, { precipitation = "hail" }))
        assert(omsi.weather.presets()[1].name == "Rain")
        assert(omsi.weather.preset("Weather/Rain.owt") and omsi.weather.get().name == "Weather/Rain.owt")
        assert(omsi.world.pause() and omsi.world.paused())
        omsi.message("ok")
        "##,
        &mut g,
    );
    assert_eq!(out, ["ok"]);
    assert!(g.log.borrow().iter().any(|l| l == "time speed 30"));
}

#[test]
fn camera_input_and_sound() {
    let mut g = Game::new();
    g.held = vec!["ShiftLeft".into()];
    let d = common::dir("media-files");
    let out = {
        let src = r##"
        local done = false
        function on_frame()
          if done then return end
          done = true
          local c = omsi.camera.get(); assert(c.view == "driver" and c.fov == 60 and c.width == 1920)
          assert(omsi.camera.view() == "driver" and select("#", omsi.camera.position()) == 3)
          assert(omsi.camera.set_view("outside") and omsi.camera.view() == "outside")
          assert(omsi.camera.set_free(0, 0, 10, 90, -10) and omsi.camera.view() == "free")
          assert(omsi.camera.set_zoom(2) and omsi.camera.look(10, 5))
          -- a point straight ahead of the camera (yaw 0: north) is in the middle of the screen
          local sx, sy = omsi.camera.project(0, 100, 2); local w, h = omsi.ui.screen()
          assert(math.abs(sx - w / 2) < 0.01 and math.abs(sy - h / 2) < 0.01, sx .. " " .. sy)
          assert(omsi.camera.project(0, -100, 2) == nil)
          assert(omsi.input.key_down("ShiftLeft") and not omsi.input.key_down("KeyA"))
          assert(omsi.input.keys_down()[1] == "ShiftLeft")
          local mx, my, left = omsi.input.mouse(); assert(mx == 200 and left == true)
          assert(omsi.input.controllers()[1].axes[2] == 1)
          assert(omsi.input.bindings()[1].action == "sim_pause" and omsi.input.bindings(true)[1].action == "horn")
          assert(not pcall(omsi.input.hotkey, "Ctrl+", function() end))
          omsi.input.hotkey("Shift+KeyH", function(k) omsi.message("hotkey " .. k) end)
          local id = omsi.audio.play("ding.wav", { volume = 0.5, on_bus = true })
          assert(id and omsi.audio.playing(id) and omsi.audio.set(id, { volume = 1 }))
          local none, why = omsi.audio.play("../ding.wav"); assert(none == nil and why)
          local none2, why2 = omsi.audio.play("missing.wav"); assert(none2 == nil and why2:find("no file"))
          assert(omsi.audio.stop(id) and not omsi.audio.stop(id))
          assert(omsi.audio.volume() == 0.8)
          omsi.message("ok")
        end
        "##;
        std::fs::create_dir_all(d.join("media")).unwrap();
        std::fs::write(d.join("media/main.lua"), src).unwrap();
        std::fs::write(d.join("media/ding.wav"), b"RIFF").unwrap();
        let mut p = omsi_plugin::Plugins::load(&[d.clone()], &omsi_plugin::HostConfig::default());
        p.frame(&mut g);
        g.keys = vec![("KeyH".into(), true)];
        p.frame(&mut g);
        g.held.clear();
        p.frame(&mut g);
        std::mem::take(&mut g.messages)
    };
    assert_eq!(out, ["ok", "hotkey KeyH"]);
    let sounds = g.sounds.borrow();
    assert_eq!(sounds.len(), 1);
    assert!(sounds[0].0.ends_with("media/ding.wav") && sounds[0].1.on_bus && sounds[0].1.volume == 0.5);
}

#[test]
fn the_game_lan_and_panels() {
    let mut g = Game::new();
    g.info = vec![("version", InfoValue::Text("0.2.22".into()))];
    let out = check(
        "game",
        r##"
        assert(omsi.game.version() == "0.2.22" and omsi.game.api() == 1)
        assert(omsi.game.has("weather.set") and omsi.game.has("omsi.bus.state") and not omsi.game.has("nope"))
        assert(type(omsi.game.platform()) == "string")
        assert(omsi.game.settings().units == "metric" and omsi.game.stats().km == 12.5)
        assert(omsi.game.fps() == 60 and not omsi.game.menu_open())
        assert(omsi.game.screenshot():find("png"))
        assert(omsi.game.notify("Hello", "warning") and not pcall(omsi.game.notify, "x", "loud"))
        assert(omsi.game.action("view_set_map"))
        assert(omsi.lan.active())
        local id, name, host = omsi.lan.me(); assert(id == 1 and name == "Host" and host)
        assert(omsi.lan.players()[1].name == "Anna")
        assert(omsi.lan.send(2, "hello"))
        assert(omsi.lan.send(0, "all"))
        local ok, why = omsi.lan.send(2, string.rep("x", 121)); assert(not ok and why:find("120"))
        assert(not omsi.lan.send(2, "a|b"))
        assert(omsi.lan.chat("hi"))
        assert(omsi.ui.set("p", { children = {
          { type = "text", id = "t", text = "a" },
          { type = "slider", id = "s", value = 3, min = 0, max = 10 },
          { type = "chart", id = "c", values = { 1, 2 } },
        } }))
        assert(omsi.ui.update("p", "t", { text = "b" }) and omsi.ui.update("p", "c", { values = { 3, 4, 5 } }))
        local ok2, why2 = omsi.ui.update("p", "zz", {}); assert(not ok2 and why2:find("zz"))
        assert(omsi.ui.value("p", "s") == 3)
        assert(omsi.ui.show("p", false) and omsi.ui.toggle("p") and omsi.ui.panels()[1] == "p")
        local dx, dy = omsi.ui.moved("p"); assert(dx == 0 and dy == 0)
        assert(not omsi.ui.typing())
        omsi.message("ok")
        "##,
        &mut g,
    );
    assert_eq!(out, ["ok"]);
    assert_eq!(g.lan_sent, [("game".to_string(), 2, "hello".to_string()), ("game".into(), 2, "all".into())]);
    assert!(g.log.borrow().iter().any(|l| l == "notify 1 Hello"));
}

#[test]
fn helpers() {
    let mut g = Game::new();
    let out = check(
        "util",
        r##"
        assert(omsi.vec.length(3, 4) == 5 and omsi.vec.distance(0, 0, 3, 4) == 5 and omsi.vec.distance3(0, 0, 0, 1, 2, 2) == 3)
        assert(omsi.vec.heading(1, 0) == 90 and omsi.vec.bearing(0, 0, 0, -1) == 180)
        assert(omsi.vec.angle_diff(350, 10) == 20 and omsi.vec.angle_diff(10, 350) == -20)
        local x, y = omsi.vec.rotate(0, 1, 90); assert(math.abs(x - 1) < 1e-9 and math.abs(y) < 1e-9)
        local nx, ny = omsi.vec.normalize(0, 5); assert(ny == 1)
        assert(omsi.vec.dot(1, 2, 3, 4) == 11 and omsi.vec.lerp(0, 10, 0.25) == 2.5 and omsi.vec.clamp(5, 0, 1) == 1)
        local right, ahead = omsi.vec.to_local(10, 0, 0, 0, 90); assert(math.abs(ahead - 10) < 1e-9 and math.abs(right) < 1e-9)
        assert(omsi.fmt.clock(3725) == "01:02" and omsi.fmt.clock(3725, true) == "01:02:05")
        assert(omsi.fmt.duration(65) == "1 min 05 s" and omsi.fmt.delay(150) == "+2:30" and omsi.fmt.delay(-45) == "-0:45")
        assert(omsi.fmt.number(1234567.5, 1, ",") == "1,234,567.5" and omsi.fmt.money(2.1, "EUR") == "2.10 EUR")
        assert(omsi.fmt.speed(100, "mph") == "62 mph" and omsi.fmt.distance(2400) == "2.4 km")
        assert(omsi.fmt.pad("ab", 4) == "ab  " and omsi.fmt.pad("ab", 4, true) == "  ab")
        assert(#omsi.fmt.split("a,b,,c") == 4 and omsi.fmt.trim("  x ") == "x")
        assert(omsi.util.now() > 1.7e9 and omsi.util.date(0).year == 1970 and omsi.util.date(0).weekday == 4)
        assert(omsi.util.ms() >= 0)
        local r = omsi.util.random(1, 6); assert(r >= 1 and r <= 6 and math.type(r) == "integer")
        local f = omsi.util.random(); assert(f >= 0 and f < 1)
        assert(omsi.json.encode({ 1, 2 }) == "[1,2]" and omsi.json.encode({ a = true }) == '{"a":true}')
        local v, why = omsi.json.decode("{"); assert(v == nil and why)
        assert(omsi.info_value("nope") == nil)
        omsi.message("ok")
        "##,
        &mut g,
    );
    assert_eq!(out, ["ok"]);
}

#[test]
fn derived_events() {
    let mut g = Game::new();
    let src = r##"
        local log = {}
        local function say(...) local t = {} for i = 1, select("#", ...) do t[i] = tostring(select(i, ...)) end log[#log + 1] = table.concat(t, " ") end
        for _, e in ipairs({ "door", "doors", "engine_start", "gear", "indicator", "horn", "passengers", "stop_arrive", "stop_depart", "trip_start", "duty_end", "minute", "hour", "day", "weather", "light_ahead", "red_light", "message", "resume" }) do
          omsi.on(e, function(...) say(e, ...) end)
        end
        function on_frame() if omsi.time() >= 3 then omsi.message(table.concat(log, "; ")) end end
    "##;
    let mut p = run("derived", src, &mut g, 1);
    g.doors = vec![1.0, 0.0];
    g.engine = true;
    g.gear = 2.0;
    g.indicator = 2;
    g.vars.insert("cockpit_hupe".into(), 1.0);
    g.passengers = 5;
    g.duty.as_mut().unwrap().at_stop = true;
    g.clock.time += 60.0;
    g.weather.precip_kind = 1;
    g.light = Some(Light { aspect: "red", change_in: 10.0, distance: 8.0 });
    g.vars.insert("Velocity".into(), 30.0);
    g.events_ex = vec![("resume", Vec::new())];
    p.frame(&mut g);
    g.events_ex.clear();
    g.duty.as_mut().unwrap().at_stop = false;
    g.duty.as_mut().unwrap().next = 2;
    g.light = None;
    g.clock.time += 3600.0;
    g.duty.as_mut().unwrap().current.name = "Back".into();
    p.frame(&mut g);
    g.duty = None;
    g.cars.push(AiCar { id: 1, ..Default::default() });
    for _ in 0..3 {
        p.frame(&mut g);
    }
    let msg = g.messages.last().cloned().unwrap_or_default();
    for want in [
        "resume",
        "door 1 true",
        "doors true",
        "engine_start",
        "gear 2 1",
        "indicator right off",
        "horn true",
        "passengers 5 3",
        "stop_arrive Markt 12 2 30.0",
        "minute 12 1",
        "weather Sunny",
        "light_ahead red 8.0",
        "stop_depart Markt 12 2 30.0",
        "red_light 30.0",
        "light_ahead nil nil",
        "hour 13",
        "trip_start 1 Back Rathaus",
        "duty_end 136 3",
    ] {
        assert!(msg.contains(want), "{want} is missing from: {msg}");
    }
}
