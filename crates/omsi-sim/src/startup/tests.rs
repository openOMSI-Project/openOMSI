use super::*;
use crate::{SimClock, VehicleHost, VehicleType};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// Original, minimal scripts: no installed game content is needed by these tests.
struct Fixture {
    dir: PathBuf,
    ty: Arc<VehicleType>,
}

impl Fixture {
    fn new(script: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "omsi-startup-regression-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("test.bus"),
            "[model]\nmodel.cfg\n[varnamelist]\n1\nvars.txt\n[script]\n1\nmain.osc\n",
        )
        .unwrap();
        std::fs::write(dir.join("model.cfg"), "").unwrap();
        let vars: BTreeSet<&str> = script
            .split("(L.L.")
            .skip(1)
            .chain(script.split("(S.L.").skip(1))
            .map(|s| s.split_once(')').unwrap().0)
            .collect();
        std::fs::write(
            dir.join("vars.txt"),
            vars.into_iter().collect::<Vec<_>>().join("\n"),
        )
        .unwrap();
        std::fs::write(dir.join("main.osc"), script).unwrap();
        let ty = Arc::new(VehicleType::load(&dir, &dir.join("test.bus")).unwrap());
        assert!(ty.program.errors.is_empty(), "{:?}", ty.program.errors);
        Self { dir, ty }
    }

    fn vehicle(&self) -> VehicleInstance {
        VehicleInstance::new(self.ty.clone(), VehicleHost::new(SimClock::default()))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn advance(v: &mut VehicleInstance, start: &mut StartUp, dt: f32, seconds: f32) {
    for _ in 0..(seconds / dt).ceil() as usize {
        if start.running() {
            start.tick(v, &[], dt);
        }
        v.update(dt);
    }
}

/// The request must stay held: releasing before combustion cancels the start.
fn sustained_script(press: &str, frame: &str, release: &str) -> String {
    format!(
        r#"
{{init}}
1 (S.L.battery_on) 3 (S.L.engine_ignition)
{{end}}
{{trigger:kw_m_enginestart}}
1 (S.L.request) 1 (S.L.engine_ignition)
(L.L.presses) 1 + (S.L.presses)
{press}
{{end}}
{{trigger:kw_m_enginestart_off}}
0 (S.L.request) 3 (S.L.engine_ignition)
(L.L.releases) 1 + (S.L.releases)
(L.L.elapsed) (S.L.last_hold)
(L.L.engine_on) ! {{if}} 0 (S.L.engine_n) 0 (S.L.elapsed) {{endif}}
{release}
{{end}}
{{frame}}
(L.L.request) {{if}}
    (L.L.elapsed) (L.S.Timegap) + (S.L.elapsed)
    {frame}
{{endif}}
{{end}}
"#
    )
}

#[test]
fn holds_ignition_request_through_the_scripted_start_delay() {
    let fixture = Fixture::new(&sustained_script(
        "",
        r#"
        (L.L.elapsed) 0.6 > {if} 1 (S.L.engine_on) 650 (S.L.engine_n) {endif}
    "#,
        "",
    ));
    for dt in [1.0 / 30.0, 1.0 / 60.0] {
        let mut v = fixture.vehicle();
        assert_eq!(v.var("engine_starter"), None);
        let mut start = StartUp::new(&v, &[]);
        advance(&mut v, &mut start, dt, 5.0);
        assert!(engine_running(&v), "{:?}", start.report);
        assert!(!start.running());
        assert_eq!(v.var("presses"), Some(1.0));
        assert_eq!(v.var("releases"), Some(1.0));
        assert!(v.var("last_hold").unwrap() > 0.6);
        assert!(start.report.iter().any(|s| s.contains("engine started")));
    }
}

#[test]
fn starter_rpm_does_not_release_before_combustion() {
    // A conventional starter flag isolates RPM success detection from ignition discovery.
    let fixture = Fixture::new(&sustained_script(
        "1 (S.L.engine_starter)",
        r#"
        (L.L.elapsed) 0.1 > {if} 600 (S.L.engine_n) {endif}
        (L.L.elapsed) 0.8 > {if} 1 (S.L.engine_on) {endif}
    "#,
        "0 (S.L.engine_starter)",
    ));
    let mut v = fixture.vehicle();
    let mut start = StartUp::new(&v, &[]);
    advance(&mut v, &mut start, 1.0 / 60.0, 5.0);
    assert!(engine_running(&v), "{:?}", start.report);
    assert_eq!(v.var("engine_on"), Some(1.0));
    assert!(!start.running());
    assert_eq!(v.var("presses"), Some(1.0));
    assert_eq!(v.var("releases"), Some(1.0));
    assert!(v.var("last_hold").unwrap() > 0.8);
}

#[test]
fn first_firing_at_low_rpm_keeps_the_starter_held() {
    let fixture = Fixture::new(&sustained_script(
        "",
        r#"
        (L.L.elapsed) 0.1 > {if} 1 (S.L.engine_on) 150 (S.L.engine_n) {endif}
        (L.L.elapsed) 0.8 > {if} 650 (S.L.engine_n) {endif}
    "#,
        r#"
        (L.L.engine_n) 300 < {if} 0 (S.L.engine_on) 0 (S.L.engine_n) {endif}
    "#,
    ));
    let mut v = fixture.vehicle();
    let mut start = StartUp::new(&v, &[]);
    advance(&mut v, &mut start, 1.0 / 60.0, 5.0);
    assert!(engine_running(&v), "{:?}", start.report);
    assert!(!start.running());
    assert!(v.var("last_hold").unwrap() > 0.8);
    assert_eq!(v.var("presses"), Some(1.0));
    assert_eq!(v.var("releases"), Some(1.0));
}

#[test]
fn detects_starter_engagement_after_several_frames() {
    let fixture = Fixture::new(&sustained_script(
        "3 (S.L.engine_ignition)",
        r#"
        (L.L.elapsed) 0.1 > {if} 1 (S.L.engine_starter) {endif}
        (L.L.elapsed) 0.6 > {if} 1 (S.L.engine_on) 650 (S.L.engine_n) {endif}
    "#,
        "0 (S.L.engine_starter)",
    ));
    // Include a coarse frame where engagement is observed at the ordinary release time.
    for dt in [1.0 / 60.0, 0.1] {
        let mut v = fixture.vehicle();
        let mut start = StartUp::new(&v, &[]);
        advance(&mut v, &mut start, dt, 5.0);
        assert!(engine_running(&v), "dt={dt}: {:?}", start.report);
        assert!(!start.running());
        assert_eq!(v.var("presses"), Some(1.0));
        assert_eq!(v.var("releases"), Some(1.0));
    }
}

#[test]
fn ignition_stop_and_off_states_do_not_engage_the_starter() {
    for ignition in [2, 3] {
        let fixture = Fixture::new(&sustained_script(
            &format!("{ignition} (S.L.engine_ignition)"),
            "",
            "",
        ));
        let mut v = fixture.vehicle();
        let mut start = StartUp::new(&v, &[]);
        advance(&mut v, &mut start, 1.0 / 60.0, 12.0);
        assert!(!engine_running(&v));
        assert!(!start.running(), "{:?}", start.report);
        assert_eq!(v.var("presses"), Some(16.0));
        assert_eq!(v.var("releases"), Some(16.0));
        assert_eq!(v.var("request"), Some(0.0));
        assert!(start
            .report
            .iter()
            .any(|s| s.contains("could not work the starter")));
    }
}

#[test]
fn running_flags_are_authoritative_and_rpm_remains_a_fallback() {
    for flag in [
        "engine_on",
        "engine_running",
        "motor_on",
        "motor_running",
        "diesel_motor_running",
    ] {
        let fixture = Fixture::new(&format!(
            "{{init}} 0 (S.L.{flag}) 600 (S.L.engine_n) {{end}} {{frame}} {{end}}"
        ));
        let mut v = fixture.vehicle();
        assert!(!engine_running(&v), "{flag} explicitly reports off");
        assert!(!engine_caught(&v));
        // A rotating engine with an explicit off flag must start, not shut down.
        assert!(!StartUp::new(&v, &[]).shutting_down());
        v.set_var(flag, 1.0);
        assert!(engine_running(&v));
        assert!(engine_caught(&v));
        v.set_var("engine_n", 150.0);
        assert!(engine_running(&v));
        assert!(
            !engine_caught(&v),
            "first firing at low RPM is not enough to release"
        );
    }
    let rpm_only = Fixture::new("{init} 350 (S.L.motor_rpm) {end} {frame} {end}");
    let mut v = rpm_only.vehicle();
    assert!(!engine_running(&v));
    v.set_var("motor_rpm", 351.0);
    assert!(engine_running(&v));
    assert!(engine_caught(&v));
    let flag_only = Fixture::new("{init} 1 (S.L.engine_on) {end} {frame} {end}");
    assert!(engine_caught(&flag_only.vehicle()));
}

#[test]
fn staged_starter_still_gets_separate_presses() {
    let fixture = Fixture::new(&sustained_script(
        r#"
        (L.L.presses) 2 >= {if}
            1 (S.L.engine_starter)
        {else}
            3 (S.L.engine_ignition) 0 (S.L.request)
        {endif}
    "#,
        r#"
        (L.L.elapsed) 0.6 > {if} 1 (S.L.engine_on) 650 (S.L.engine_n) {endif}
    "#,
        "0 (S.L.engine_starter)",
    ));
    let mut v = fixture.vehicle();
    let mut start = StartUp::new(&v, &[]);
    advance(&mut v, &mut start, 1.0 / 60.0, 5.0);
    assert!(engine_running(&v), "{:?}", start.report);
    assert!(!start.running());
    assert_eq!(v.var("presses"), Some(2.0));
    assert_eq!(v.var("releases"), Some(2.0));
}

#[test]
fn moves_past_an_unresponsive_starter_candidate() {
    let script = sustained_script(
        "",
        r#"
        (L.L.elapsed) 0.6 > {if} 1 (S.L.engine_on) 650 (S.L.engine_n) {endif}
    "#,
        "",
    )
    .replace("kw_m_enginestart", "z_engine_start");
    let fixture = Fixture::new(&format!(
        r#"{script}
        {{trigger:a_engine_start}}
        (L.L.bad_presses) 1 + (S.L.bad_presses)
        {{end}}
        {{trigger:a_engine_start_off}}
        (L.L.bad_releases) 1 + (S.L.bad_releases)
        {{end}}
    "#
    ));
    let mut v = fixture.vehicle();
    let mut start = StartUp::new(&v, &[]);
    advance(&mut v, &mut start, 1.0 / 60.0, 15.0);
    assert!(engine_running(&v), "{:?}", start.report);
    assert!(!start.running());
    assert_eq!(v.var("bad_presses"), Some(16.0));
    assert_eq!(v.var("bad_releases"), Some(16.0));
    assert_eq!(v.var("presses"), Some(1.0));
    assert_eq!(v.var("releases"), Some(1.0));
}

#[test]
fn crank_timeout_includes_the_pending_delay_and_releases_each_attempt_once() {
    let fixture = Fixture::new(&sustained_script(
        "3 (S.L.engine_ignition)",
        r#"
        (L.L.elapsed) 0.15 > {if} 1 (S.L.engine_starter) {endif}
    "#,
        "0 (S.L.engine_starter)",
    ));
    let mut v = fixture.vehicle();
    let mut start = StartUp::new(&v, &[]);
    let dt = 1.0 / 60.0;
    advance(&mut v, &mut start, dt, 30.0);
    assert!(!start.running());
    assert!(!engine_running(&v));
    assert_eq!(v.var("presses"), Some(3.0));
    assert_eq!(v.var("releases"), Some(3.0));
    let held = v.var("last_hold").unwrap();
    assert!((held - MAX_CRANK).abs() <= dt * 2.0, "held {held}");
    assert_eq!(v.var("request"), Some(0.0));
    assert_eq!(v.var("engine_starter"), Some(0.0));
    assert!(start.report.iter().any(|s| s.contains("3 tries")));
    assert!(!start.tick(&mut v, &[], dt));
    assert_eq!(v.var("releases"), Some(3.0));
}

#[test]
fn known_power_loss_releases_the_starter_and_bounds_retries() {
    let fixture = Fixture::new(&sustained_script(
        "",
        r#"
        (L.L.elapsed) 0.1 > {if} 0 (S.L.battery_on) {endif}
    "#,
        "",
    ));
    let mut v = fixture.vehicle();
    let mut start = StartUp::new(&v, &[]);
    advance(&mut v, &mut start, 1.0 / 60.0, 5.0);
    assert!(!start.running());
    assert!(!engine_running(&v));
    assert!(!power_on(&v));
    assert_eq!(v.var("presses"), Some(3.0));
    assert_eq!(v.var("releases"), Some(3.0));
    assert!(v.var("last_hold").unwrap() < PRESS);
    assert_eq!(v.var("request"), Some(0.0));
}

#[test]
fn post_start_stall_retries_then_verifies_a_stable_engine() {
    let fixture = Fixture::new(&format!(
        r#"{}
        {{trigger:stall}}
        0 (S.L.engine_on) 0 (S.L.engine_n) 0 (S.L.elapsed)
        {{end}}
    "#,
        sustained_script(
            "",
            r#"
        (L.L.elapsed) 0.6 > {if} 1 (S.L.engine_on) 650 (S.L.engine_n) {endif}
    "#,
            ""
        )
    ));
    let mut v = fixture.vehicle();
    let mut start = StartUp::new(&v, &[]);
    let dt = 1.0 / 60.0;
    for _ in 0..300 {
        start.tick(&mut v, &[], dt);
        v.update(dt);
        if start.step == Step::Verify {
            break;
        }
    }
    assert_eq!(start.step, Step::Verify);
    assert_eq!(v.var("releases"), Some(1.0));
    v.trigger("stall");
    advance(&mut v, &mut start, dt, 6.0);
    assert!(engine_running(&v), "{:?}", start.report);
    assert!(!start.running());
    assert_eq!(v.var("presses"), Some(2.0));
    assert_eq!(v.var("releases"), Some(2.0));
    assert!(start
        .report
        .iter()
        .any(|s| s.contains("died again: cranking once more")));

    // An engine that dies after every release must also stop retrying after three tries.
    let mut v = fixture.vehicle();
    let mut start = StartUp::new(&v, &[]);
    for _ in 0..1200 {
        let running = start.tick(&mut v, &[], dt);
        v.update(dt);
        if start.step == Step::Verify {
            v.trigger("stall");
        }
        if !running {
            break;
        }
    }
    assert!(!start.running());
    assert!(!engine_running(&v));
    assert_eq!(v.var("presses"), Some(3.0));
    assert_eq!(v.var("releases"), Some(3.0));
    assert_eq!(v.var("request"), Some(0.0));
    assert!(start
        .report
        .iter()
        .any(|s| s == "the engine died again after starting"));
}

/// A fictional bus with arbitrary startup timings and speeds. Its only purpose is to
/// reproduce a sustained start request and rotation before combustion, using the state
/// names understood by the controller.
fn scripted_bus(ignition_request: bool) -> Fixture {
    let (init, press, held, release, caught, rpm) = if ignition_request {
        (
            "3 (S.L.engine_ignition)",
            "1 (S.L.engine_ignition)",
            "(L.L.engine_ignition) 1 =",
            "(L.L.engine_ignition) 1 = {if} 3 (S.L.engine_ignition) {endif}",
            "0 (S.L.engine_ignition)",
            450,
        )
    } else {
        (
            "0 (S.L.engine_starter)",
            "1 (S.L.engine_starter)",
            "(L.L.engine_starter)",
            "0 (S.L.engine_starter)",
            "",
            150,
        )
    };
    Fixture::new(&format!(
        r#"
{{init}}
0 (S.L.battery_on) 0 (S.L.engine_on) 0 (S.L.engine_n)
1 (S.L.selected_gear)
{init}
{{end}}
{{trigger:master_battery}}
1 (S.L.battery_on)
{{end}}
{{trigger:automatic_N}}
0 (S.L.selected_gear)
{{end}}
{{trigger:engine_start}}
(L.L.battery_on) (L.L.selected_gear) 0 = &&
(L.L.engine_on) ! && {{if}}
    {press}
    (L.L.presses) 1 + (S.L.presses)
{{endif}}
{{end}}
{{trigger:engine_start_off}}
{release}
(L.L.releases) 1 + (S.L.releases)
(L.L.elapsed) (S.L.last_hold)
(L.L.engine_on) ! {{if}}
    0 (S.L.engine_n) 0 (S.L.elapsed) 0 (S.L.battery_on)
{{endif}}
{{end}}
{{frame}}
(L.L.battery_on) {held} && {{if}}
    (L.L.elapsed) (L.S.Timegap) + (S.L.elapsed)
    (L.L.elapsed) 0.35 > {{if}} {rpm} (S.L.engine_n) {{endif}}
    (L.L.elapsed) 0.85 > {{if}}
        1 (S.L.engine_on) 720 (S.L.engine_n)
        {caught}
    {{endif}}
{{endif}}
{{end}}
"#
    ))
}

#[test]
fn scripted_buses_manual_and_automatic_startup() {
    let bound = vec!["master_battery".into(), "engine_start".into()];
    for ignition_request in [true, false] {
        let fixture = scripted_bus(ignition_request);
        for dt in [1.0 / 30.0, 1.0 / 60.0] {
            for automatic in [false, true] {
                let mut v = fixture.vehicle();
                assert!(!power_on(&v));
                assert_eq!(v.var("engine_on"), Some(0.0));
                assert_eq!(v.var("selected_gear"), Some(1.0));
                if ignition_request {
                    assert_eq!(v.var("engine_starter"), None);
                }
                if automatic {
                    let mut start = StartUp::new(&v, &bound);
                    for _ in 0..(6.0 / dt) as usize {
                        let running = start.tick(&mut v, &bound, dt);
                        v.update(dt);
                        if !running {
                            break;
                        }
                    }
                    assert!(!start.running(), "{:?}", start.report);
                    assert!(
                        start.report.iter().any(|s| s.contains("engine started")),
                        "{:?}",
                        start.report
                    );
                } else {
                    assert!(v.trigger("master_battery"));
                    v.update(dt);
                    v.trigger("master_battery_off");
                    // This fictional selector uses 0 for neutral and 1 for a gear.
                    assert!(v.trigger("engine_start"));
                    assert!(!starter_engaged(&v));
                    assert_eq!(v.var("presses"), Some(0.0));
                    assert!(v.trigger("automatic_N"));
                    v.trigger("automatic_N_off");
                    assert!(v.trigger("engine_start"));
                    for _ in 0..(1.2 / dt) as usize {
                        v.update(dt);
                    }
                    assert!(v.trigger("engine_start_off"));
                }
                assert_eq!(v.var("selected_gear"), Some(0.0));
                assert_eq!(v.var("presses"), Some(1.0));
                assert_eq!(v.var("releases"), Some(1.0));
                assert!(v.var("last_hold").unwrap() > 0.85);
                // Assert combustion and released controls on every frame after release.
                for _ in 0..(2.0 / dt) as usize {
                    v.update(dt);
                    assert!(power_on(&v));
                    assert_eq!(v.var("engine_on"), Some(1.0));
                    assert!(v.var("engine_n").unwrap() > 300.0);
                    assert!(!starter_engaged(&v));
                }
            }
        }
    }
}

#[test]
fn scripted_ignition_bus_cancels_if_released_at_starter_rpm() {
    let fixture = scripted_bus(true);
    let mut v = fixture.vehicle();
    let dt = 1.0 / 60.0;
    v.trigger("master_battery");
    v.trigger("automatic_N");
    v.trigger("engine_start");
    for _ in 0..36 {
        v.update(dt);
    }
    assert_eq!(v.var("engine_n"), Some(450.0));
    assert_eq!(v.var("engine_on"), Some(0.0));
    v.trigger("engine_start_off");
    for _ in 0..120 {
        v.update(dt);
    }
    assert_eq!(v.var("engine_on"), Some(0.0));
    assert_eq!(v.var("engine_n"), Some(0.0));
    assert!(!power_on(&v));
    assert!(!starter_engaged(&v));
}
