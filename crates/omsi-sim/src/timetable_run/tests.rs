include!("../../../../tools/test-support/original_root.rs");

use super::*;

#[test]
fn a_stop_is_no_target_of_itself() {
    // a circular line: from A round to A; B has two platforms of one name
    let names = |id: i64| match id {
        1 => "A".to_string(),
        2 | 3 => "B".to_string(),
        _ => "C".to_string(),
    };
    let t = station_targets([(vec![1, 2, 4, 3, 1], "A".to_string())].into_iter(), names);
    let of = |id: i64| t[&id].iter().map(|x| x.0.as_str()).collect::<Vec<_>>();
    assert_eq!(of(1), ["B", "C"]);
    assert_eq!(of(2), ["C", "A"]);
    assert_eq!(of(4), ["B", "A"]);
    assert_eq!(of(3), ["A"]);
    assert!(t[&1].iter().all(|x| x.1.contains("A")));
}

#[test]
fn a_terminus_is_the_first_row_of_its_name() {
    let t = |code: i32, id: &str, s: &[&str]| omsi_vehicle::hof::Terminus { code, texture_id: id.into(), terminus_stop: Some(id.into()), all_exit: false, strings: s.iter().map(|x| x.to_string()).collect() };
    let hof = omsi_vehicle::Hof { termini: vec![t(0, "Depot", &[]), t(3, "61-Other", &["61-MaoFangChang"]), t(4, "61-Third", &[]), t(1, "61-MaoFangChang", &["61-MaoFangChang"]), t(2, "Wickenberg Nord", &[])], ..Default::default() };
    assert_eq!(super::find_terminus(&hof, "61-MaoFangChang"), Some(3));
    assert_eq!(super::find_terminus(&hof, "  61-MaoFangChang "), Some(3));
    let hof = omsi_vehicle::Hof { termini: vec![t(0, "A", &["Wickenberg"]), t(1, "B", &["Wickenberg"])], ..Default::default() };
    assert_eq!(super::find_terminus(&hof, "wickenberg"), Some(0));
}

/// By its sign text a row is the one whose sign reads so, not an earlier one whose
/// second line does (Spandau's Machandelweg, district RUHLEBEN, before U Ruhleben).
#[test]
fn a_terminus_is_found_by_its_own_sign_before_another_signs_second_line() {
    let t = |code: i32, id: &str, s: &[&str]| omsi_vehicle::hof::Terminus { code, texture_id: id.into(), terminus_stop: Some(id.into()), strings: s.iter().map(|x| x.to_string()).collect(), ..Default::default() };
    let hof = omsi_vehicle::Hof {
        termini: vec![t(0, "Empty", &[""]), t(194, "Machandelweg", &["MACHANDELWEG", "RUHLEBEN", "MACHANDELWEG"]), t(282, "U Ruhleben", &["RUHLEBEN", "U-BAHNHOF"]), t(5, "Boerse", &["BOERSE", "RATHAUSMARKT"]), t(555, "RMarkt", &["", "RATHAUSMARKT"])],
        ..Default::default()
    };
    assert_eq!(super::find_terminus(&hof, "RUHLEBEN"), Some(2));
    assert_eq!(super::find_terminus(&hof, "Ruhleben"), Some(2));
    // (a sign whose first line is blank goes by the next)
    assert_eq!(super::find_terminus(&hof, "RATHAUSMARKT"), Some(4));
    // the ident first, as before
    assert_eq!(super::find_terminus(&hof, "Machandelweg"), Some(1));
    // another bus showing it (a LAN player's: its sign text): the same row
    let mut v = ibis_test_vehicle();
    set_ai_destination(&mut v, Some(&hof), "5", "RUHLEBEN", &[]);
    assert_eq!(v.var("IBIS_TerminusCode"), Some(282.0));
}

#[test]
#[ignore = "needs the original OMSI 2 install (OMSI_ROOT)"]
fn stock_depot_termini_are_found_by_their_sign_text() {
    // every row of the stock Spandau and Grundorf depot files by its sign text
    for file in ["Spandau 86.hof", "Grundorf.hof"] {
        let path = original_root().join("Vehicles/MAN_SD200").join(file);
        let hof = omsi_vehicle::Hof::load(&path).expect(file);
        for (i, row) in hof.termini.iter().enumerate() {
            let Some(sign) = row.strings.iter().find(|s| !s.trim().is_empty()) else { continue };
            let found = super::find_terminus(&hof, sign).map(|k| hof.termini[k].code);
            assert_eq!(found, Some(row.code), "{file} row {i} {} '{}'", row.code, sign.trim());
        }
    }
}

#[test]
fn complex_line_keeps_letter_suffix() {
    assert_eq!(complex_line_text("5", 5.0), "005  ");
    assert_eq!(complex_line_text("5E", 5.0), "   5E");
    assert_eq!(line_suffix_from_text("5E"), 10);
    assert_eq!(line_suffix_from_text("5N"), 4);
    assert_eq!(line_suffix_from_text("5S"), 23);
    assert_eq!(line_code_from_text("5E", Some(505)), Some(510));
    assert_eq!(line_code_from_text("5", Some(505)), Some(500));
}

/// #459: a four-digit line keeps its number and gets no suffix from its route code.
#[test]
fn four_digit_line_keeps_its_number() {
    assert_eq!(line_code_from_text("7110", Some(711001)), Some(711000));
    assert_eq!(line_code_from_text("7110", None), Some(711000));
    assert_eq!(line_code_from_text("7110-10", Some(711010)), Some(711000));
    assert_eq!(line_code_from_text("1234E", None), Some(123410));
    assert_eq!(complex_line_text("7110", 7110.0), "7110  ");
}

/// #546: a letter-first line had no number, and the DL05's matrix blanks line 0.
#[test]
fn line_with_letter_prefix_keeps_its_number() {
    assert_eq!(line_code_from_text("X10", None), Some(1036));
    assert_eq!(line_code_from_text("X10", Some(51001)), Some(51036));
    assert_eq!(line_code_from_text("M41", Some(4101)), Some(4128));
    assert_eq!(line_code_from_text("N9", None), Some(935));
    assert_eq!(line_code_from_text("TML", Some(7601)), Some(7601));
    assert_eq!(line_suffix_from_text("X10"), 36);
    assert_eq!(line_number_digits("X10"), "10");
    assert_eq!(line_number_digits("5E"), "5");
}

#[test]
#[ignore = "needs the original OMSI 2 install (OMSI_ROOT)"]
fn berlin_5e_uses_its_real_terminus_when_no_hof_route_exists() {
    let path = original_root().join("Vehicles/MAN_SD202/Berlin.hof");
    let hof = omsi_vehicle::Hof::load(&path).expect("Berlin.hof");
    let target = ibis_target(&hof, "5E", "Fernbahnhof Spandau", &[], None).expect("5E target");
    assert_eq!(target.terminus_code, Some(232));
    assert_eq!(
        target.terminus_index,
        hof.termini.iter().position(|t| t.code == 232).unwrap() as i32
    );
    let target = ibis_target(&hof, "5E", "Spektefeld Schulzentrum", &[], None)
        .expect("5E shortened HOF target");
    assert_eq!(target.terminus_code, Some(233));
}

#[test]
#[ignore = "needs the original OMSI 2 install (OMSI_ROOT)"]
fn berlin_5e_does_not_turn_hof_route_505_into_s5() {
    let path = original_root().join("Vehicles/MAN_NL_NG/Spandau 89-11.hof");
    let hof = omsi_vehicle::Hof::load(&path).expect("Spandau 89-11.hof");
    let target = ibis_target(&hof, "5E", "Nervenklinik", &["U Rathaus Spandau"], None)
        .expect("5E route target");
    assert_eq!(target.route, Some(3));
    assert_eq!(target.suffix, 10);
}

#[test]
fn stop_names_meet_in_any_order_and_spelling() {
    assert_eq!(stop_words("Nordstadt Bhf"), stop_words("Bhf. Nordstadt"));
    assert_eq!(stop_words("F_Kirchweg"), stop_words("Kirchweg"));
    assert_ne!(stop_words("Bhf Nordstadt"), stop_words("Nordstadt"));
    assert!(stop_words("").is_empty());
}

/// A made-up line 7 with six routes to Hafen, each telling one rule of `pick_route`.
fn hafen_depot() -> omsi_vehicle::Hof {
    let mut hof = omsi_vehicle::Hof {
        termini: vec![omsi_vehicle::hof::Terminus {
            code: 100,
            strings: vec!["Hafen".into()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let routes: [(&str, &[&str]); 6] = [
        ("707", &["Markt", "Schule", "Park", "Ufer", "Hafen"]),
        ("701", &["Markt", "Schule", "Park", "Hafen"]),
        ("702", &["Bhf Nordstadt", "F_Kirchweg", "Markt", "Schule", "Park", "Hafen"]),
        ("703", &["Schule", "Park", "Hafen"]),
        ("705", &["Bhf Nordstadt", "Markt", "Rathaus", "Schule", "Park", "Hafen"]),
        ("706", &["Am Wald", "Kirchweg", "Markt", "Schule", "Park", "Hafen"]),
    ];
    for (code, stops) in routes {
        hof.info_trips.push(omsi_vehicle::hof::InfoTrip {
            code: code.into(),
            route: "100".into(),
            line: "7".into(),
            ..Default::default()
        });
        hof.info_busstop_lists
            .push(stops.iter().map(|s| s.to_string()).collect());
    }
    hof
}

#[test]
fn the_route_follows_the_trips_stops() {
    let hof = hafen_depot();
    let route = |stops: &[&str]| {
        ibis_target(&hof, "7", "Hafen", stops, None)
            .expect("line 7 target")
            .route
    };
    // the depot file spells the first stop another way, and with a one-letter prefix
    assert_eq!(
        route(&["Nordstadt Bhf", "Kirchweg", "Markt", "Schule", "Park", "Hafen"]),
        Some(2)
    );
    // a short working gets its own route, not the long ones it is part of
    assert_eq!(route(&["Schule", "Park", "Hafen"]), Some(3));
    // of two routes from the trip's first stop, the one as long as the trip
    assert_eq!(route(&["Markt", "Schule", "Park", "Hafen"]), Some(1));
    // starting at the trip's first stop counts before one stop more of the trip: 705
    // has Rathaus too but begins at Bhf Nordstadt, so a route from Markt (707, as
    // long as the trip) is taken
    assert_eq!(
        route(&["Markt", "Rathaus", "Schule", "Park", "Hafen"]),
        Some(7)
    );
    // nothing known of the trip: the first route, as before
    assert_eq!(route(&[]), Some(7));
}

#[test]
fn the_ibis_stands_at_the_trips_first_stop_on_a_route_that_begins_before_it() {
    let hof = hafen_depot();
    // no route begins at Kirchweg; 702 and 706 have the trip's stops after one more
    let stops = ["Kirchweg", "Markt", "Schule", "Park", "Hafen"];
    let target = ibis_target(&hof, "7", "Hafen", &stops, Some((0, stops[0])))
        .expect("line 7 target");
    assert_eq!(target.route, Some(2));
    assert_eq!(target.stop, 1);
}

fn link(a: i64, b: i64) -> Option<f64> {
    // 100 m between neighbours, 300 m from 3 to 4
    Some(if (a, b) == (3, 4) { 300.0 } else { 100.0 })
}

#[test]
fn trip_times_from_the_profile() {
    let stations = [1, 2, 3, 4, 5];
    // no manual times: the duration split by the link lengths
    let p = omsi_timetable::TripProfile {
        name: "p".into(),
        factor: 10.0,
        ..Default::default()
    };
    let t = TripTimes::new(&stations, Some(&p), &link);
    let arr: Vec<f64> = t.stations.iter().map(|s| s.0).collect();
    assert_eq!(arr, vec![0.0, 100.0, 200.0, 500.0, 600.0]);
    assert_eq!(t.duration, 600.0);
    // no time of a stop's own: none of them is a time point the bus waits at
    assert_eq!(t.holds, vec![false; 5]);
    // manual minutes win, the rest in between by length; a passed station stops nowhere
    let p = omsi_timetable::TripProfile {
        name: "p".into(),
        factor: 10.0,
        man_dep_time: vec![(0, 1.0), (1, 2.0)],
        man_arr_time: vec![(3, 6.0), (4, 9.0)],
        other_stopping: vec![(2, 2)],
    };
    let t = TripTimes::new(&stations, Some(&p), &link);
    assert_eq!(t.stations[0], (60.0, 60.0));
    assert_eq!(t.stations[1], (120.0, 120.0));
    // station 2 lies 100 m of the 400 m between the departure at 2 min and the arrival at 6
    assert!((t.stations[2].0 - 180.0).abs() < 1e-9, "{:?}", t.stations);
    assert_eq!(t.stations[3], (360.0, 360.0));
    assert_eq!(t.duration, 540.0);
    assert_eq!(t.stops, vec![true, true, false, true, true]);
    // the stops the map wrote a time for are the ones the bus waits at
    assert_eq!(t.holds, vec![true, true, false, true, true]);
    // a profile without stations keeps its duration (flights on a track)
    assert_eq!(
        TripTimes::new(
            &[],
            Some(&omsi_timetable::TripProfile {
                factor: 25.0,
                ..Default::default()
            }),
            &link
        )
            .duration,
        1500.0
    );
}

pub(super) fn planned(departure: f64, stops: &[(f64, f64, f64)]) -> PlannedTrip {
    let stops: Vec<PlannedStop> = stops
        .iter()
        .enumerate()
        .map(|(i, &(x, arr, dep))| PlannedStop {
            object_id: i as i64,
            name: format!("s{i}"),
            arr,
            dep,
            position: Some(glam::DVec3::new(x, 0.0, 0.0)),
            dir: StopDir::default(),
            stops: true,
        })
        .collect();
    PlannedTrip {
        name: format!("t{departure}"),
        line: "5".into(),
        terminus: "T".into(),
        departure,
        end: stops.last().unwrap().arr,
        stops,
    }
}

/// A vehicle whose script drops its duty and destination while it has no timetable,
/// and shows what the timetable callbacks tell it.
fn timetable_test_vehicle() -> crate::VehicleInstance {
    timetable_test_vehicle_with_door("door_0")
}

fn timetable_test_vehicle_with_door(door: &str) -> crate::VehicleInstance {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "omsi_tt_restore_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("device.osc");
    let vars = dir.join("vars.txt");
    let strings = dir.join("strings.txt");
    std::fs::write(
        &vars,
        format!("duty\nobserved_stop\nobserved_delay\n{door}\n"),
    )
    .unwrap();
    std::fs::write(&strings, "destination\nobserved_line\n").unwrap();
    std::fs::write(
        &script,
        r#"
{frame}
(L.L.schedule_active) ! (M.V.GetTTBusstopCount) 0 = ||
{if}
0 (S.L.duty)
"" (S.$.destination)
{endif}
(M.V.GetTTLineString) (S.$.observed_line)
(M.V.GetTTBusstopIndex) (S.L.observed_stop)
(M.V.GetTTDelay) (S.L.observed_delay)
{end}
"#,
    )
    .unwrap();
    let program = omsi_script::compile(&omsi_script::CompileInput {
        scripts: vec![script],
        varlists: vec![vars],
        stringvarlists: vec![strings],
        builtin_vars: vec!["schedule_active".into()],
        ..Default::default()
    });
    assert!(program.errors.is_empty(), "{:?}", program.errors);
    let ty = Arc::new(crate::VehicleType {
        def: Default::default(),
        model: Default::default(),
        model_dir: dir.clone(),
        program: Arc::new(program),
        meshes: Vec::new(),
        paint_schemes: Vec::new(),
        texchanges: Vec::new(),
        wheel_meshes: Vec::new(),
        suspension_axles: Vec::new(),
        missing_packs: Vec::new(),
        mesh_bounds: Vec::new(),
        mesh_boxes: Vec::new(),
    });
    std::fs::remove_dir_all(dir).unwrap();
    crate::VehicleInstance::new(ty, crate::VehicleHost::new(Default::default()))
}

/// A vehicle that declares the IBIS variables a destination is written to.
pub(super) fn ibis_test_vehicle() -> crate::VehicleInstance {
    script_test_vehicle("{frame}\n{end}\n", "IBIS_LinieKurs\nIBIS_TerminusIndex\nIBIS_TerminusCode\n", "IBIS_terminus_name\n")
}

/// A vehicle of the script `osc` that declares the variables `varlist` and the string
/// variables `stringvarlist` (one a line).
pub(crate) fn script_test_vehicle(osc: &str, varlist: &str, stringvarlist: &str) -> crate::VehicleInstance {
    // (a folder of its own: tests run side by side)
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("omsi_ibis_dest_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("ibis.osc");
    let vars = dir.join("vars.txt");
    let strings = dir.join("strings.txt");
    std::fs::write(&vars, varlist).unwrap();
    std::fs::write(&strings, stringvarlist).unwrap();
    std::fs::write(&script, osc).unwrap();
    let program = omsi_script::compile(&omsi_script::CompileInput {
        scripts: vec![script],
        varlists: vec![vars],
        stringvarlists: vec![strings],
        ..Default::default()
    });
    assert!(program.errors.is_empty(), "{:?}", program.errors);
    let ty = std::sync::Arc::new(crate::VehicleType {
        def: Default::default(),
        model: Default::default(),
        model_dir: dir.clone(),
        program: std::sync::Arc::new(program),
        meshes: Vec::new(),
        paint_schemes: Vec::new(),
        texchanges: Vec::new(),
        wheel_meshes: Vec::new(),
        suspension_axles: Vec::new(),
        missing_packs: Vec::new(),
        mesh_bounds: Vec::new(),
        mesh_boxes: Vec::new(),
    });
    std::fs::remove_dir_all(dir).unwrap();
    crate::VehicleInstance::new(ty, crate::VehicleHost::new(Default::default()))
}

/// #738: of four destinations of one name the one picked from the list is set, not
/// the first of that name.
#[test]
fn a_destination_picked_from_the_list_is_that_one_of_its_name() {
    let t = |code: i32, id: &str| omsi_vehicle::hof::Terminus { code, texture_id: id.into(), strings: vec!["ul. Xutorskaya".into()], ..Default::default() };
    let hof = omsi_vehicle::Hof { termini: vec![t(92, "Xut_92"), t(120, "Xut_120"), t(123, "Xut_123"), t(124, "Xut_124")], ..Default::default() };
    let mut v = ibis_test_vehicle();
    set_player_destination_at(&mut v, &hof, "39", 1, &[]);
    assert_eq!(v.var("IBIS_TerminusCode"), Some(120.0));
    assert_eq!(v.var("IBIS_TerminusIndex"), Some(1.0));
    assert_eq!(v.var("IBIS_LinieKurs"), Some(39.0));
    assert_eq!(v.str_var("IBIS_terminus_name"), "ul. Xutorskaya");
    set_player_destination_at(&mut v, &hof, "39", 3, &[]);
    assert_eq!(v.var("IBIS_TerminusCode"), Some(124.0));
    // (by its name it is the first of them, as the list used to set it)
    set_player_destination_directly(&mut v, Some(&hof), "39", "ul. Xutorskaya", &[]);
    assert_eq!(v.var("IBIS_TerminusCode"), Some(92.0));
}

/// A destination picked from the list (or a route number) turns a hand-cranked roller
/// blind to its row, as OMSI 2's line and destination dialog does: the AI's way, the
/// stock blinds' ai_scheduled_settarget taking AI_target_index for the blind's place.
/// On a bus not switched on yet (#1098: the SD77 is put on the road cold) once the main
/// switch is on and the start-up is done.
#[test]
fn a_destination_picked_from_the_list_turns_the_roller_blind() {
    let osc = "{trigger:rollband_sync}\n{end}\n{trigger:ai_scheduled_settarget}\n(L.L.AI_target_index) (S.L.rlbnd_ziel_target)\n(L.$.SetLineTo) (S.$.rlbnd_line)\n{end}\n";
    let vars = "IBIS_LinieKurs\nIBIS_TerminusIndex\nIBIS_TerminusCode\nAI_target_index\nrlbnd_ziel_target\nelec_busbar_main_sw\n";
    let t = |code: i32, id: &str| omsi_vehicle::hof::Terminus { code, texture_id: id.into(), strings: vec![id.to_uppercase()], ..Default::default() };
    let hof = omsi_vehicle::Hof { termini: vec![t(0, "Empty"), t(205, "U Ruhleben"), t(154, "U Rathaus Spandau")], ..Default::default() };
    let mut v = script_test_vehicle(osc, vars, "SetLineTo\nrlbnd_line\nIBIS_terminus_name\n");
    v.set_var("elec_busbar_main_sw", 1.0);
    let mut pick = set_player_destination_at(&mut v, &hof, "145", 2, &[]);
    turn_roller_blind(&mut v, &mut pick, false);
    assert_eq!(pick, None);
    assert_eq!(v.var("rlbnd_ziel_target"), Some(2.0));
    assert_eq!(v.str_var("rlbnd_line"), "145");
    // (and the IBIS as before)
    assert_eq!(v.var("IBIS_TerminusCode"), Some(154.0));
    // with the main switch off the pick waits for it (the stock trigger switches it on
    // itself, and the start-up's toggle then switched it off again), and for the start-up
    v.set_var("elec_busbar_main_sw", 0.0);
    let mut pick = set_player_destination_at(&mut v, &hof, "145", 1, &[]);
    turn_roller_blind(&mut v, &mut pick, false);
    assert_eq!(v.var("rlbnd_ziel_target"), Some(2.0));
    assert_eq!(v.var("IBIS_TerminusCode"), Some(205.0));
    v.set_var("elec_busbar_main_sw", 1.0);
    turn_roller_blind(&mut v, &mut pick, true);
    assert_eq!(v.var("rlbnd_ziel_target"), Some(2.0));
    turn_roller_blind(&mut v, &mut pick, false);
    assert_eq!(v.var("rlbnd_ziel_target"), Some(1.0));
    assert_eq!(pick, None);
    // a bus without a roller blind has nothing to turn
    let mut ibis = ibis_test_vehicle();
    assert_eq!(set_player_destination_at(&mut ibis, &hof, "145", 1, &[]), None);
}

#[test]
fn a_destination_picked_from_the_list_updates_display_target() {
    let osc = "{trigger:ai_scheduled_settarget}\n(L.L.AI_target_index) (S.L.display_target)\n{end}\n";
    let vars = "IBIS_LinieKurs\nIBIS_TerminusIndex\nIBIS_TerminusCode\nAI_target_index\ndisplay_target\n";
    let t = |code: i32, id: &str| omsi_vehicle::hof::Terminus { code, texture_id: id.into(), strings: vec![id.into()], ..Default::default() };
    let hof = omsi_vehicle::Hof { termini: vec![t(100, "First"), t(200, "Second")], ..Default::default() };
    let mut v = script_test_vehicle(osc, vars, "IBIS_terminus_name\n");
    set_player_destination_at(&mut v, &hof, "10", 1, &[]);
    assert_eq!(v.var("display_target"), Some(1.0));
}

/// A route number set by hand keeps the destination a roller blind shows: the row it
/// was cranked to, of which the IBIS knows nothing (taken from the IBIS, at its empty
/// row, a route pick turned the blind back to Empty), its plug-in sign's where one is up,
/// and a pick still waiting for the electrics before either.
#[test]
fn a_route_picked_by_hand_keeps_the_roller_blinds_destination() {
    let osc = "{trigger:rollband_sync}\n{end}\n{trigger:ai_scheduled_settarget}\n(L.L.AI_target_index) (S.L.rlbnd_ziel_target)\n(L.$.SetLineTo) (S.$.rlbnd_line)\n{end}\n";
    let vars = "IBIS_LinieKurs\nIBIS_TerminusIndex\nIBIS_TerminusCode\nAI_target_index\nrlbnd_ziel_target\nrlbnd_steckschild_Termindex\nelec_busbar_main_sw\n";
    let t = |code: i32, id: &str, sign: &str| omsi_vehicle::hof::Terminus { code, texture_id: id.into(), strings: vec![sign.into()], ..Default::default() };
    let hof = omsi_vehicle::Hof {
        termini: vec![t(0, "Empty", ""), t(205, "U Ruhleben", "U RUHLEBEN"), t(154, "U Rathaus Spandau", "U RATHAUS SPANDAU"), t(1001, "Falkensee Bhf", "FALKENSEE BHF")],
        ..Default::default()
    };
    let mut v = script_test_vehicle(osc, vars, "SetLineTo\nrlbnd_line\nIBIS_terminus_name\n");
    v.set_var("elec_busbar_main_sw", 1.0);
    v.set_var("rlbnd_steckschild_Termindex", -1.0);
    // cranked by hand to row 2; the IBIS (none on the bus) still at the empty row
    v.set_var("rlbnd_ziel_target", 2.0);
    v.set_var("IBIS_TerminusIndex", 0.0);
    v.set_var("IBIS_TerminusCode", 0.0);
    let ti = shown_destination(&v, &hof, None);
    assert_eq!(ti, Some(2));
    let mut pick = set_player_destination_at(&mut v, &hof, "5", ti.unwrap(), &[]);
    turn_roller_blind(&mut v, &mut pick, false);
    assert_eq!(v.var("rlbnd_ziel_target"), Some(2.0));
    assert_eq!(v.str_var("rlbnd_line"), "  5");
    assert_eq!(v.var("IBIS_TerminusCode"), Some(154.0));
    // a plug-in sign up: its row (the script's target_index_int); not one of no sign text
    v.set_var("rlbnd_steckschild_Termindex", 3.0);
    assert_eq!(shown_destination(&v, &hof, None), Some(3));
    v.set_var("rlbnd_steckschild_Termindex", 0.0);
    assert_eq!(shown_destination(&v, &hof, None), Some(2));
    // a destination picked before the bus was switched on, not turned to yet
    assert_eq!(shown_destination(&v, &hof, Some(&BlindPick { row: 1, line: "5".into() })), Some(1));
    // a bus without a roller blind: the IBIS's row
    let mut ibis = ibis_test_vehicle();
    ibis.set_var("IBIS_TerminusIndex", 1.0);
    ibis.set_var("IBIS_TerminusCode", 205.0);
    assert_eq!(shown_destination(&ibis, &hof, None), Some(1));
}

/// A depot file with a row of one destination per route, each of its own code (#738's
/// four "ul. Xutorskaya"): a duty types the route its stops take, and the IBIS shows that
/// route's destination, not the first row's code with no route.
#[test]
fn a_trip_to_a_destination_of_several_codes_types_its_own_route() {
    let t = |code: i32, id: &str, name: &str| omsi_vehicle::hof::Terminus { code, texture_id: id.into(), terminus_stop: Some(id.into()), strings: vec![name.into()], ..Default::default() };
    let mut hof = omsi_vehicle::Hof {
        termini: vec![t(0, "Leerfeld", ""), t(92, "Xut_92", "ul. Xutorskaya"), t(120, "Xut_120", "ul. Xutorskaya"), t(123, "Xut_123", "ul. Xutorskaya"), t(124, "Xut_124", "ul. Xutorskaya"), t(50, "Vokzal", "Vokzal")],
        ..Default::default()
    };
    let routes: [(&str, &str, &str, &[&str]); 4] = [
        ("3901", "39", "50", &["ul. Xutorskaya", "Rynok", "Vokzal"]),
        ("3902", "39", "120", &["Vokzal", "Rynok", "ul. Xutorskaya"]),
        ("4101", "41", "123", &["Park", "Shkola", "ul. Xutorskaya"]),
        ("4102", "41", "124", &["Vokzal", "Shkola", "ul. Xutorskaya"]),
    ];
    for (code, line, terminus, stops) in routes {
        hof.info_trips.push(omsi_vehicle::hof::InfoTrip { code: code.into(), route: terminus.into(), line: line.into(), ..Default::default() });
        hof.info_busstop_lists.push(stops.iter().map(|s| s.to_string()).collect());
    }
    let target = |hof: &omsi_vehicle::Hof, line: &str, stops: &[&str]| {
        let t = ibis_target(hof, line, "ul. Xutorskaya", stops, None).expect("a target");
        (t.route, t.terminus_code, t.terminus_index)
    };
    assert_eq!(target(&hof, "39", &["Vokzal", "Rynok", "ul. Xutorskaya"]), (Some(2), None, 2));
    // two routes of line 41 there, each to a row of its own
    assert_eq!(target(&hof, "41", &["Park", "Shkola", "ul. Xutorskaya"]), (Some(1), None, 3));
    assert_eq!(target(&hof, "41", &["Vokzal", "Shkola", "ul. Xutorskaya"]), (Some(2), None, 4));
    // no route of the line goes there: the first row's code, typed in the destination mode
    assert_eq!(target(&hof, "7", &[]), (None, Some(92), 1));
    // set without typing (when the typing fails): the same row
    let mut v = ibis_test_vehicle();
    set_player_destination_directly(&mut v, Some(&hof), "41", "ul. Xutorskaya", &["Vokzal", "Shkola", "ul. Xutorskaya"]);
    assert_eq!((v.var("IBIS_TerminusCode"), v.var("IBIS_TerminusIndex")), (Some(124.0), Some(4.0)));
    // an AI bus shows the first row of the name, as Omsi.exe's AI_target_index does
    set_ai_destination(&mut v, Some(&hof), "41", "ul. Xutorskaya", &["Vokzal", "Shkola", "ul. Xutorskaya"]);
    assert_eq!(v.var("IBIS_TerminusIndex"), Some(1.0));
    // rows of one ident as well
    for term in &mut hof.termini[1..5] {
        term.texture_id = "ul. Xutorskaya".into();
    }
    assert_eq!(target(&hof, "39", &["Vokzal", "Rynok", "ul. Xutorskaya"]), (Some(2), None, 2));
    assert_eq!(target(&hof, "41", &["Vokzal", "Shkola", "ul. Xutorskaya"]), (Some(2), None, 4));
}

/// An AI group that names no depot file, or a terminus its depot file has no row for:
/// Omsi.exe still puts the line into `SetLineTo` and runs `ai_scheduled_settarget`, which
/// is what the mods' roller blinds and matrices switch their destination picture on in.
#[test]
fn an_ai_bus_without_a_destination_still_gets_its_line_and_the_ai_trigger() {
    let osc = "{trigger:ai_scheduled_settarget}\n(L.$.SetLineTo) (S.$.shown)\n1 (S.L.elec_busbar_main_sw)\n{end}\n";
    let vars = "AI_target_index\nelec_busbar_main_sw\n";
    let strings = "SetLineTo\nshown\n";
    // no depot file at all (a plain [aigroup_2] group)
    let mut v = script_test_vehicle(osc, vars, strings);
    set_ai_destination(&mut v, None, "82A", "82A Seaview Bay", &[]);
    assert_eq!(v.str_var("SetLineTo"), "82A");
    assert_eq!(v.str_var("shown"), "82A");
    assert_eq!(v.var("elec_busbar_main_sw"), Some(1.0));
    // a terminus the depot file knows nothing of (AI_target_index stays as it was)
    let hof = omsi_vehicle::Hof {
        termini: vec![omsi_vehicle::hof::Terminus { code: 92, texture_id: "Xut_92".into(), ..Default::default() }],
        ..Default::default()
    };
    let mut v = script_test_vehicle(osc, vars, strings);
    set_ai_destination(&mut v, Some(&hof), "7", "Hafen", &[]);
    assert_eq!(v.str_var("SetLineTo"), "7");
    assert_eq!(v.str_var("shown"), "7");
    assert_eq!(v.var("elec_busbar_main_sw"), Some(1.0));
    assert_eq!(v.var("AI_target_index"), Some(0.0));
}

mod authored_station;
mod duty;
mod route;

#[test]
fn destination_picks_finish_the_current_transition_then_animate_the_latest() {
    // Build pending text at the start of a three-frame transition,
    // then publish it and acknowledge the latest requested row when it finishes.
    // Later selections must wait, leaving the pending text and timer intact.
    let osc = r#"{trigger:ai_scheduled_settarget}
(L.L.AI_target_index) (S.L.requested_row)
-1 (S.L.applied_row)
{end}
{frame}
(L.L.requested_row) (L.L.applied_row) = !
{if}
    (L.L.LW_req_ziel_change) 0 =
    {if}
        (L.L.requested_row) 0 (M.V.GetTerminusString) (S.$.LW_req_voll)
    {endif}
    (L.L.LW_req_ziel_change) 1 + (S.L.LW_req_ziel_change) 3 >=
    {if}
        (L.$.LW_req_voll) (S.$.shown_text)
        (L.L.requested_row) (S.L.applied_row)
        0 (S.L.LW_req_ziel_change)
    {endif}
{endif}
{end}
"#;
    let mut v = script_test_vehicle(
        osc,
        "AI_target_index\nrequested_row\napplied_row\nLW_req_ziel_change\n",
        "LW_req_voll\nshown_text\n",
    );
    let hof = omsi_vehicle::Hof {
        termini: ["First destination", "Second destination", "Third destination"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| omsi_vehicle::hof::Terminus {
                code: 100 + i as i32,
                texture_id: name.into(),
                strings: vec![name.into()],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    v.host.hof = Some(std::sync::Arc::new(hof.clone()));

    set_player_destination_at(&mut v, &hof, "", 0, &[]);
    v.update(0.1);
    assert_eq!(v.str_var("LW_req_voll"), "First destination");
    assert_eq!(v.str_var("shown_text"), "");
    assert!(v.var("LW_req_ziel_change").unwrap() > 0.0);

    let timer = v.var("LW_req_ziel_change");
    set_player_destination_at(&mut v, &hof, "", 1, &[]);
    set_player_destination_at(&mut v, &hof, "", 2, &[]);
    assert_eq!(v.var("LW_req_ziel_change"), timer);
    assert_eq!(v.str_var("LW_req_voll"), "First destination");
    assert_eq!(v.var("AI_target_index"), Some(0.0));
    assert_eq!(shown_destination(&v, &hof, None), Some(2));

    // The first transition completes with its original target and text.
    for _ in 0..2 {
        v.update(0.1);
    }
    assert_eq!(v.str_var("shown_text"), "First destination");
    assert_eq!(v.var("applied_row"), Some(0.0));
    assert_eq!(v.var("LW_req_ziel_change"), Some(0.0));

    // Start a full transition to the newest selection; skip the intermediate one.
    v.update(0.1);
    assert_eq!(v.str_var("LW_req_voll"), "Third destination");
    assert_eq!(v.str_var("shown_text"), "First destination");
    v.update(0.1);
    assert_eq!(v.str_var("shown_text"), "First destination");
    v.update(0.1);
    assert_eq!(v.str_var("shown_text"), "Third destination");
    assert_eq!(v.var("applied_row"), Some(2.0));

    // A selection after a completed transition still updates normally.
    set_player_destination_at(&mut v, &hof, "", 1, &[]);
    for _ in 0..3 {
        v.update(0.1);
    }
    assert_eq!(v.str_var("shown_text"), "Second destination");
}

#[test]
fn guarded_display_transitions_finish_before_the_latest_selection_starts() {
    // The trigger accepts a request only when idle. A countdown
    // then publishes its pending text. Unrelated animation state must survive a pick.
    let osc = r#"{trigger:ai_scheduled_settarget}
(L.L.Matrix_ziel_animationtimer) 0 =
{if}
    (L.L.AI_target_index) 0 (M.V.GetTerminusString) (S.$.pending_text)
    3 (S.L.Matrix_ziel_animationtimer)
{endif}
{end}
{frame}
(L.L.Matrix_ziel_animationtimer) 0 >
{if}
    (L.L.Matrix_ziel_animationtimer) 1 - (S.L.Matrix_ziel_animationtimer) 0 =
    {if}
        (L.$.pending_text) (S.$.shown_text)
    {endif}
{endif}
{end}
"#;
    let hof = omsi_vehicle::Hof {
        termini: ["First", "Second", "Third"].into_iter().enumerate().map(|(i, name)| {
            omsi_vehicle::hof::Terminus {
                code: 100 + i as i32,
                texture_id: name.into(),
                strings: vec![name.into()],
                ..Default::default()
            }
        }).collect(),
        ..Default::default()
    };
    for player in [true, false] {
        let mut v = script_test_vehicle(
            osc,
            "AI_target_index\nMatrix_ziel_animationtimer\nunrelated_animation\n",
            "pending_text\nshown_text\n",
        );
        v.host.hof = Some(std::sync::Arc::new(hof.clone()));
        v.set_var("unrelated_animation", 7.0);
        super::set_destination_at(&mut v, &hof, "", 0, &[], player);
        v.update(0.1);
        assert_eq!(v.str_var("pending_text"), "First");
        assert!(v.var("Matrix_ziel_animationtimer").unwrap() > 0.0);

        let tick = |v: &mut crate::VehicleInstance| {
            if player {
                v.update(0.1);
            } else {
                v.update_ai(0.1, &crate::vehicle::AiFrame::default());
            }
        };
        super::set_destination_at(&mut v, &hof, "", 1, &[], player);
        super::set_destination_at(&mut v, &hof, "", 2, &[], player);
        assert_eq!(v.var("Matrix_ziel_animationtimer"), Some(2.0));
        assert_eq!(v.str_var("pending_text"), "First");
        tick(&mut v);
        tick(&mut v);
        assert_eq!(v.str_var("shown_text"), "First");
        assert_eq!(v.var("Matrix_ziel_animationtimer"), Some(0.0));

        tick(&mut v);
        assert_eq!(v.str_var("pending_text"), "Third");
        assert_eq!(v.str_var("shown_text"), "First");
        assert_eq!(v.var("Matrix_ziel_animationtimer"), Some(2.0));
        tick(&mut v);
        assert_eq!(v.str_var("shown_text"), "First");
        tick(&mut v);
        assert_eq!(v.str_var("shown_text"), "Third", "player={player}");
        assert_eq!(v.var("unrelated_animation"), Some(7.0));
    }
}

#[test]
fn a_destination_pick_without_a_display_trigger_keeps_script_timers() {
    let mut v = script_test_vehicle(
        "{frame}\n{end}\n",
        "IBIS_TerminusIndex\nLW_req_ziel_change\nMatrix_ziel_animationtimer\n",
        "IBIS_terminus_name\n",
    );
    let hof = omsi_vehicle::Hof {
        termini: vec![omsi_vehicle::hof::Terminus {
            code: 100,
            strings: vec!["Destination".into()],
            ..Default::default()
        }],
        ..Default::default()
    };
    v.set_var("LW_req_ziel_change", 5.0);
    v.set_var("Matrix_ziel_animationtimer", 6.0);
    set_player_destination_at(&mut v, &hof, "", 0, &[]);
    assert_eq!(v.str_var("IBIS_terminus_name"), "Destination");
    assert_eq!(v.var("IBIS_TerminusIndex"), Some(0.0));
    assert_eq!(v.var("LW_req_ziel_change"), Some(5.0));
    assert_eq!(v.var("Matrix_ziel_animationtimer"), Some(6.0));
}

#[test]
fn queued_destinations_are_discarded_when_the_depot_or_script_state_changes() {
    let osc = "{trigger:ai_scheduled_settarget}\n(L.L.AI_target_index) (S.L.accepted_row)\n{end}\n{frame}\n{end}\n";
    let hof = omsi_vehicle::Hof {
        name: "Original depot".into(),
        termini: vec![omsi_vehicle::hof::Terminus { code: 100, ..Default::default() }],
        ..Default::default()
    };
    for restore in [false, true] {
        let mut v = script_test_vehicle(
            osc,
            "AI_target_index\naccepted_row\nLW_req_ziel_change\n",
            "",
        );
        v.host.hof = Some(std::sync::Arc::new(hof.clone()));
        v.set_var("accepted_row", -1.0);
        v.set_var("LW_req_ziel_change", 1.0);
        set_player_destination_at(&mut v, &hof, "", 0, &[]);
        if restore {
            v.restore_script_state(&[("LW_req_ziel_change".into(), 0.0)], &[]);
        } else {
            let mut other = hof.clone();
            other.name = "Replacement depot".into();
            v.host.hof = Some(std::sync::Arc::new(other));
            v.set_var("LW_req_ziel_change", 0.0);
        }
        v.update_scripts_only(0.0);
        assert_eq!(v.var("accepted_row"), Some(-1.0));
        assert!(v.pending_destination.is_none());
    }
}

/// The row OMSI's AI bus is given: the first whose ident is the destination, whatever
/// the codes' order; of equally loose matches the first as well.
#[test]
fn a_tour_bus_takes_a_trip_on_only_at_its_start() {
    let route = [10, 11, 12, 13, 14, 15, 16];
    assert_eq!(tour_entry(&route, 10), Some(0));
    assert_eq!(tour_entry(&route, 13), Some(3));
    // the same trip again: the bus stands on its last lane
    assert_eq!(tour_entry(&route, 16), None);
    assert_eq!(tour_entry(&route, 99), None);
}
