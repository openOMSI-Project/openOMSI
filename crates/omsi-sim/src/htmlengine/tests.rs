use super::*;

const DEMO: &str = include_str!("../../../../docs/examples/htmltexture/demo.html");

fn px(frame: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
}

#[test]
fn css_colours_and_sizes_are_painted() {
    let html = "<style>#a{width:10px;height:4px;background:#ff0000;margin:1px}.b{background:rgb(0,0,255);height:2px}</style><body style='margin:0'><div id=a></div><div class=b></div></body>";
    let mut r = EngineRenderer::new(20, 20, html);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 20, 5, 2), [255, 0, 0, 255]);
    assert_eq!(px(&f, 20, 15, 2), [0, 0, 0, 0]);
    assert_eq!(px(&f, 20, 3, 7), [0, 0, 255, 255]);
}

#[test]
fn scripts_write_variables_back() {
    let html = "<body><script>window.omsi = window.omsi || {}; window.omsi.update = function (d) { if (d.num.door > 0) omsi.setVar('lamp', 1); else window.omsi.setVar('lamp', 0); };</script></body>";
    let mut r = EngineRenderer::new(8, 8, html);
    r.set_vars(&[("door".into(), 1.0)], &[]);
    assert_eq!(r.take_events(), vec![("lamp".to_string(), 1.0)]);
    r.set_vars(&[("door".into(), 0.0)], &[]);
    assert_eq!(r.take_events(), vec![("lamp".to_string(), 0.0)]);
}

#[test]
fn javascript_basics() {
    let html = "<body><p id=o></p><script>var t=0; for (var i=1;i<=4;i++){ if (i==3) continue; t+=i*2; } var f=(a,b)=>a>b?a:b; var o={x:[1,2,3]}; document.getElementById('o').textContent = t + ':' + f(3,9) + ':' + o.x.length + ':' + (7.126).toFixed(2) + ':' + 'ab'.padStart(4,'-') + ':' + Math.max(1,5,2) + ':' + typeof o;</script></body>";
    let r = EngineRenderer::new(8, 8, html);
    assert_eq!(r.text_of("o").as_deref(), Some("14:9:3:7.13:--ab:5:object"));
}

#[test]
fn a_broken_script_does_not_stop_the_page() {
    let html = "<body style='background:#00ff00'><script>this is not javascript (</script></body>";
    let mut r = EngineRenderer::new(4, 4, html);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 4, 1, 1), [0, 255, 0, 255]);
}

#[test]
fn an_endless_loop_is_cut_off() {
    let html = "<body><script>while (true) {}</script></body>";
    let mut r = EngineRenderer::new(4, 4, html);
    assert!(r.poll_frame().is_some());
}

// ─────────────── operating the page ───────────────

fn click(r: &mut EngineRenderer, x: f32, y: f32) {
    r.pointer(x, y, PointerKind::Down);
    r.pointer(x, y, PointerKind::Up);
}

const KEYPAD: &str = "<style>body{margin:0;width:200px;height:100px;background:#000}\
    button{width:40px;height:20px;margin:0;padding:0}</style><body>\
    <button id=a onclick=\"omsi.setVar('key', 1)\">A</button><button id=b>B</button><button id=c>C</button>\
    <script>document.getElementById('b').addEventListener('click', function (e) { omsi.setVar('key', 2); });\
    document.getElementById('c').onclick = () => omsi.setVar('key', 3);</script></body>";

#[test]
fn buttons_sit_in_a_row_and_take_clicks() {
    let mut r = EngineRenderer::new(200, 100, KEYPAD);
    // three 40px buttons side by side: x 0..40, 40..80, 80..120, all in the first row
    click(&mut r, 20.0, 10.0);
    assert_eq!(r.take_events(), vec![("key".to_string(), 1.0)]);
    click(&mut r, 60.0, 10.0);
    assert_eq!(r.take_events(), vec![("key".to_string(), 2.0)]);
    click(&mut r, 100.0, 10.0);
    assert_eq!(r.take_events(), vec![("key".to_string(), 3.0)]);
    // beside the buttons and below them nothing happens
    click(&mut r, 150.0, 10.0);
    click(&mut r, 20.0, 60.0);
    assert!(r.take_events().is_empty());
}

#[test]
fn a_click_needs_a_press_and_a_release() {
    let mut r = EngineRenderer::new(200, 100, KEYPAD);
    r.pointer(20.0, 10.0, PointerKind::Up);
    assert!(r.take_events().is_empty(), "a release alone is no click");
    r.pointer(20.0, 10.0, PointerKind::Down);
    assert!(r.take_events().is_empty(), "a press alone is no click");
    r.pointer(20.0, 10.0, PointerKind::Up);
    assert_eq!(r.take_events().len(), 1);
}

#[test]
fn inline_blocks_wrap_and_shrink_to_fit() {
    let html = "<style>body{margin:0;width:100px;height:100px}.k{display:inline-block;width:30px;height:10px;margin:0}</style>\
        <body><div class=k onclick=\"omsi.setVar('k',1)\"></div><div class=k onclick=\"omsi.setVar('k',2)\"></div>\
        <div class=k onclick=\"omsi.setVar('k',3)\"></div><div class=k onclick=\"omsi.setVar('k',4)\"></div></body>";
    let mut r = EngineRenderer::new(100, 100, html);
    // 3 fit into 100px, the fourth wraps to the second row
    click(&mut r, 75.0, 5.0);
    assert_eq!(r.take_events(), vec![("k".to_string(), 3.0)]);
    click(&mut r, 5.0, 15.0);
    assert_eq!(r.take_events(), vec![("k".to_string(), 4.0)]);
    // a button without a width is as wide as its label
    let plain = "<body style='margin:0'><button id=x onclick=\"omsi.setVar('x',1)\" style='padding:0'>Hi</button></body>";
    let mut r = EngineRenderer::new(200, 50, plain);
    click(&mut r, 190.0, 5.0);
    assert!(r.take_events().is_empty(), "the button does not fill the row");
    click(&mut r, 3.0, 5.0);
    assert_eq!(r.take_events().len(), 1);
}

#[test]
fn clicks_bubble_and_can_be_stopped() {
    let html = "<body style='margin:0'><div id=outer onclick=\"omsi.setVar('outer',1)\" style='height:50px'>\
        <div id=inner style='height:20px'></div></div><script>\
        document.getElementById('inner').addEventListener('click', function (e) { omsi.setVar('inner', 1); if (e.x > 100) e.stopPropagation(); });</script></body>";
    let mut r = EngineRenderer::new(200, 100, html);
    click(&mut r, 10.0, 5.0);
    let ev = r.take_events();
    assert_eq!(ev, vec![("inner".to_string(), 1.0), ("outer".to_string(), 1.0)]);
    click(&mut r, 150.0, 5.0);
    assert_eq!(r.take_events(), vec![("inner".to_string(), 1.0)]);
    click(&mut r, 10.0, 40.0);
    assert_eq!(r.take_events(), vec![("outer".to_string(), 1.0)]);
}

#[test]
fn a_span_in_text_is_hit_on_its_own() {
    let html = "<body style='margin:0;font-size:20px'>Go <span id=s onclick=\"omsi.setVar('s',1)\">here</span> now</body>";
    let mut r = EngineRenderer::new(300, 40, html);
    click(&mut r, 2.0, 10.0);
    assert!(r.take_events().is_empty());
    click(&mut r, 42.0, 10.0);
    assert_eq!(r.take_events(), vec![("s".to_string(), 1.0)]);
}

#[test]
fn a_page_reacts_to_its_own_clicks() {
    // a route picker: the list is built by script, a click selects and writes the choice back
    let html = "<style>body{margin:0}.row{height:20px}.sel{background:#00ff00}</style><body><div id=list></div><script>\
        var routes = ['A', 'B', 'C']; var chosen = -1;\
        function draw() { var l = document.getElementById('list'); l.innerHTML = '';\
          routes.forEach(function (n, i) { var d = document.createElement('div'); d.className = 'row' + (i == chosen ? ' sel' : '');\
            d.textContent = n; d.addEventListener('click', function () { chosen = i; omsi.setVar('route', i); draw(); }); l.appendChild(d); }); }\
        draw();</script></body>";
    let mut r = EngineRenderer::new(50, 80, html);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 50, 45, 30), [0, 0, 0, 0]);
    click(&mut r, 10.0, 30.0); // the second row
    assert_eq!(r.take_events(), vec![("route".to_string(), 1.0)]);
    let f = r.poll_frame().expect("the click changed the page");
    assert_eq!(px(&f, 50, 45, 30), [0, 255, 0, 255]);
    assert_eq!(px(&f, 50, 45, 10), [0, 0, 0, 0]);
}

#[test]
fn class_list_and_inner_html() {
    let html = "<style>.on{background:#ff0000}</style><body style='margin:0'><div id=a style='height:4px'></div><div id=b></div><script>\
        var a = document.getElementById('a'); a.classList.add('x'); a.classList.toggle('on'); a.classList.toggle('x');\
        var t = a.classList.contains('on') + ',' + a.classList.contains('x');\
        document.getElementById('b').innerHTML = '<p id=p>hi <b>there</b></p>';\
        document.getElementById('b').setAttribute('id', 'b2'); window.t = t;</script></body>";
    let mut r = EngineRenderer::new(20, 20, html);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 20, 5, 2), [255, 0, 0, 255]);
    assert_eq!(r.text_of("p").as_deref(), Some("hi there"));
}

#[test]
fn timers_fire_when_due_and_can_be_cleared() {
    let html = "<body><p id=o></p><script>var n = 0; var once = 0; var id = setInterval(function () { n++; document.getElementById('o').textContent = 'n' + n;\
        if (n == 3) clearInterval(id); }, 100); setTimeout(function () { once++; omsi.setVar('once', once); }, 250);</script></body>";
    let mut r = EngineRenderer::new(8, 8, html);
    r.js.now = 0.05;
    assert!(!r.run_timers());
    r.js.now = 0.11;
    assert!(r.run_timers());
    assert_eq!(r.text_of("o").as_deref(), Some("n1"));
    r.js.now = 0.26;
    assert!(r.run_timers());
    assert_eq!(r.text_of("o").as_deref(), Some("n2"));
    assert_eq!(r.take_events(), vec![("once".to_string(), 1.0)]);
    r.js.now = 0.40;
    r.run_timers();
    assert_eq!(r.text_of("o").as_deref(), Some("n3"));
    r.js.now = 5.0;
    assert!(!r.run_timers(), "cleared, and the timeout only ran once");
}

#[test]
fn arrays_objects_and_strings() {
    let html = "<body><p id=o></p><script>var a = [3, 1, 2]; var b = a.map(function (x) { return x * 2; }).filter(x => x > 2);\
        var o = {z: 1, a: 2}; var s = 'a-b-c'.split('-'); var sum = 0; a.forEach(function (x, i) { sum += x * i; });\
        document.getElementById('o').textContent = b.join('') + '|' + Object.keys(o).join('') + '|' + s.length + s[2] + '|' + sum + '|'\
          + a.indexOf(2) + a.includes(9) + '|' + a.slice(1).join('') + '|' + 'x.y'.replace('.', '+') + '|' + a.pop() + a.length;</script></body>";
    let r = EngineRenderer::new(8, 8, html);
    assert_eq!(r.text_of("o").as_deref(), Some("64|az|3c|5|2false|12|x+y|22"));
}

#[test]
fn a_failing_callback_or_handler_does_not_stop_the_page() {
    let html = "<body style='margin:0'><div id=d style='height:10px' onclick=\"nope.x()\"></div><div id=e style='height:10px' onclick=\"omsi.setVar('ok', 1)\"></div>\
        <script>[1].forEach(function () { missing.call(); }); document.getElementById('d').addEventListener('click', function () { throw_it(); });</script></body>";
    let mut r = EngineRenderer::new(40, 40, html);
    click(&mut r, 5.0, 5.0);
    click(&mut r, 5.0, 15.0);
    assert_eq!(r.take_events(), vec![("ok".to_string(), 1.0)]);
}

#[test]
fn a_click_can_press_a_trigger() {
    let html = "<body style='margin:0'><button style='padding:0;width:30px;height:10px' onclick=\"omsi.trigger('door_1'); omsi.setVar('seen', 1)\">Door</button></body>";
    let mut r = EngineRenderer::new(60, 20, html);
    click(&mut r, 5.0, 5.0);
    assert_eq!(r.take_triggers(), vec!["door_1".to_string()]);
    assert_eq!(r.take_events(), vec![("seen".to_string(), 1.0)]);
    assert!(r.take_triggers().is_empty());
}

fn map(items: Vec<(&str, crate::vehicle_api::ApiValue)>) -> crate::vehicle_api::ApiValue {
    crate::vehicle_api::ApiValue::Map(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn sample_vehicle() -> crate::vehicle_api::ApiValue {
    use crate::vehicle_api::ApiValue as A;
    let door = |n: f64, open: f64| map(vec![("number", A::Num(n)), ("open", A::Num(open)), ("isOpen", A::Bool(open > 0.05))]);
    map(vec![
        ("info", map(vec![("number", A::Str("4711".into()))])),
        ("engine", map(vec![("running", A::Bool(true)), ("rpm", A::Null)])),
        ("doors", map(vec![("count", A::Num(2.0)), ("list", A::List(vec![door(1.0, 0.0), door(2.0, 1.0)]))])),
    ])
}

#[test]
fn a_page_reads_the_vehicle_object() {
    let html = "<body><p id=e></p><p id=d></p><p id=n></p><p id=r></p><script>\
        window.omsi = window.omsi || {};\
        window.omsi.update = function (d) {\
          var v = omsi.vehicle;\
          document.getElementById('e').textContent = v.engine.running ? 'on' : 'off';\
          document.getElementById('d').textContent = v.doors.list[1].isOpen + ':' + v.doors.count + ':' + v.doors.list[0].number;\
          document.getElementById('n').textContent = d.vehicle.info.number;\
          document.getElementById('r').textContent = v.engine.rpm == null ? 'none' : 'rpm';\
        };</script></body>";
    let mut r = EngineRenderer::new(40, 40, html);
    r.set_vehicle(&sample_vehicle());
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("e").as_deref(), Some("on"));
    assert_eq!(r.text_of("d").as_deref(), Some("true:2:1"));
    assert_eq!(r.text_of("n").as_deref(), Some("4711"));
    assert_eq!(r.text_of("r").as_deref(), Some("none"), "a signal the bus lacks is null");
}

#[test]
fn every_variable_can_be_read_by_name_in_any_letter_case() {
    let html = "<body><p id=a></p><p id=b></p><p id=c></p><p id=t></p><script>\
        window.omsi = window.omsi || {};\
        window.omsi.update = function (d) {\
          document.getElementById('a').textContent = omsi.getVar('Engine_N');\
          document.getElementById('b').textContent = omsi.vars.num.engine_n;\
          document.getElementById('c').textContent = omsi.getVar('IDENT') + '/' + typeof omsi.getVar('nope');\
          document.getElementById('t').textContent = d.vars.num.throttle;\
        };</script></body>";
    let mut r = EngineRenderer::new(40, 40, html);
    r.set_vars(
        &[("Engine_N".into(), 812.0), ("Throttle".into(), 0.5)],
        &[("ident".into(), "OMS-1".into())],
    );
    assert_eq!(r.text_of("a").as_deref(), Some("812"));
    assert_eq!(r.text_of("b").as_deref(), Some("812"));
    assert_eq!(r.text_of("c").as_deref(), Some("OMS-1/undefined"));
    assert_eq!(r.text_of("t").as_deref(), Some("0.5"));
}

#[test]
fn a_timer_sees_the_latest_vehicle_without_an_update_function() {
    let html = "<body><p id=e></p><script>\
        setInterval(function () {\
          document.getElementById('e').textContent = omsi.vehicle.doors ? omsi.vehicle.doors.count : 'none';\
        }, 10);</script></body>";
    let mut r = EngineRenderer::new(40, 40, html);
    r.set_vehicle(&sample_vehicle());
    r.set_vars(&[], &[]);
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert!(r.poll_frame().is_some());
    assert_eq!(r.text_of("e").as_deref(), Some("2"));
}

// ─────────────── demo.html: header, speed, lamps, doors, controls ───────────────

/// The vehicle object `demo.html` reads. `bare`: a bus without doors, boarding doors and
/// rpm signal (those come as `null` / empty).
fn demo_vehicle(speed: f64, engine_on: bool, door_open: bool, bare: bool) -> crate::vehicle_api::ApiValue {
    use crate::vehicle_api::ApiValue as A;
    let s = |t: &str| A::Str(t.to_string());
    let num = |items: Vec<(&str, f64)>| map(items.into_iter().map(|(k, v)| (k, A::Num(v))).collect());
    let flags = |items: Vec<(&str, bool)>| map(items.into_iter().map(|(k, v)| (k, A::Bool(v))).collect());
    let open = if door_open && !bare { 1.0 } else { 0.0 };
    let door = |n: f64, o: f64| map(vec![("number", A::Num(n)), ("open", A::Num(o)), ("isOpen", A::Bool(o > 0.05))]);
    let (count, doors, entries, exits) = if bare {
        (0.0, vec![], vec![], vec![])
    } else {
        (
            2.0,
            vec![door(1.0, open), door(2.0, 0.0)],
            vec![map(vec![("number", A::Num(1.0)), ("open", A::Bool(door_open)), ("requested", A::Bool(false))])],
            vec![map(vec![("number", A::Num(2.0)), ("open", A::Bool(false)), ("requested", A::Bool(true))])],
        )
    };
    map(vec![
        ("info", map(vec![("number", s("42")), ("ident", s("DEMO")), ("route", s("Line 7")), ("nextStop", s("Central Station"))])),
        (
            "motion",
            num(vec![
                ("speedKmh", speed),
                ("heading", 90.0),
                ("pitch", 0.4),
                ("bank", -0.2),
                ("steeringDeg", 5.0),
                ("x", 120.5),
                ("y", 33.2),
                ("z", 0.1),
                ("odometerKm", 1234.5),
            ]),
        ),
        (
            "engine",
            map(vec![
                ("running", A::Bool(engine_on)),
                ("rpm", if bare { A::Null } else { A::Num(800.0) }),
                ("gear", A::Num(3.0)),
                ("tankContent", A::Num(180.0)),
                ("throttle", A::Num(0.4)),
                ("brake", A::Num(0.0)),
                ("clutch", A::Num(0.0)),
            ]),
        ),
        ("electrics", flags(vec![("on", false), ("failure", false)])),
        ("battery", map(vec![("on", A::Null)])),
        ("brakes", flags(vec![("parking", false), ("stop", false), ("kneeling", false)])),
        ("wipers", flags(vec![("running", false)])),
        ("train", num(vec![("trailers", 0.0)])),
        (
            "lights",
            map(vec![
                ("headlights", A::Num(0.0)),
                ("brake", A::Bool(false)),
                ("reverse", A::Bool(false)),
                ("fog", A::Bool(false)),
                ("indicatorLeft", A::Bool(false)),
                ("indicatorRight", A::Bool(false)),
                ("hazard", A::Bool(false)),
                ("interior", A::Num(0.0)),
            ]),
        ),
        ("doors", map(vec![("count", A::Num(count)), ("anyOpen", A::Bool(open > 0.05)), ("list", A::List(doors))])),
        ("passengers", map(vec![("onboard", A::Num(23.0)), ("entries", A::List(entries)), ("exits", A::List(exits))])),
        ("cabin", num(vec![("temperature", 21.5)])),
        ("condition", num(vec![("dirt", 0.12), ("crashes", 0.0), ("lastImpactKJ", 0.0), ("streetCondition", 1.0)])),
    ])
}

/// A renderer with `demo.html` loaded and the vehicle handed over (as the game does on the
/// first update).
fn demo_page(v: &crate::vehicle_api::ApiValue) -> EngineRenderer {
    let mut r = EngineRenderer::new(800, 480, DEMO);
    r.set_vehicle(v);
    r.set_vars(&[], &[]);
    r
}

fn has_colour(frame: &[u8], rgb: [u8; 3]) -> bool {
    frame.chunks(4).any(|p| p[0] == rgb[0] && p[1] == rgb[1] && p[2] == rgb[2])
}

/// `.warn` (open door, brake held) and `.on` (engine lamp, open boarding door).
const RED: [u8; 3] = [0xe5, 0x38, 0x3b];
const GREEN: [u8; 3] = [0x1f, 0xa6, 0x4f];

fn inside(r: &EngineRenderer, mut n: usize, ancestor: usize) -> bool {
    loop {
        if n == ancestor {
            return true;
        }
        match r.js.dom.nodes[n].parent {
            Some(p) if p != n => n = p,
            _ => return false,
        }
    }
}

/// A point inside the element `id`, found the way a finger finds it: the first row at column
/// `x` where a press would reach the element (the page's height depends on what the script
/// has built, so no fixed coordinates).
fn point_in(r: &EngineRenderer, id: &str, x: f32) -> (f32, f32) {
    let node = r.js.dom.by_id(id).unwrap_or_else(|| panic!("no element {id}"));
    let mut y = 0.0;
    while y < 900.0 {
        if inside(r, r.hit_node(x, y), node) {
            return (x, y + 2.0);
        }
        y += 6.0;
    }
    panic!("{id} is not under column {x}");
}

fn click_on(r: &mut EngineRenderer, id: &str, x: f32) {
    let (x, y) = point_in(r, id, x);
    click(r, x, y);
}

// buttons of the demo: 170 px wide, 6 px apart, the row starts 10 px in
const X_STOP: f32 = 60.0;
const X_OPEN: f32 = 270.0;
const X_BRAKE: f32 = 450.0;

#[test]
fn the_demo_shows_the_header_and_the_speed() {
    let mut r = EngineRenderer::new(800, 480, DEMO);
    r.set_vehicle(&demo_vehicle(-36.6, true, false, false));
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("num").as_deref(), Some("Bus 42"));
    assert_eq!(r.text_of("ident").as_deref(), Some("DEMO"));
    assert_eq!(r.text_of("route").as_deref(), Some("Line 7"));
    assert_eq!(r.text_of("stop").as_deref(), Some("Central Station"));
    assert_eq!(r.text_of("kmh").as_deref(), Some("37"), "the speed is shown without its sign");
    assert_eq!(r.text_of("rpm").as_deref(), Some("800"));
    assert_eq!(r.text_of("gear").as_deref(), Some("3"));
    assert_eq!(r.text_of("tank").as_deref(), Some("180"));
    assert_eq!(r.text_of("thr").as_deref(), Some("0.40"));
    let a = r.poll_frame().unwrap();
    assert_eq!(a.len(), 800 * 480 * 4);
    assert_eq!(px(&a, 800, 799, 200), [0x09, 0x0e, 0x14, 255], "the page background");
    assert_eq!(px(&a, 800, 799, 10), [0x11, 0x19, 0x25, 255], "the header bar");
    r.set_vehicle(&demo_vehicle(7.0, true, false, false));
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("kmh").as_deref(), Some("7"));
    let b = r.poll_frame().unwrap();
    assert_ne!(a, b);
    assert!(r.poll_frame().is_none(), "no new frame without a change");
}

#[test]
fn the_demo_footer_counts_updates_and_reads_variables() {
    let mut r = EngineRenderer::new(800, 480, DEMO);
    r.set_vehicle(&demo_vehicle(0.0, true, false, false));
    r.set_vars(&[("Engine_N".into(), 812.0)], &[]);
    let foot = r.text_of("foot").unwrap();
    assert!(foot.contains("api 1 | updates 1 | vars 1 | Engine_N 812"), "{foot}");
    assert!(!foot.contains("DEMO |"), "the host is there, so it is no browser demo: {foot}");
    r.set_vars(&[("Engine_N".into(), 900.0)], &[]);
    let foot = r.text_of("foot").unwrap();
    assert!(foot.contains("updates 2") && foot.contains("Engine_N 900"), "{foot}");
}

#[test]
fn the_demo_lists_doors_and_boarding() {
    let r = demo_page(&demo_vehicle(0.0, true, true, false));
    let sum = r.text_of("doorsum").unwrap();
    assert!(sum.contains("2 door(s), any open: yes"), "{sum}");
    let pax = r.text_of("pax").unwrap();
    assert!(pax.contains("IN 1"), "{pax}");
    assert!(pax.contains("OUT 2 REQ"), "the requested exit is marked: {pax}");
    let doors = r.text_of("doors").unwrap();
    assert!(doors.contains("OPEN 100%") && doors.contains("shut"), "{doors}");
    let r = demo_page(&demo_vehicle(0.0, true, false, false));
    assert!(r.text_of("doorsum").unwrap().contains("any open: no"));
}

#[test]
fn the_demo_copes_with_a_bus_that_lacks_signals() {
    let r = demo_page(&demo_vehicle(0.0, false, false, true));
    assert_eq!(r.text_of("rpm").as_deref(), Some("--"), "a signal the bus lacks is null");
    assert!(r.text_of("doorsum").unwrap().contains("(none reported)"));
    assert_eq!(r.text_of("pax").as_deref(), Some("no boarding doors reported"));
    assert_eq!(r.text_of("kmh").as_deref(), Some("0"), "the rest of the page still updates");
}

#[test]
fn the_demo_draws_open_doors_and_the_running_engine() {
    let mut r = EngineRenderer::new(800, 480, DEMO);
    let idle = r.poll_frame().unwrap();
    assert!(!has_colour(&idle, RED) && !has_colour(&idle, GREEN), "everything is dark before the first update");
    r.set_vehicle(&demo_vehicle(0.0, true, false, false));
    r.set_vars(&[], &[]);
    let f = r.poll_frame().unwrap();
    assert!(has_colour(&f, GREEN), "the running engine is drawn green");
    assert!(!has_colour(&f, RED), "all doors shut: nothing red");
    r.set_vehicle(&demo_vehicle(0.0, false, true, false));
    r.set_vars(&[], &[]);
    let f = r.poll_frame().unwrap();
    assert!(has_colour(&f, RED), "the open door is drawn red");
}

#[test]
fn the_demo_stop_request_and_outside_opener_press_their_triggers() {
    let mut r = demo_page(&demo_vehicle(0.0, true, false, false));
    click_on(&mut r, "b_stop", X_STOP);
    assert_eq!(r.take_triggers(), vec!["door_haltewunsch".to_string(), "door_haltewunsch_off".to_string()]);
    assert!(r.text_of("foot").unwrap().contains("last: trigger door_haltewunsch"));
    assert!(r.take_events().is_empty(), "the buttons only press triggers");
    click_on(&mut r, "b_open", X_OPEN);
    assert_eq!(r.take_triggers(), vec!["door_aussenoeffner".to_string(), "door_aussenoeffner_off".to_string()]);
    assert!(r.text_of("foot").unwrap().contains("last: trigger door_aussenoeffner"));
    // beside the buttons nothing happens
    click(&mut r, 790.0, 5.0);
    assert!(r.take_triggers().is_empty() && r.take_events().is_empty());
}

#[test]
fn the_demo_brake_button_holds_while_pressed() {
    let mut r = demo_page(&demo_vehicle(0.0, false, false, false));
    let (x, y) = point_in(&r, "b_brake", X_BRAKE);
    r.pointer(x, y, PointerKind::Down);
    assert_eq!(r.take_events(), vec![("Brake".to_string(), 1.0)]);
    assert!(has_colour(&r.poll_frame().unwrap(), RED), "the held button turns red");
    assert!(r.text_of("foot").unwrap().contains("last: setVar Brake 1"));
    r.pointer(x, y, PointerKind::Up);
    assert_eq!(r.take_events(), vec![("Brake".to_string(), 0.0)], "released once, not twice");
    assert!(!has_colour(&r.poll_frame().unwrap(), RED));
    assert!(r.text_of("foot").unwrap().contains("last: setVar Brake 0"));
}

#[test]
fn the_demo_brake_lets_go_when_released_beside_the_button() {
    let mut r = demo_page(&demo_vehicle(0.0, false, false, false));
    let (x, y) = point_in(&r, "b_brake", X_BRAKE);
    r.pointer(x, y, PointerKind::Down);
    assert_eq!(r.take_events(), vec![("Brake".to_string(), 1.0)]);
    r.pointer(790.0, 5.0, PointerKind::Up);
    assert_eq!(r.take_events(), vec![("Brake".to_string(), 0.0)]);
    // a release without a hold sends nothing
    r.pointer(790.0, 5.0, PointerKind::Up);
    assert!(r.take_events().is_empty());
}

#[test]
fn the_demo_redraws_from_the_latest_snapshot_every_second() {
    let mut r = demo_page(&demo_vehicle(0.0, true, false, false));
    assert!(r.text_of("foot").unwrap().contains("| 0 s |"));
    // a new snapshot without an update call: the once-a-second timer picks it up
    let mut v = demo_vehicle(20.0, true, false, false);
    if let crate::vehicle_api::ApiValue::Map(m) = &mut v {
        for (k, val) in m.iter_mut() {
            if k.as_str() == "info" {
                *val = map(vec![
                    ("number", crate::vehicle_api::ApiValue::Str("42".into())),
                    ("ident", crate::vehicle_api::ApiValue::Str("DEMO".into())),
                    ("route", crate::vehicle_api::ApiValue::Str("Line 9".into())),
                    ("nextStop", crate::vehicle_api::ApiValue::Str("Depot".into())),
                ]);
            }
        }
    }
    r.set_vehicle(&v);
    r.js.now += 1.2;
    assert!(r.run_timers());
    assert_eq!(r.text_of("route").as_deref(), Some("Line 9"));
    assert_eq!(r.text_of("kmh").as_deref(), Some("20"));
    assert!(r.text_of("foot").unwrap().contains("| 1 s |"));
}

#[test]
fn the_demo_shows_a_real_vehicle_snapshot() {
    use crate::vehicle_api::{snapshot, Inputs};
    let var = |n: &str| match n.to_ascii_lowercase().as_str() {
        "door_0" => Some(0.0),
        "door_1" => Some(1.0),
        "engine_n" => Some(800.0),
        _ => None,
    };
    let text = |n: &str| match n {
        "number" => "4711".to_string(),
        "act_route" => "Line 7".to_string(),
        "act_busstop" => "Hauptbahnhof".to_string(),
        _ => String::new(),
    };
    let api = snapshot(&Inputs {
        var: &var,
        text: &text,
        speed_kmh: 36.6,
        steer_deg: 0.0,
        heading: 0.0,
        pitch: 0.0,
        bank: 0.0,
        position: (0.0, 0.0, 0.0),
        engine_running: true,
        interior_light: 0.0,
        crashes: 0,
        last_impact_j: 0.0,
        dirt: 0.0,
        trailers: 0,
    });
    let mut r = EngineRenderer::new(800, 480, DEMO);
    r.set_vehicle(&api);
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("kmh").as_deref(), Some("37"));
    assert_eq!(r.text_of("num").as_deref(), Some("Bus 4711"));
    assert_eq!(r.text_of("route").as_deref(), Some("Line 7"));
    assert_eq!(r.text_of("stop").as_deref(), Some("Hauptbahnhof"));
    let f = r.poll_frame().unwrap();
    assert!(has_colour(&f, RED), "the open door is drawn red");
    assert!(has_colour(&f, GREEN), "the running engine is drawn green");
}
#[test]
fn time_date_and_locale_reach_the_page() {
    use crate::vehicle_api::environment;
    let mut c = crate::SimClock::default();
    c.set_date(2026, 9, 30);
    c.time = 13.0 * 3600.0 + 5.0 * 60.0 + 9.0;
    let mut r = EngineRenderer::new(
        200,
        100,
        "<div id='a'></div><div id='b'></div><div id='c'></div><div id='d'></div>\
         <script>\
          omsi.update = function () {\
            var t = omsi.time, d = omsi.date;\
            document.getElementById('a').textContent = t.hour + ':' + t.minute + ':' + t.second + ' ' + t.asString;\
            document.getElementById('b').textContent = d.day + '.' + d.month + '.' + d.year + ' ' + d.asString;\
            document.getElementById('c').textContent = omsi.locale;\
          };\
         </script>",
    );
    r.set_env(&environment(&c, "de"));
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("a").as_deref(), Some("13:5:9 13:05:09"));
    assert_eq!(r.text_of("b").as_deref(), Some("30.9.2026 30.09.2026"));
    assert_eq!(r.text_of("c").as_deref(), Some("de"));
}

#[test]
fn a_scenery_page_has_only_the_basic_api() {
    let mut r = EngineRenderer::with_api(
        200,
        100,
        "<div id='a'></div><div id='b'></div>         <script>          document.getElementById('a').textContent = [typeof omsi.vehicle, typeof omsi.depot, typeof omsi.setRoute, typeof omsi.setNextStop].join(',');          omsi.update = function (d) {            document.getElementById('b').textContent = [typeof d.vehicle, typeof omsi.setVar, typeof omsi.trigger, typeof omsi.time, omsi.locale].join(',');          };         </script>",
        crate::htmltex::PageApi::Scenery,
    );
    r.set_env(&crate::vehicle_api::environment(&crate::SimClock::default(), "de"));
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("a").as_deref(), Some("undefined,undefined,undefined,undefined"));
    assert_eq!(r.text_of("b").as_deref(), Some("undefined,function,function,object,de"));
}

#[test]
fn the_english_date_is_month_first() {
    use crate::vehicle_api::{environment, ApiValue};
    let mut c = crate::SimClock::default();
    c.set_date(2026, 9, 30);
    let ApiValue::Map(m) = environment(&c, "en") else { panic!() };
    let ApiValue::Map(d) = &m.iter().find(|(k, _)| k == "date").unwrap().1 else { panic!() };
    assert_eq!(d.iter().find(|(k, _)| k == "asString").unwrap().1, ApiValue::Str("09/30/2026".into()));
}

#[test]
fn prefix_increment_and_decrement() {
    let mut r = EngineRenderer::new(
        200,
        100,
        "<div id='a'></div><div id='b'></div>\
         <script>\
          var S = { pt: 0 }, n = 5;\
          var t = ++S.pt;\
          ++S.pt;\
          document.getElementById('a').textContent = t + ',' + S.pt;\
          document.getElementById('b').textContent = (--n) + ',' + n;\
         </script>",
    );
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("a").as_deref(), Some("1,2"));
    assert_eq!(r.text_of("b").as_deref(), Some("4,4"));
}
// ─────────────── pictures ───────────────

/// A solid 24-bit BMP.
fn bmp(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let row = ((w * 3 + 3) / 4 * 4) as usize;
    let data = row * h as usize;
    let mut b = Vec::with_capacity(54 + data);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&((54 + data) as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(data as u32).to_le_bytes());
    b.extend_from_slice(&2835u32.to_le_bytes());
    b.extend_from_slice(&2835u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..h {
        for _ in 0..w {
            b.extend_from_slice(&[rgb[2], rgb[1], rgb[0]]);
        }
        b.resize(b.len() + row - (w * 3) as usize, 0);
    }
    b
}

/// A page in a fresh folder that holds `red.bmp` (4x4 red) and `blue.bmp` (2x2 blue).
fn picture_page(name: &str, html: &str, w: u32, h: u32) -> EngineRenderer {
    let dir = std::env::temp_dir().join(format!("omsi_htmlimg_{}_{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("red.bmp"), bmp(4, 4, [255, 0, 0])).unwrap();
    std::fs::write(dir.join("blue.bmp"), bmp(2, 2, [0, 0, 255])).unwrap();
    let mut r = EngineRenderer::new(w, h, html);
    r.set_asset_dirs(vec![dir]);
    r
}

#[test]
fn img_is_drawn_at_its_own_size() {
    let mut r = picture_page("own", "<body style='margin:0'><img src='red.bmp'></body>", 12, 12);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 12, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&f, 12, 3, 3), [255, 0, 0, 255]);
    assert_eq!(px(&f, 12, 4, 0), [0, 0, 0, 0]);
    assert_eq!(px(&f, 12, 0, 4), [0, 0, 0, 0]);
}

#[test]
fn img_takes_css_and_attribute_sizes() {
    let html = "<body style='margin:0'><div><img src='red.bmp' style='width:8px;height:6px'></div><div><img src='blue.bmp' width='4'></div></body>";
    let mut r = picture_page("size", html, 16, 16);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 16, 7, 5), [255, 0, 0, 255]);
    assert_eq!(px(&f, 16, 8, 0), [0, 0, 0, 0]);
    assert_eq!(px(&f, 16, 0, 10), [0, 0, 0, 0]);
    // the second picture: width 4 attribute, proportions kept (4x4)
    assert_eq!(px(&f, 16, 3, 6 + 3), [0, 0, 255, 255]);
    assert_eq!(px(&f, 16, 4, 6), [0, 0, 0, 0]);
}

#[test]
fn a_missing_picture_draws_nothing_and_does_not_stop_the_page() {
    let html = "<body style='margin:0;background:#00ff00'><img src='nope.png'><div id=t>x</div></body>";
    let mut r = picture_page("missing", html, 8, 8);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 8, 7, 0), [0, 255, 0, 255]);
    assert_eq!(r.text_of("t").as_deref(), Some("x"));
}

#[test]
fn background_image_tiles_by_default() {
    let html = "<body style='margin:0'><div style='width:10px;height:10px;background:url(red.bmp)'></div></body>";
    let mut r = picture_page("tile", html, 16, 16);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 16, 9, 9), [255, 0, 0, 255]);
    assert_eq!(px(&f, 16, 10, 5), [0, 0, 0, 0]);
}

#[test]
fn background_no_repeat_stays_once() {
    let html = "<body style='margin:0'><div style='width:10px;height:10px;background:url(red.bmp) no-repeat'></div></body>";
    let mut r = picture_page("norepeat", html, 16, 16);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 16, 3, 3), [255, 0, 0, 255]);
    assert_eq!(px(&f, 16, 6, 6), [0, 0, 0, 0]);
}

#[test]
fn background_size_and_position() {
    let cover = "<body style='margin:0'><div style='width:10px;height:6px;background-image:url(red.bmp);background-repeat:no-repeat;background-size:cover'></div></body>";
    let mut r = picture_page("cover", cover, 16, 16);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 16, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&f, 16, 9, 5), [255, 0, 0, 255]);

    let contain = "<body style='margin:0'><div style='width:10px;height:6px;background:url(red.bmp) no-repeat center / contain'></div></body>";
    let mut r = picture_page("contain", contain, 16, 16);
    let f = r.poll_frame().unwrap();
    // contain: 6x6, centred in 10 wide -> x 2..8
    assert_eq!(px(&f, 16, 1, 3), [0, 0, 0, 0]);
    assert_eq!(px(&f, 16, 2, 3), [255, 0, 0, 255]);
    assert_eq!(px(&f, 16, 7, 3), [255, 0, 0, 255]);
    assert_eq!(px(&f, 16, 8, 3), [0, 0, 0, 0]);

    let right = "<body style='margin:0'><div style='width:10px;height:4px;background:url(red.bmp) no-repeat right top'></div></body>";
    let mut r = picture_page("right", right, 16, 16);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 16, 5, 1), [0, 0, 0, 0]);
    assert_eq!(px(&f, 16, 6, 1), [255, 0, 0, 255]);
}

#[test]
fn background_image_of_the_body_covers_the_texture() {
    let html = "<body style='background:url(blue.bmp)'></body>";
    let mut r = picture_page("body", html, 9, 9);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 9, 0, 8), [0, 0, 255, 255]);
    assert_eq!(px(&f, 9, 8, 8), [0, 0, 255, 255]);
}

#[test]
fn background_image_is_clipped_to_rounded_corners() {
    let html = "<body style='margin:0'><div style='width:10px;height:10px;border-radius:5px;background:url(red.bmp)'></div></body>";
    let mut r = picture_page("round", html, 16, 16);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 16, 5, 5), [255, 0, 0, 255]);
    assert_eq!(px(&f, 16, 0, 0)[3], 0);
}

#[test]
fn scripts_can_set_the_source_of_a_picture() {
    let html = "<body style='margin:0'><img id=i><script>var e = document.getElementById('i'); e.src = 'blue.bmp'; e.setAttribute('width', '6');</script></body>";
    let mut r = picture_page("js", html, 8, 8);
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 8, 5, 5), [0, 0, 255, 255]);
    assert_eq!(px(&f, 8, 6, 0), [0, 0, 0, 0]);
}

#[test]
fn pictures_are_resized_once_and_kept() {
    let dir = std::env::temp_dir().join(format!("omsi_htmlimg_store_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("red.bmp"), bmp(4, 4, [255, 0, 0])).unwrap();
    let store = ImageStore::new(vec![dir]);
    assert_eq!(store.dims("red.bmp"), Some((4, 4)));
    let a = store.scaled("red.bmp", 9, 3).unwrap();
    let b = store.scaled("red.bmp", 9, 3).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!((a.w, a.h), (9, 3));
    assert_eq!(&a.rgba[..4], &[255u8, 0, 0, 255][..]);
    assert!(store.get("missing.png").is_none());
    assert!(store.get("http://example.com/a.png").is_none());
}

#[test]
fn background_values_are_split_and_read() {
    assert_eq!(split_top("url(a b.png) rgba(0, 0, 0, .5) center/cover"), vec!["url(a b.png)", "rgba(0, 0, 0, .5)", "center", "/", "cover"]);
    let (p, s, e) = find_url("#fff URL( 'img/a.png' ) no-repeat").unwrap();
    assert_eq!(p, "img/a.png");
    assert_eq!(&"#fff URL( 'img/a.png' ) no-repeat"[s..e], "URL( 'img/a.png' )");
    assert!(find_url("#fff").is_none());
}

#[test]
fn style_sheet_urls_are_made_relative_to_the_page() {
    use crate::htmltex::rebase_css_urls;
    let css = "a{background:url(../img/x.png)} b{background:url('y.png')} c{background:url(data:x)} d{background:url(/abs.png)}";
    assert_eq!(
        rebase_css_urls(css, "css/"),
        "a{background:url(css/../img/x.png)} b{background:url('css/y.png')} c{background:url(data:x)} d{background:url(/abs.png)}"
    );
    assert_eq!(rebase_css_urls(css, ""), css);
}

#[test]
fn linked_style_sheet_and_script_are_inlined_with_their_pictures() {
    let dir = std::env::temp_dir().join(format!("omsi_htmlimg_page_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("css")).unwrap();
    std::fs::create_dir_all(dir.join("img")).unwrap();
    std::fs::write(dir.join("img").join("red.bmp"), bmp(4, 4, [255, 0, 0])).unwrap();
    std::fs::write(dir.join("css").join("s.css"), "#a{width:6px;height:6px;background:url(../img/red.bmp)}").unwrap();
    std::fs::write(dir.join("index.html"), "<link rel='stylesheet' href='css/s.css'><body style='margin:0'><div id=a></div></body>").unwrap();
    let dirs = [dir.as_path()];
    let html = crate::htmltex::load_page(&dirs, "index.html");
    assert!(html.contains("url(css/../img/red.bmp)"));
    let mut r = EngineRenderer::new(8, 8, &html);
    r.set_asset_dirs(crate::htmltex::asset_dirs(&dirs, "index.html"));
    let f = r.poll_frame().unwrap();
    assert_eq!(px(&f, 8, 5, 5), [255, 0, 0, 255]);
}
/// Speed of the engine on `docs/examples/htmltexture/demo.html`. Not part of the normal test
/// run; start it in release mode (debug numbers mean nothing):
///
/// `cargo test -p omsi-sim --release --lib bench_htmlengine -- --ignored --nocapture`
///
/// Set `HTMLBENCH_PAGE=path/to/page.html` to measure your own page and `HTMLBENCH_RUNS=n` for
/// the number of timed runs (default 200).
#[test]
#[ignore]
fn bench_htmlengine() {
    use std::time::{Duration, Instant};

    fn stats(mut v: Vec<Duration>) -> String {
        v.sort();
        format!("median {:>9.3?}  min {:>9.3?}  p95 {:>9.3?}", v[v.len() / 2], v[0], v[v.len() * 95 / 100])
    }

    let page = std::env::var("HTMLBENCH_PAGE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/examples/htmltexture/demo.html").to_string());
    let runs: usize = std::env::var("HTMLBENCH_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    let html = std::fs::read_to_string(&page).unwrap();
    println!("page: {page} ({} bytes), {runs} runs", html.len());

    for &(w, h) in &[(512u32, 256u32), (1024, 512), (2048, 1024)] {
        let t = Instant::now();
        let mut r = EngineRenderer::new(w, h, &html);
        let parse = t.elapsed();
        let t = Instant::now();
        let _ = r.render();
        let cold = t.elapsed();
        for _ in 0..10 {
            let _ = r.render();
        }
        let render: Vec<_> = (0..runs).map(|_| { let t = Instant::now(); let _ = r.render(); t.elapsed() }).collect();
        let frame: Vec<_> = (0..runs).map(|i| { let t = Instant::now(); r.set_vars(&[("engine_n".to_string(), i as f32)], &[]); let _ = r.poll_frame(); t.elapsed() }).collect();
        let hit: Vec<_> = (0..runs).map(|_| { let t = Instant::now(); let _ = r.hit_node(100.0, 100.0); t.elapsed() }).collect();
        println!("--- {w}x{h}: parse + scripts {parse:?}, first render {cold:?}");
        println!("    render          {}", stats(render));
        println!("    update + frame  {}", stats(frame));
        println!("    pointer hit     {}", stats(hit));
    }
}
#[test]
fn hit_test_follows_layout_changes_and_idle_updates_draw_nothing() {
    let html = "<body style='margin:0'><div id=a style='height:10px'></div><div id=b style='height:10px'></div><script>window.omsi = window.omsi || {}; window.omsi.update = function () {}; function grow() { document.getElementById('a').style.height = '30px'; }</script></body>";
    let mut r = EngineRenderer::new(20, 40, html);
    let (a, b) = (r.js.dom.by_id("a").unwrap(), r.js.dom.by_id("b").unwrap());
    assert_eq!(r.hit_node(5.0, 15.0), b);
    assert!(r.poll_frame().is_some());
    r.set_vars(&[("x".to_string(), 1.0)], &[]);
    assert!(r.poll_frame().is_none(), "an update that changes nothing draws nothing");
    r.js.run("grow()").unwrap();
    assert_eq!(r.hit_node(5.0, 15.0), a, "the cached layout is dropped when the page changes");
    assert!(r.poll_frame().is_some());
}

#[test]
fn departures_are_asked_by_stop_and_handed_back() {
    use crate::vehicle_api::ApiValue;
    let html = "<body><p id=o></p><script>window.omsi.update = function (d) { var l = omsi.getDepartures('Central '); document.getElementById('o').textContent = l.length + ':' + (l.length ? l[0].line + '|' + l[0].destination + '|' + l[0].time : ''); };</script></body>";
    let mut r = EngineRenderer::new(8, 8, html);
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("o").as_deref(), Some("0:"));
    assert_eq!(r.take_departure_wants(), vec!["central".to_string()]);
    let entry = ApiValue::Map(vec![
        ("line".into(), ApiValue::Str("5".into())),
        ("destination".into(), ApiValue::Str("Hbf".into())),
        ("time".into(), ApiValue::Num(1000.0)),
    ]);
    r.set_departures(&ApiValue::Map(vec![("central".into(), ApiValue::List(vec![entry]))]));
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("o").as_deref(), Some("1:5|Hbf|1000"));
}
#[test]
fn hangul_is_drawn_with_the_fallback_face() {
    // (no fallback font on this machine: nothing to check)
    if fallback_bytes().is_none() {
        return;
    }
    let ink = |text: &str| {
        let html = format!("<body style='margin:0;background:#000;color:#fff;font-size:20px'>{text}</body>");
        let mut r = EngineRenderer::new(120, 30, &html);
        let f = r.poll_frame().unwrap();
        f.chunks(4).filter(|p| p[0] > 128).count()
    };
    assert!(ink("가나다라") > 40);
    assert!(ink("<b>정류장</b>") > 40);
    with_fonts(|reg, _| face_for(reg, false, '가', |_, id, fi| assert!(id.0 != 0 && fi == 1)));
    with_fonts(|reg, _| face_for(reg, false, 'A', |_, _, fi| assert_eq!(fi, 0)));
}
