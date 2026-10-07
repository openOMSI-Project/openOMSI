//! Founding a company: its name and short name, its colours (and a logo picture if there is
//! one), its home map and depot, its first day (its year decides which buses are still
//! built new), how hard the economy is - with the starting capital each difficulty gives -
//! and how it buys its buses (the dealer's quick buy, or haggling and contracts).

use super::super::theme::*;
use super::super::ui::ButtonKind;
use super::super::Launcher;
use super::kit::{self, Foot};
use super::{data, eur, section};
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_launcher_lib::company::{self as co, Difficulty};
use omsi_ui::paint::Align;
use omsi_ui::{Rect, Weight};

/// The colours a company can wear (Omsi-Hub's accent colours and the classic city bus ones).
pub(super) const PALETTE: [&str; 12] = ["#f28c28", "#e03c31", "#c2185b", "#7b1fa2", "#283593", "#1e88e5", "#00897b", "#43a047", "#fdd835", "#6d4c41", "#455a64", "#f5f5f5"];

pub struct Wizard {
    name: String,
    short: String,
    colours: [usize; 2],
    logo: Option<String>,
    map: usize,
    depot: usize,
    /// The first company day: today (0), the map's date (1), or one of the player's own (2,
    /// `date`).
    when: usize,
    date: String,
    difficulty: usize,
    /// 0 simple, 1 advanced (`dealer::BuyingMode::ALL`).
    buying: usize,
}

impl Wizard {
    pub fn new(l: &Launcher) -> Wizard {
        let map = l.state.maps.iter().position(|m| m.file == l.state.choice.map).unwrap_or(0);
        Wizard { name: String::new(), short: String::new(), colours: [0, 4], logo: None, map, depot: 0, when: 0, date: today(), difficulty: 1, buying: 1 }
    }
}

/// Today's date as the computer has it, in its own time zone (taken anew each time; the UTC
/// date where the machine does not say).
pub(super) fn today() -> String {
    if let Some((y, m, d, _, _)) = omsi_launcher_lib::local_now() {
        return format!("{y:04}-{m:02}-{d:02}");
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    co::dates::fmt((secs / 86_400) as i64)
}

/// The date a map is set in: the date the launcher drives it on when it is the map chosen
/// there, else the year in its name ("Bad_Huegelsdorf_2020", "Vienna 2005") on the game's day
/// of the year, else the game's own first day.
pub(super) fn map_date(map: &core::MapInfo, chosen_map: &str, chosen_date: &str) -> String {
    if map.file == chosen_map && co::dates::parse(chosen_date).is_some() {
        return chosen_date.to_string();
    }
    let text = format!("{} {} {}", map.file, map.name, map.friendly);
    let year = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|w| w.len() == 4)
        .filter_map(|w| w.parse::<i32>().ok())
        .find(|y| (1950..=2099).contains(y));
    match year {
        Some(y) => format!("{y}{}", &core::DEFAULT_DATE[4..]),
        None => core::DEFAULT_DATE.to_string(),
    }
}

/// The first company day the wizard's choice comes to.
pub(super) fn first_day(when: usize, map: Option<&core::MapInfo>, chosen_map: &str, chosen_date: &str, own: &str) -> String {
    match when {
        1 => map.map(|m| map_date(m, chosen_map, chosen_date)).unwrap_or_else(today),
        2 if co::dates::parse(own).is_some() => own.to_string(),
        _ => today(),
    }
}

/// The depot files a company on `map` can give its buses: the map's own first, then those of
/// the installed buses that name the map, else all of them.
pub(super) fn depots_for(map: &core::MapInfo, vehicles: &[core::VehicleInfo]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    fn push(s: &str, out: &mut Vec<String>) {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(s)) {
            out.push(s.to_string());
        }
    }
    push(&map.hof, &mut out);
    let folder = map.file.split(['/', '\\']).rev().nth(1).unwrap_or("").to_lowercase();
    let words: Vec<String> = format!("{} {} {}", folder, map.name, map.friendly).to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 4).map(str::to_string).collect();
    let mut all: Vec<&String> = vehicles.iter().flat_map(|v| v.hofs.iter()).collect();
    all.sort_by_key(|h| h.to_lowercase());
    for h in &all {
        let lh = h.to_lowercase();
        if words.iter().any(|w| lh.contains(w.as_str())) {
            push(h, &mut out);
        }
    }
    if out.is_empty() {
        for h in all {
            push(h, &mut out);
        }
    }
    out
}

fn map_label(m: &core::MapInfo) -> String {
    if m.friendly.trim().is_empty() {
        m.name.clone()
    } else {
        m.friendly.clone()
    }
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(mut w) = l.company.wizard.take() else { return };
    let has = l.company.companies.as_ref().is_some_and(|c| !c.is_empty());
    // the head: what this is
    l.ui.text_in("Found your bus company", Rect::new(area.x, area.y, area.w, 32.0), 24.0, Weight::Bold, TEXT, Align::Left);
    let ih = l.ui.paragraph("Run your own transport company on a map: buy, lease or rent buses, hire drivers and run the map's lines or your own. The company has its own clock: simulate its time by the hour or by days, and what happens is told as it comes; what you drive yourself counts as it was driven.", Vec2::new(area.x, area.y + 40.0), area.w.min(1100.0), kit::BODY, Weight::Regular, TEXT_SOFT);
    let top = area.y + 40.0 + ih + 18.0;
    let gap = 18.0;
    let col_w = (area.w - gap) / 2.0;
    let h = (area.bottom() - top - 64.0).max(200.0);
    // who the company is
    let left = section(&mut l.ui, Rect::new(area.x, top, col_w, h), "The company");
    let mut y = left.y;
    l.ui.label(Rect::new(left.x, y, left.w, 20.0), "Name");
    y += 24.0;
    l.ui.text_input("company-name", Rect::new(left.x, y, left.w, 40.0), &mut w.name, "Stadtbus Grundorf", None);
    y += 40.0 + 16.0;
    let half = (left.w - GAP) / 2.0;
    l.ui.label(Rect::new(left.x, y, left.w, 20.0), "Short name (plates and logo)");
    y += 24.0;
    let suggested = co::short_of(&w.name);
    let placeholder = if suggested.is_empty() { "SG".to_string() } else { suggested };
    l.ui.text_input("company-short", Rect::new(left.x, y, half, 40.0), &mut w.short, &placeholder, None);
    if w.short.chars().count() > 4 {
        w.short = w.short.chars().take(4).collect();
    }
    y += 40.0 + 16.0;
    for (k, title) in ["Main colour", "Second colour"].iter().enumerate() {
        l.ui.label(Rect::new(left.x, y, left.w, 20.0), title);
        y += 26.0;
        let sw = ((left.w - 11.0 * 6.0) / 12.0).min(34.0);
        for (i, hex) in PALETTE.iter().enumerate() {
            let r = Rect::new(left.x + i as f32 * (sw + 6.0), y, sw, sw);
            let (hov, _, clicked) = l.ui.interact(super::super::ui::id_of(&format!("company-colour-{k}-{i}")), r);
            l.ui.p().rounded(r, 6.0, super::super::ownlines::colour_of(hex));
            if w.colours[k] == i {
                l.ui.p().rounded_border(r.inset(-3.0), 8.0, 2.0, TEXT);
            } else if hov {
                l.ui.p().rounded_border(r.inset(-2.0), 7.0, 1.0, TEXT_DIM);
            }
            if clicked {
                w.colours[k] = i;
            }
        }
        y += sw + 16.0;
    }
    // the mark as it will look, and a logo picture
    let preview = co::Company { short: if w.short.trim().is_empty() { placeholder.clone() } else { w.short.trim().to_uppercase() }, colours: [PALETTE[w.colours[0]].into(), PALETTE[w.colours[1]].into()], logo: w.logo.clone(), ..co::found(&co::Founding::default(), "") };
    super::company_mark(l, Rect::new(left.x, y, 68.0, 68.0), &preview);
    let logo_text = w.logo.as_deref().map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()).unwrap_or_else(|| omsi_ui::tr("No logo picture: the short name is the mark.").into_owned());
    l.ui.text_in(&logo_text, Rect::new(left.x + 84.0, y + 2.0, left.w - 84.0, 22.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
    let lw = Foot::width(&l.ui, "Choose a logo picture", Some("photo_camera")).min(left.w - 84.0);
    if l.ui.button("company-logo", Rect::new(left.x + 84.0, y + 30.0, lw, 36.0), "Choose a logo picture", Some("photo_camera"), ButtonKind::Normal) {
        if let Some(p) = core::pick_file("Choose a logo picture") {
            w.logo = Some(p.to_string_lossy().to_string());
        }
    }
    if w.logo.is_some() {
        let rw = Foot::width(&l.ui, "Remove the picture", Some("close")).min((left.w - 84.0 - lw - 10.0).max(0.0));
        if rw > 60.0 && l.ui.button("company-logo-remove", Rect::new(left.x + 84.0 + lw + 10.0, y + 30.0, rw, 36.0), "Remove the picture", Some("close"), ButtonKind::Normal) {
            w.logo = None;
        }
    }
    // where and how
    let rx = area.x + col_w + gap;
    let right = section(&mut l.ui, Rect::new(rx, top, col_w, h), "Home and difficulty");
    let mut y = right.y;
    let maps: Vec<core::MapInfo> = l.state.maps.clone();
    if maps.is_empty() {
        l.ui.paragraph("No maps found: the company needs a map to be at home on.", Vec2::new(right.x, y), right.w, kit::BODY, Weight::Regular, WARN);
    } else {
        w.map = w.map.min(maps.len() - 1);
        l.ui.label(Rect::new(right.x, y, right.w * 0.5, 20.0), "Home map");
        l.ui.label(Rect::new(right.x + right.w * 0.5 + GAP * 0.5, y, right.w * 0.5 - GAP * 0.5, 20.0), "Depot (the buses' depot file)");
        y += 24.0;
        let names: Vec<String> = maps.iter().map(map_label).collect();
        let mut m = w.map;
        if l.ui.select("company-map", Rect::new(right.x, y, right.w * 0.5 - GAP * 0.5, 40.0), &mut m, &names) {
            w.map = m;
            w.depot = 0;
        }
        let depots = depots_for(&maps[w.map], &l.state.vehicles);
        let dr = Rect::new(right.x + right.w * 0.5 + GAP * 0.5, y, right.w * 0.5 - GAP * 0.5, 40.0);
        if depots.is_empty() {
            l.ui.text_in("The map has no depot file.", dr, kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        } else {
            w.depot = w.depot.min(depots.len() - 1);
            let mut d = w.depot;
            if l.ui.select("company-depot", dr, &mut d, &depots) {
                w.depot = d;
            }
        }
        y += 40.0 + 16.0;
        // the first company day: today, the map's own date, or one of the player's own
        l.ui.label(Rect::new(right.x, y, right.w, 20.0), "First company day");
        y += 24.0;
        let labels: Vec<String> = ["Today", "The map's date", "A date of my own"].iter().map(|s| omsi_ui::tr(s).into_owned()).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let cw = l.ui.chips_height(right.w, 36.0, &refs);
        l.ui.chips("company-when", Rect::new(right.x, y, right.w, 36.0), &mut w.when, &refs);
        y += cw + 10.0;
        let first = first_day(w.when, maps.get(w.map), &l.state.choice.map, &l.state.choice.date, &w.date);
        if w.when == 2 {
            l.ui.date_field("company-date", Rect::new(right.x, y, 220.0, 40.0), &mut w.date);
            y += 50.0;
        } else {
            let t = omsi_ui::tr("The company begins on %{date}.").replace("%{date}", &super::day_label(&first));
            l.ui.text_in(&t, Rect::new(right.x, y, right.w, 24.0), kit::BODY, Weight::Medium, TEXT, Align::Left);
            y += 34.0;
        }
        y += 4.0;
    }
    l.ui.label(Rect::new(right.x, y, right.w, 20.0), "Difficulty");
    y += 26.0;
    let texts = [
        "Generous: grants on new buses, more passengers, few breakdowns, loans without interest.",
        "German city bus prices and wages, contracts with penalties, a margin of a few per cent.",
        "Tight: prices rise faster than the contract pays, dear loans, more breakdowns and illness.",
    ];
    let cw = (right.w - 2.0 * GAP) / 3.0;
    let ch = (right.bottom() - y - 96.0).clamp(140.0, 210.0);
    for (k, d) in Difficulty::ALL.iter().enumerate() {
        let r = Rect::new(right.x + k as f32 * (cw + GAP), y, cw, ch);
        let on = w.difficulty == k;
        if l.ui.row(&format!("company-difficulty-{k}"), r, false) {
            w.difficulty = k;
        }
        l.ui.p().rounded_border(r, RADIUS, if on { 2.0 } else { 1.0 }, if on { accent() } else { HAIRLINE });
        l.ui.text_in(d.label(), Rect::new(r.x + 14.0, r.y + 12.0, r.w - 28.0, 24.0), kit::HEAD, Weight::Bold, if on { TEXT } else { TEXT_SOFT }, Align::Left);
        let capital = co::economy::rules(*d).start_capital;
        l.ui.text_in(&eur(capital), Rect::new(r.x + 14.0, r.y + 38.0, r.w - 28.0, 22.0), kit::ROWS, Weight::Bold, LINE, Align::Left);
        l.ui.paragraph(texts[k], Vec2::new(r.x + 14.0, r.y + 66.0), r.w - 28.0, kit::NOTE, Weight::Regular, TEXT_SOFT);
    }
    // how it buys its buses
    let y = y + ch + 18.0;
    l.ui.label(Rect::new(right.x, y, right.w, 20.0), "Buying buses");
    let modes: Vec<String> = co::dealer::BuyingMode::ALL.iter().map(|m| omsi_ui::tr(m.label()).into_owned()).collect();
    let refs: Vec<&str> = modes.iter().map(String::as_str).collect();
    let sw = (right.w * 0.4).clamp(200.0, 280.0);
    l.ui.segmented("company-buying", Rect::new(right.x, y + 26.0, sw, 38.0), &mut w.buying, &refs);
    let say = if w.buying == 0 { "A model, a number, the list price: the buses are yours at once." } else { "Haggle with the dealer, agree on extras and sign a contract; new buses are delivered." };
    l.ui.paragraph(say, Vec2::new(right.x + sw + GAP, y + 26.0), right.w - sw - GAP, kit::NOTE, Weight::Regular, TEXT_SOFT);
    // found it
    let by = area.bottom() - 44.0;
    let ok = !w.name.trim().is_empty() && !maps.is_empty();
    let fw = Foot::width(&l.ui, "Found the company", Some("check_circle"));
    let found_r = Rect::new(area.right() - fw, by, fw, 44.0);
    if has && l.ui.button("company-wizard-cancel", Rect::new(found_r.x - 12.0 - 140.0, by, 140.0, 44.0), "Cancel", None, ButtonKind::Normal) {
        l.company.wizard = None;
        return;
    }
    if !ok {
        l.ui.text_in("Give the company a name.", Rect::new(area.x, by, area.w - 460.0, 44.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
    }
    if l.ui.button("company-found", found_r, "Found the company", Some("check_circle"), ButtonKind::Primary) {
        if !ok {
            let p = if maps.is_empty() {
                kit::Popup::new("map", "No map", omsi_ui::tr("The company needs a map to be at home on: none was found in the OMSI folder."), omsi_ui::tr("Install a map, or choose the OMSI folder under Setup."), None)
            } else {
                kit::Popup::new("badge", "A name first", omsi_ui::tr("The company needs a name: it is on its buses, its papers and its contracts."), omsi_ui::tr("Type one in the field \"Name\"; the short name is made of it."), None)
            };
            kit::show(l, p);
            l.company.wizard = Some(w);
            return;
        }
        let m = &maps[w.map];
        let depots = depots_for(m, &l.state.vehicles);
        let f = co::Founding {
            name: w.name.trim().to_string(),
            short: w.short.clone(),
            colours: [PALETTE[w.colours[0]].to_string(), PALETTE[w.colours[1]].to_string()],
            logo: w.logo.clone(),
            map: m.file.clone(),
            map_name: map_label(m),
            depot: depots.get(w.depot).cloned().unwrap_or_default(),
            date: first_day(w.when, Some(m), &l.state.choice.map, &l.state.choice.date, &w.date),
            difficulty: Difficulty::ALL[w.difficulty.min(2)],
            buying: co::dealer::BuyingMode::ALL[w.buying.min(1)],
        };
        let mut c = co::found(&f, &l.state.config.profile);
        c.id = co::store::unused_id(&data(), &c.name);
        l.company.company = Some(c);
        l.company.tab = 0;
        super::changed(l);
        // (the company's tour goes on with its pages, if the welcome came before the founding)
        super::tutorial::founded(l);
        l.state.set_status(omsi_ui::tr("%{name} is founded. Add a line, buy a bus and hire drivers.").replace("%{name}", &f.name), false);
        return;
    }
    l.company.wizard = Some(w);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(file: &str, hof: &str) -> core::MapInfo {
        core::MapInfo { name: "Grundorf".into(), friendly: "Grundorf".into(), file: file.into(), description: String::new(), entry_points: Vec::new(), hof: hof.into(), installed: false }
    }

    fn bus(hofs: &[&str]) -> core::VehicleInfo {
        core::VehicleInfo {
            name: "Bus".into(),
            manufacturer: String::new(),
            type_name: String::new(),
            file: "Vehicles/Bus/bus.bus".into(),
            folder: "Bus".into(),
            description: String::new(),
            default_paint: String::new(),
            paints: Vec::new(),
            hofs: hofs.iter().map(|s| s.to_string()).collect(),
            installed: false,
            missing_packs: Vec::new(),
            numbers: Vec::new(),
        }
    }

    #[test]
    fn the_first_day_is_today_or_the_maps() {
        // today, as the computer has it in its own time zone (not the UTC date, which is
        // another day around midnight)
        let t = today();
        if let Some((y, m, d, _, _)) = omsi_launcher_lib::local_now() {
            assert_eq!(t, format!("{y:04}-{m:02}-{d:02}"));
        }
        assert!(co::dates::parse(&t).is_some(), "{t}");
        assert_eq!(first_day(0, None, "", "", ""), t);
        // the map's: the launcher's date when it is the map chosen there, else the year in its
        // name on the game's day, else the game's first day
        let mut bh = map("maps/Bad_Huegelsdorf_2020/global.cfg", "VBBH");
        bh.name = "Bad_Huegelsdorf_2020".into();
        assert_eq!(map_date(&bh, "maps/Bad_Huegelsdorf_2020/global.cfg", "2005-10-12"), "2005-10-12");
        assert_eq!(map_date(&bh, "maps/Other/global.cfg", "2005-10-12"), "2020-05-30");
        assert_eq!(map_date(&map("maps/Grundorf/global.cfg", ""), "", ""), core::DEFAULT_DATE);
        assert_eq!(first_day(1, Some(&bh), "", "", ""), "2020-05-30");
        // one of the player's own, when it is a date
        assert_eq!(first_day(2, None, "", "", "1999-01-02"), "1999-01-02");
        assert_eq!(first_day(2, None, "", "", "nonsense"), t);
    }

    #[test]
    fn the_depots_of_a_map_come_first() {
        let m = map("maps/Grundorf/global.cfg", "Grundorf");
        let v = vec![bus(&["Spandau", "Grundorf_Linie"]), bus(&["grundorf"])];
        assert_eq!(depots_for(&m, &v), vec!["Grundorf", "Grundorf_Linie"]);
        // a map whose name no depot file has: all of them
        let other = map("maps/Neustadt/global.cfg", "");
        let mut o = other.clone();
        o.name = "Neustadt".into();
        o.friendly = "Neustadt".into();
        assert_eq!(depots_for(&o, &v), vec!["grundorf", "Grundorf_Linie", "Spandau"]);
    }
}
