//! Readable names for the cockpit: what the HUD says about the switch or part under the
//! cursor. The scripts and models only know internal, mostly German names
//! (`cp_batterietrennschalter_toggle`, `Zahltisch_Wechsler_0_05`), so a name is looked up
//! the way OMSI's own key assignment dialog does it - `Languages/<LANG>_key_veh_gen*.olf`
//! gives the text of every trigger the keyboard can reach (`KY_<trigger>`) - and whatever
//! the language files do not know is translated word by word with the common OMSI cockpit
//! vocabulary below (German compounds are split into the words they are made of).

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// The trigger texts of one language, keys in lower case without the `KY_`.
pub struct ControlNames {
    /// `ENG`, `DEU` or `FRA`.
    pub lang: String,
    texts: HashMap<String, String>,
    /// Original spelling of each `KY_<event>` name, by its lower-case lookup key.
    spellings: HashMap<String, String>,
}

/// How far the scan of the installed buses' scripts is (`scan_script_actions`), and the
/// actions it found since the last update with the bus file each came from.
#[derive(Clone, Debug, Default)]
pub struct ScriptActionScanUpdate {
    pub done: usize,
    pub total: usize,
    pub current: String,
    pub discovered: Vec<(String, String)>,
    pub complete: bool,
}

impl ControlNames {
    /// Read the key assignment texts of `lang` from the installation (and the mods' copies).
    pub fn load(root: &Path, lang: &str) -> ControlNames {
        // (OMSI's cockpit names are in English, German and French: every other language
        // reads the English ones - the interface around them is translated)
        let lang = match language_code(lang).as_str() {
            l @ ("DEU" | "FRA") => l.to_string(),
            _ => "ENG".to_string(),
        };
        let mut texts = HashMap::new();
        let mut spellings = HashMap::new();
        let mut dirs = omsi_cfg::content_dirs("Languages");
        dirs.reverse(); // the installation first, the mods' files override it
        let own = root.join("Languages");
        if !dirs.iter().any(|d| d == &own) {
            dirs.insert(0, own);
        }
        for dir in dirs {
            // (through the content file system: a mod's language files may be in an archive)
            let Some(list) = omsi_cfg::vfs::list_dir(&dir) else { continue };
            let mut files: Vec<std::path::PathBuf> = list
                .into_iter()
                .filter(|(_, is_dir)| !*is_dir)
                .map(|(n, _)| dir.join(n))
                .filter(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    n.starts_with(&format!("{}_key_", lang.to_ascii_lowercase())) && n.ends_with(".olf")
                })
                .collect();
            files.sort();
            for f in files {
                if let Ok(l) = omsi_content::language::Language::load(&f) {
                    for (k, v) in l.strings {
                        if let Some(name) = k.strip_prefix("KY_").or_else(|| k.strip_prefix("ky_")) {
                            let v = v.trim();
                            if !v.is_empty() && !v.starts_with('<') {
                                let key = name.to_ascii_lowercase();
                            texts.insert(key.clone(), v.to_string());
                            spellings.insert(key, name.to_string());
                            }
                        }
                    }
                }
            }
        }
        log::info!("control names: {} texts in {lang}", texts.len());
        ControlNames { lang, texts, spellings }
    }

    /// The actions OMSI's key assignment dialog can show, paired with their readable names.
    pub fn actions(&self) -> Vec<(String, String)> {
        let mut actions: Vec<_> = self.texts.iter()
            .map(|(action, label)| (action.clone(), label.clone()))
            .collect();
        actions.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
        actions
    }

    /// Trigger names compiled from every installed vehicle's scripts, including mod buses.
    /// Progress updates carry each action's bus-file provenance as it is found.
    pub fn scan_script_actions(mut report: impl FnMut(ScriptActionScanUpdate)) {
        let mut buses = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for vehicles in omsi_cfg::content_dirs("Vehicles") {
            for pack in omsi_cfg::vfs::read_dir_paths(&vehicles) {
                if !omsi_cfg::vfs::is_dir(&pack) {
                    continue;
                }
                for path in omsi_cfg::vfs::read_dir_paths(&pack) {
                    let is_vehicle = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("bus") || ext.eq_ignore_ascii_case("ovh"));
                    if !is_vehicle {
                        continue;
                    }
                    let key = path.strip_prefix(&vehicles).unwrap_or(&path).to_string_lossy().replace('\\', "/").to_ascii_lowercase();
                    if seen.insert(key) {
                        let source = vehicle_source(&path, &vehicles);
                        buses.push((path, source));
                    }
                }
            }
        }
        buses.sort_by_key(|b| b.1.to_lowercase());
        scan_bus_scripts(buses, &mut report);
    }

    /// The same names from a table (tests).
    #[cfg(test)]
    pub fn from_table(lang: &str, table: &[(&str, &str)]) -> ControlNames {
        ControlNames {
            lang: language_code(lang),
            texts: table.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect(),
            spellings: table.iter().map(|(k, _)| (k.to_ascii_lowercase(), k.to_string())).collect(),
        }
    }

    fn text(&self, trigger: &str) -> Option<&str> {
        self.texts.get(&trigger.to_ascii_lowercase()).map(|s| s.as_str())
    }

    /// Every event OMSI exposes in `Languages/<LANG>_key_veh_gen*.olf`, as
    /// (event name without `KY_`, readable label). Mods may add their own files, so this
    /// is the same pool the original key-assignment "Add event..." dialog draws from.
    pub fn events(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .texts
            .iter()
            .map(|(key, label)| {
                (
                    self.spellings.get(key).cloned().unwrap_or_else(|| key.clone()),
                    label.clone(),
                )
            })
            .collect();
        out.sort_by(|a, b| a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase()).then_with(|| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase())));
        out
    }

    /// What a `[mouseevent]` does, for the HUD.
    pub fn control(&self, event: &str) -> String {
        let event = event.trim();
        if let Some(t) = self.official(event) {
            return t;
        }
        // variants of a trigger the key dialog knows: the mouse version of a key, a second
        // button for the same job, the outside button of a door
        for (suffix, extra) in [("_mouse", ""), ("_external", " (outside)"), ("_2", " 2"), ("_sw", ""), ("_button", "")] {
            if let Some(base) = strip_suffix_ci(event, suffix) {
                if let Some(t) = self.official(base) {
                    return format!("{t}{}", if self.lang == "ENG" { extra } else { "" });
                }
            }
        }
        if self.lang == "DEU" {
            return humanize(event);
        }
        translate(event)
    }

    /// The official text of a trigger, also under the names OMSI uses for the same job in
    /// the key and the mouse version (`kw_` / `cp_`, with or without `_toggle`).
    fn official(&self, event: &str) -> Option<String> {
        let mut names = vec![event.to_string(), format!("{event}_toggle")];
        let lower = event.to_ascii_lowercase();
        for (a, b) in [("kw_", "cp_"), ("cp_", "kw_")] {
            if let Some(rest) = lower.strip_prefix(a) {
                names.push(format!("{b}{rest}"));
                names.push(format!("{b}{rest}_toggle"));
            }
        }
        names.iter().find_map(|n| self.text(n)).map(str::to_string)
    }

    /// A part of the bus that is not a control, from its mesh file name.
    pub fn part(&self, mesh_stem: &str) -> String {
        if self.lang == "DEU" {
            return humanize(mesh_stem);
        }
        translate(mesh_stem)
    }
}

fn scan_bus_scripts(
    buses: Vec<(std::path::PathBuf, String)>,
    report: &mut impl FnMut(ScriptActionScanUpdate),
) {
    let mut trigger_cache: HashMap<String, Vec<String>> = HashMap::new();
    let mut pending = Vec::new();
    let total = buses.len();
    if total == 0 {
        report(ScriptActionScanUpdate { complete: true, ..Default::default() });
        return;
    }
    for (index, (bus, source)) in buses.into_iter().enumerate() {
        let done = index + 1;
        let mut source_name = source.clone();
        if let Ok(vehicle) = omsi_vehicle::Vehicle::load(&bus) {
            let friendly = [vehicle.manufacturer.trim(), vehicle.type_name.trim()]
                .into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
            if !friendly.is_empty() {
                source_name = format!("{source} ({friendly})");
            }
            let scripts = &vehicle.scripts.scripts;
            let key = scripts.iter().map(|p| p.to_string_lossy().to_ascii_lowercase()).collect::<Vec<_>>().join("\0");
            let actions = trigger_cache.entry(key).or_insert_with(|| script_triggers(scripts));
            pending.extend(actions.iter().cloned().map(|action| (action, source_name.clone())));
        }
        let current = source_name;
        if done % 8 == 0 || done == total {
            report(ScriptActionScanUpdate {
                done,
                total,
                current,
                discovered: std::mem::take(&mut pending),
                complete: done == total,
            });
        }
    }
}

fn vehicle_source(path: &Path, vehicles: &Path) -> String {
    path.strip_prefix(vehicles).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn script_triggers(scripts: &[std::path::PathBuf]) -> Vec<String> {
    omsi_script::compile(&omsi_script::CompileInput { scripts: scripts.to_vec(), ..Default::default() }).trigger_names()
}

#[cfg(test)]
mod script_action_tests {
    use super::*;

    #[test]
    fn discovers_custom_triggers_from_a_bus_script() {
        let dir = std::env::temp_dir().join(format!(
            "openomsi_key_actions_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let vehicles = dir.join("Vehicles");
        assert_eq!(
            vehicle_source(&vehicles.join("VehiclePack/Vehicle.bus"), &vehicles),
            "VehiclePack/Vehicle.bus",
        );
        let omsi_root = dir.join("OMSI");
        assert_eq!(
            vehicle_source(&omsi_root.join("Vehicles/DefaultBus/default.bus"), &omsi_root.join("Vehicles")),
            "DefaultBus/default.bus",
        );
        let bus = dir.join("Vehicle.bus");
        std::fs::write(&bus, "[script]\n1\nmain.osc\n").unwrap();
        std::fs::write(
            dir.join("main.osc"),
            "{trigger:cruise_control_toggle}\n{end}\n{trigger:ivu_ticket_cancel}\n{end}\n",
        ).unwrap();

        let mut updates = Vec::new();
        scan_bus_scripts(
            vec![(bus, "VehiclePack/Vehicle.bus".into())],
            &mut |update| updates.push(update),
        );
        assert_eq!(updates.len(), 1);
        assert!(updates[0].complete);
        assert_eq!((updates[0].done, updates[0].total), (1, 1));
        assert_eq!(updates[0].current, "VehiclePack/Vehicle.bus");
        assert_eq!(updates[0].discovered, [
            ("cruise_control_toggle".into(), "VehiclePack/Vehicle.bus".into()),
            ("ivu_ticket_cancel".into(), "VehiclePack/Vehicle.bus".into()),
        ]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// `ENG` / `DEU` / `FRA` from the settings' spelling (default English).
pub fn language_code(s: &str) -> String {
    omsi_launcher_lib::language_code(s).to_string()
}

static NAMES: OnceLock<ControlNames> = OnceLock::new();

/// The process-wide names (the language is read once, from the first caller).
pub fn names(root: &Path, lang: &str) -> &'static ControlNames {
    NAMES.get_or_init(|| ControlNames::load(root, lang))
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    (s.len() > suffix.len() && s.is_char_boundary(s.len() - suffix.len()) && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)).then(|| &s[..s.len() - suffix.len()])
}

/// The internal name with its separators turned into spaces (German texts).
pub fn humanize(name: &str) -> String {
    let words: Vec<String> = tokens(name).into_iter().filter(|t| !matches!(t.to_ascii_lowercase().as_str(), "cp" | "kw" | "bus" | "toggle" | "mouse" | "lod")).collect();
    sentence(&words.join(" "))
}

/// Split an internal name into words: separators, camel case and digit runs.
fn tokens(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        let sep = matches!(c, '_' | '-' | ' ' | '.' | '\\' | '/');
        let boundary = match prev {
            Some(p) => (p.is_ascii_digit() != c.is_ascii_digit()) || (p.is_lowercase() && c.is_uppercase()),
            None => false,
        };
        if sep || (boundary && !cur.is_empty()) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
        if !sep {
            cur.push(c);
        }
        prev = if sep { None } else { Some(c) };
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// German spelling folded the way the file names write it.
fn fold(word: &str) -> String {
    word.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss")
}

/// The cockpit and body vocabulary of OMSI's stock and add-on buses (folded German or
/// internal English → English). An empty translation drops the word.
const WORDS: &[(&str, &str)] = &[
    // technical prefixes and noise
    ("cp", ""), ("kw", ""), ("m", ""), ("bus", ""), ("lod", ""), ("mouse", ""), ("mov", ""), ("komplett", ""), ("generic", ""),
    // doors and body
    ("tuer", "door"), ("tueren", "doors"), ("door", "door"), ("doors", "doors"), ("doorfront", "front door"), ("dooraft", "rear door"),
    ("tuerfluegel", "door leaf"), ("fluegel", "leaf"), ("tuerbuegel", "door grab rail"), ("buegel", "grab rail"), ("tuerlappen", "door flap"),
    ("tuerscheibe", "door window"), ("tuerfenst", "door window"), ("tuertaster", "door button"), ("tuerb", "door"), ("tuerext", "door outside"), ("tuerint", "door inside"),
    ("fahrertuer", "driver's door"), ("fahrgastpendel", "passenger swing gate"), ("fahrgpend", "passenger swing gate"), ("pendel", "swing gate"),
    ("aussenoeffner", "outside opener"), ("oeffner", "opener"), ("entriegelung", "release"), ("nothahn", "emergency door valve"), ("notheben", "emergency lift"),
    ("klappe", "flap"), ("frontklappe", "front flap"), ("tauschklappe", "exchange flap"), ("wagenkasten", "body"), ("kasten", "box"), ("body", "body"),
    ("dach", "roof"), ("dachluke", "roof hatch"), ("luke", "hatch"), ("heck", "rear"), ("heckscheibe", "rear window"), ("heckl", "rear light"),
    ("frontscheibe", "windscreen"), ("frontgrill", "front grille"), ("scheibe", "window"), ("scheiben", "windows"), ("fenster", "window"), ("fensterrahmen", "window frame"),
    ("klappfenster", "tilting window"), ("fahrerfenster", "driver's window"), ("innenraumscheibe", "interior window"), ("trennscheibe", "partition"), ("trennscheiben", "partitions"),
    ("cockpitscheibe", "cockpit window"), ("quersitzscheiben", "side seat screens"), ("fahrradscheiben", "bicycle area screens"), ("taucherbrille", "front window"),
    ("rahmen", "frame"), ("gelenk", "articulation"), ("knick", "articulation"), ("articul", "articulation"), ("bellows", "bellows"), ("drehgestell", "bogie"), ("unterbau", "underbody"),
    ("fahrgestell", "chassis"), ("achse", "axle"), ("rad", "wheel"), ("wheelcap", "wheel cap"), ("kupplung", "coupling"), ("stange", "pole"), ("stangen", "poles"),
    ("griff", "handle"), ("hebel", "lever"), ("platte", "plate"), ("basis", "base"), ("unterlage", "pad"), ("polster", "upholstery"), ("kunstleder", "leatherette"),
    ("sitz", "seat"), ("sitze", "seats"), ("seats", "seats"), ("klappsitz", "folding seat"), ("fahrersitz", "driver's seat"), ("innenraum", "interior"), ("interior", "interior"),
    ("innen", "inside"), ("aussen", "outside"), ("innenwaende", "interior walls"), ("waende", "walls"), ("decke", "ceiling"), ("boden", "floor"), ("stufe", "step"),
    // driver's controls and instruments
    ("panel", "panel"), ("cockpit", "cockpit"), ("lenkrad", "steering wheel"), ("lenksaeule", "steering column"), ("bremspedal", "brake pedal"), ("fahrpedal", "accelerator pedal"),
    ("pedal", "pedal"), ("feststellbremse", "parking brake"), ("hstbremse", "stop brake"), ("haltestellenbremse", "stop brake"), ("hst", "stop"), ("bremse", "brake"),
    ("bremsdruck", "brake pressure"), ("vorratsdruck", "reservoir pressure"), ("druckwarnung", "pressure warning"), ("druck", "pressure"), ("oeldruck", "oil pressure"),
    ("kuehlwasser", "coolant temperature"), ("tankuhr", "fuel gauge"), ("tachonadel", "speedometer needle"), ("tacho", "speedometer"), ("nadel", "needle"),
    ("kilometer", "odometer"), ("gangwahl", "gear selector"), ("schalter", "switch"), ("taster", "button"), ("knopf", "button"), ("drehschalter", "rotary switch"),
    ("mikroschalter", "micro switch"), ("trennschalter", "master switch"), ("batterietrennschalter", "battery master switch"), ("batterie", "battery"),
    ("zuendung", "ignition"), ("anlasser", "starter"), ("schluessel", "ignition key"), ("motorabstellung", "engine stop"), ("motor", "engine"), ("motorkuehlung", "engine cooling"),
    ("blinker", "indicator"), ("blinkerhebel", "indicator lever"), ("warnblinker", "hazard lights"), ("warn", "hazard lights"), ("licht", "light"), ("leuchte", "lamp"),
    ("leuchten", "lamps"), ("lampe", "lamp"), ("fahrerlicht", "driver's light"), ("fahrerleuchte", "driver's lamp"), ("fernlicht", "full beam"), ("fernlichtschalter", "full beam switch"),
    ("abblendlicht", "dipped beam"), ("scheinwerfer", "headlights"), ("standlicht", "parking light"), ("nebelschluss", "rear fog light"), ("nebelschlussleuchte", "rear fog light"),
    ("innenbeleuchtung", "interior light"), ("beleuchtung", "lighting"), ("lichtstufe", "light level"), ("hupe", "horn"), ("hupknopf", "horn button"), ("horn", "horn"),
    ("wischer", "wiper"), ("wiper", "wiper"), ("wischerarm", "wiper arm"), ("wischerblatt", "wiper blade"), ("wischerhebel", "wiper lever"), ("wiperlever", "wiper lever"),
    ("wipermode", "wiper mode"), ("wascher", "washer"), ("wischwasser", "washer fluid"), ("intervall", "intermittent"), ("schnell", "fast"), ("turnswitch", "rotary switch"),
    ("heizung", "heating"), ("heiz", "heating"), ("heizregler", "heating control"), ("regler", "control"), ("heizluefter", "heater fan"), ("luefter", "fan"),
    ("geblaese", "blower"), ("standheizung", "parking heater"), ("defrost", "defrost"), ("fussraum", "footwell"), ("umluft", "recirculation"), ("bug", "front"),
    ("temp", "temperature"), ("misch", "mix"), ("klimator", "roof ventilator"), ("klima", "air conditioning"), ("spiegel", "mirror"), ("spiegelheizung", "mirror heating"),
    ("rollo", "sun blind"), ("sonnenblende", "sun visor"), ("retarder", "retarder"), ("direkt", "direct"), ("kneeling", "kneeling"), ("kneel", "kneeling"), ("autokneel", "automatic kneeling"),
    ("rampe", "ramp"), ("pandus", "ramp"), ("ramplift", "ramp / lift"), ("hub", "lift"), ("hublift", "lift"), ("anheben", "raise"), ("wunsch", "request"),
    ("haltewunsch", "stop request"), ("haltewunschtaster", "stop request button"), ("haltewunschknopf", "stop request button"), ("stoptaster", "stop button"),
    ("kinderwagen", "buggy"), ("kinderwagenknopf", "buggy button"), ("kinderwagenwunsch", "buggy request"), ("microphone", "microphone"), ("mikrofon", "microphone"),
    ("kassettenrekorder", "cassette recorder"), ("kr", "cassette recorder"), ("uhr", "clock"), ("hour", "hour hand"), ("minute", "minute hand"), ("second", "second hand"),
    ("thermometer", "thermometer"), ("raendel", "thumb wheel"), ("fahrer", "driver's"), ("fahrgast", "passenger"), ("fahrrad", "bicycle"),
    ("parking", "parking"), ("brake", "brake"), ("engine", "engine"), ("startbutton", "start button"), ("engineshutdown", "engine shutdown"), ("automatic", "automatic gearbox"),
    ("indic", "indicator"), ("ind", "indicator"), ("rpm", "rev counter"), ("airpress", "air pressure"), ("asr", "ASR"),
    // tickets, money, information system
    ("fahrschein", "ticket"), ("ticket", "ticket"), ("ausgabe", "dispenser"), ("drucker", "ticket printer"), ("ticketprinter", "ticket printer"), ("getticket", "take ticket"),
    ("entwerter", "ticket validator"), ("zahltisch", "cash desk"), ("kassentisch", "cash desk"), ("kasse", "cash desk"), ("cashdesk", "cash desk"), ("wechsler", "coin changer"),
    ("changer", "coin changer"), ("geld", "money"), ("money", "money"), ("muenze", "coin"), ("ziel", "destination"), ("linie", "line"), ("lin", "line"), ("kurs", "tour"),
    ("route", "route"), ("eingabe", "enter"), ("loeschen", "delete"), ("vor", "forward"), ("rueck", "back"), ("stumm", "mute"), ("setmode", "mode"), ("fortschaltung", "advance"),
    ("matrix", "destination display"), ("vollmatrix", "destination display"), ("innenanzeige", "interior display"), ("anzeige", "display"), ("display", "display"), ("disp", "display"),
    ("seitenschild", "side sign"), ("seitenschildklemme", "side sign clamp"), ("seitenschklemme", "side sign clamp"), ("klemme", "clamp"), ("steckschild", "plug-in sign"),
    ("schild", "sign"), ("schildern", "signs"), ("rollband", "rollsign"), ("rlbnd", "rollsign"), ("rlb", "rollsign"), ("fallblatt", "flip-leaf display"), ("ibis", "IBIS"),
    ("vdv", "VDV"), ("kennz", "number plate"), ("kennzeichen", "number plate"), ("wagennummer", "fleet number"), ("wagennr", "fleet number"), ("nummer", "number"),
    ("beschriftungen", "lettering"), ("text", "text"), ("textfeld", "text field"), ("schedule", "timetable"), ("fahrplan", "timetable"), ("karte", "card"),
    // modes, locks and add-on equipment (Citaro and other mod buses)
    ("sperre", "lock"), ("tuersperre", "door lock"), ("wegfahrsperre", "immobiliser"), ("verriegelung", "lock"), ("freigabe", "release"),
    ("schulfahrt", "training run"), ("schulfahr", "training run"), ("fahrschule", "driving school"), ("funktion", "function"), ("funktionen", "functions"),
    ("modus", "mode"), ("betrieb", "operation"), ("ansage", "announcement"), ("ansagen", "announcements"), ("lautsprecher", "loudspeaker"),
    ("lautstaerke", "volume"), ("funk", "radio"), ("sprechfunk", "radio"), ("notaus", "emergency stop"), ("nothalt", "emergency stop"),
    ("notbremse", "emergency brake"), ("haltestelle", "stop"), ("frost", "frost"), ("frostschutz", "frost protection"), ("kasetka", "cash box"),
    ("comp", "computer"), ("qiut", "quit"), ("quit", "quit"), ("info", "info"), ("system", "system"), ("menu", "menu"), ("rbl", "RBL"),
    ("key", "key"), ("rot", "turn"), ("window", "window"), ("wheelchair", "wheelchair"), ("rollstuhl", "wheelchair"), ("camera", "camera"),
    ("heat", "heating"), ("air", "air"), ("internal", "recirculation"), ("power", "power"), ("ok", "OK"), ("oeffnen", "open"), ("schliessen", "close"),
    // other parts
    ("wimpel", "pennant"), ("bommel", "tassel"), ("dreck", "dirt"), ("dreckmesh", "dirt"), ("dirt", "dirt"), ("schatten", "shadow"), ("shadow", "shadow"),
    ("dummy", "unused"), ("dum", "unused"), ("blindwelle", "dummy shaft"), ("blaulicht", "blue light"), ("sticker", "sticker"), ("glas", "glass"), ("glass", "glass"),
    // directions, states, positions
    ("links", "left"), ("rechts", "right"), ("vorne", "front"), ("vorn", "front"), ("hinten", "rear"), ("oben", "top"), ("unten", "bottom"), ("mitte", "middle"),
    ("untenrechts", "lower right"), ("obenrechts", "upper right"), ("untenlinks", "lower left"), ("obenlinks", "upper left"), ("oberdeck", "upper deck"), ("unterdeck", "lower deck"),
    ("auf", "up"), ("ab", "down"), ("dn", "down"), ("up", "up"), ("down", "down"), ("frei", "release"), ("voll", "full"), ("leer", "empty"), ("quer", "transverse"),
    ("neu", "new"), ("new", "new"), ("alt", "old"), ("old", "old"), ("ist", "actual"), ("ext", "outer"), ("exterior", "outer"), ("int", "inner"), ("sw", "switch"),
    ("btn", "button"), ("button", "button"), ("switch", "switch"), ("mstr", "master"), ("master", "master"), ("opn", "open"), ("ovrd", "override"), ("select", "select"),
    ("start", "start"), ("stop", "stop"), ("play", "play"), ("cancel", "cancel"), ("enter", "enter"), ("retract", "retract"), ("sync", "synchronise"), ("change", "change"),
    ("external", "outside"), ("off", "off"), ("on", "on"), ("main", "main"), ("trail", "trailer"), ("front", "front"), ("rear", "rear"), ("back", "back"), ("center", "centre"),
    ("l", "left"), ("r", "right"), ("v", "front"), ("h", "rear"), ("vl", "front left"), ("vr", "front right"), ("hl", "rear left"), ("hr", "rear right"),
    ("lv", "left front"), ("lh", "left rear"), ("rv", "right front"), ("rh", "right rear"), ("ol", "upper left"), ("or", "upper right"), ("ul", "lower left"), ("ur", "lower right"),
];

/// Words that name the kind of control go last in English ("switch lift down" reads
/// "lift down switch"), and a side named last goes first ("mirror left" → "left mirror").
const DEVICE_WORDS: &[&str] = &["switch", "button", "lever", "rotary switch", "micro switch", "master switch"];
const SIDE_WORDS: &[&str] = &["left", "right", "front", "rear", "outer", "inner", "upper left", "upper right", "lower left", "lower right", "front left", "front right", "rear left", "rear right"];

fn dictionary() -> &'static HashMap<&'static str, &'static str> {
    static D: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    D.get_or_init(|| WORDS.iter().copied().collect())
}

/// A German compound as a chain of known words (longest first; `s`/`n` may join them).
fn split_compound(word: &str) -> Option<Vec<&'static str>> {
    fn go(rest: &str, depth: usize, d: &HashMap<&'static str, &'static str>) -> Option<Vec<&'static str>> {
        if rest.is_empty() {
            return Some(Vec::new());
        }
        if depth > 4 {
            return None;
        }
        for end in (3..=rest.len()).rev() {
            if !rest.is_char_boundary(end) {
                continue;
            }
            if let Some((k, v)) = d.get_key_value(&rest[..end]) {
                let _ = k;
                for skip in ["", "s", "n", "en"] {
                    if let Some(tail) = rest[end..].strip_prefix(skip) {
                        if tail.is_empty() && !skip.is_empty() {
                            continue;
                        }
                        if let Some(mut more) = go(tail, depth + 1, d) {
                            more.insert(0, *v);
                            return Some(more);
                        }
                    }
                }
            }
        }
        None
    }
    let parts = go(word, 0, dictionary())?;
    (parts.len() >= 2).then_some(parts)
}

/// Turn an internal name into an English phrase.
pub fn translate(name: &str) -> String {
    let d = dictionary();
    let toks = tokens(name);
    let mut words: Vec<String> = Vec::new();
    let mut on_off = false;
    // vehicle codes in front of a mesh name (SD_, D_, GN92_, EN92_, N92_ ...)
    let mut start = 0;
    while start + 1 < toks.len() {
        let t = toks[start].to_ascii_lowercase();
        let code = matches!(t.as_str(), "lod" | "sd" | "d" | "gn" | "en" | "n" | "nl" | "mb" | "man" | "o" | "generic") || (t.chars().all(|c| c.is_ascii_digit()) && start > 0 && toks[start - 1].len() <= 2);
        if !code {
            break;
        }
        start += 1;
    }
    let toks = &toks[start..];
    for (i, t) in toks.iter().enumerate() {
        let f = fold(t);
        if f == "toggle" {
            on_off = true;
            continue;
        }
        // the changer's coin values: `0_05` → 0.05
        if f.chars().all(|c| c.is_ascii_digit()) {
            if f.len() == 2 && i > 0 && toks[i - 1].chars().all(|c| c.is_ascii_digit()) && words.last().map(|w| w.chars().all(|c| c.is_ascii_digit())).unwrap_or(false) {
                let int = words.pop().unwrap_or_default();
                words.push(format!("{int}.{f}"));
            } else {
                words.push(f);
            }
            continue;
        }
        match d.get(f.as_str()) {
            Some(e) if e.is_empty() => {}
            Some(e) => words.push(e.to_string()),
            None => match split_compound(&f) {
                Some(parts) => words.extend(parts.into_iter().filter(|p| !p.is_empty()).map(str::to_string)),
                // keep what cannot be translated as it is written (IBIS, VDV, Almex, L0 ...)
                None => words.push(t.clone()),
            },
        }
    }
    // a doubled word from a compound and its neighbour ("door door leaf")
    words.dedup();
    // "switch lift down" → "lift down switch"
    if words.len() > 1 && DEVICE_WORDS.contains(&words[0].as_str()) {
        let w = words.remove(0);
        words.push(w);
    }
    // "driver's door left" → "left driver's door"
    if words.len() > 1 && SIDE_WORDS.contains(&words[words.len() - 1].as_str()) && !SIDE_WORDS.contains(&words[words.len() - 2].as_str()) {
        let w = words.pop().unwrap_or_default();
        words.insert(0, w);
    }
    let mut s = words.join(" ");
    if on_off {
        s.push_str(" on/off");
    }
    if s.trim().is_empty() {
        return humanize(name);
    }
    sentence(&s)
}

fn sentence(s: &str) -> String {
    let s = s.trim();
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eng() -> ControlNames {
        ControlNames::from_table(
            "ENG",
            &[
                ("cp_batterietrennschalter_toggle", "Electricity On/Off"),
                ("parking_brake_toggle", "Parking Brake On/Off"),
                ("bus_doorfront0", "Front Door, 1st Wing Open/Close"),
                ("automatic_R", "Automatic Gearbox: R"),
                ("door_haltewunsch", "Stop Request Button"),
                ("IBIS_loeschen", "IBIS: Delete"),
            ],
        )
    }

    #[test]
    fn official_texts_first() {
        let n = eng();
        assert_eq!(n.control("kw_batterietrennschalter"), "Electricity On/Off");
        assert_eq!(n.control("parking_brake_mouse"), "Parking Brake On/Off");
        assert_eq!(n.control("bus_doorfront0_external"), "Front Door, 1st Wing Open/Close (outside)");
        assert_eq!(n.control("automatic_R_mouse"), "Automatic Gearbox: R");
        assert_eq!(n.control("door_haltewunsch_2"), "Stop Request Button 2");
        assert_eq!(n.control("IBIS_Loeschen"), "IBIS: Delete");
    }

    #[test]
    fn german_names_translated() {
        let n = eng();
        assert_eq!(n.control("cp_Fahrertuer"), "Driver's door");
        assert_eq!(n.control("Fahrertuer_Links"), "Left driver's door");
        assert_eq!(n.control("Spiegel_Links"), "Left mirror");
        assert_eq!(n.control("cp_spiegelheizung_toggle"), "Mirror heating on/off");
        assert_eq!(n.control("cp_kneeling_toggle"), "Kneeling on/off");
        assert_eq!(n.control("cp_schalter_hub_dn_toggle"), "Lift down switch on/off");
        assert_eq!(n.control("taster_heiz_DEF"), "Heating DEF button");
        assert_eq!(n.control("cp_heizregler_fussraum"), "Heating control footwell");
        assert_eq!(n.control("cp_klappfenster_OL1"), "Tilting window upper left 1");
        assert_eq!(n.control("door_aussenoeffner"), "Door outside opener");
        assert_eq!(n.control("matrix_seitenschildklemme"), "Destination display side sign clamp");
        assert_eq!(n.control("cp_dachluke_1"), "Roof hatch 1");
        assert_eq!(n.control("cp_Fahrgastpendel"), "Passenger swing gate");
        assert_eq!(n.control("cp_schluessel_mov"), "Ignition key");
        assert_eq!(n.control("haltestellenbremse"), "Stop brake");
        assert_eq!(n.part("Zahltisch_Wechsler_0_05"), "Cash desk coin changer 0.05");
        assert_eq!(n.part("SD_Panel_Tachonadel"), "Panel speedometer needle");
        assert_eq!(n.part("GN92_Lenksaeule"), "Steering column");
        assert_eq!(n.part("N92_Blinker_Schalter"), "Indicator switch");
        assert_eq!(n.part("D_Panel_IBIS_0"), "Panel IBIS 0");
        assert_eq!(n.part("Kinderwagenknopf"), "Buggy button");
        assert_eq!(n.part("Haltewunschtaster"), "Stop request button");
        assert_eq!(n.part("Innenbeleuchtung"), "Interior light");
        assert_eq!(n.part("Türflügel_vorne"), "Front door leaf");
        assert_eq!(n.part("EN92_fenster_ext"), "Outer window");
        assert_eq!(n.control("kw_m_engine_startbutton"), "Engine start button");
        // the Citaro LE VER/BVG mod's switches (acceptance test: shown in German)
        assert_eq!(n.control("cp_tuersperre"), "Door lock");
        assert_eq!(n.control("cp_schulfahrschalter"), "Training run switch");
        assert_eq!(n.control("ALMEX_Funktion"), "ALMEX function");
        assert_eq!(n.control("comp_button_qiut"), "Computer button quit");
        assert_eq!(n.control("cp_frostregler_temp"), "Frost control temperature");
        assert_eq!(n.control("cp_schalter_knick_ovrd_toggle"), "Articulation override switch on/off");
    }

    /// Every word of every stock and add-on `[mouseevent]` name is translated (a German word
    /// that stays is shown verbatim on an English HUD).
    #[test]
    fn mod_switch_names_have_no_german_left() {
        let n = eng();
        let german = ["sperre", "schul", "funktion", "schalter", "regler", "taster", "tuer", "fenster", "licht", "heiz", "knopf", "hebel"];
        for name in [
            "cp_tuersperre", "cp_schulfahrschalter", "ALMEX_Funktion", "cp_doorEntriegelung_01", "cp_frostregler_temp", "cp_klappfenster_UR4",
            "taster_heiz_Misch", "cp_schalter_knick_ovrd_toggle", "cp_licht_unterdeck_toggle", "rlbnd_linie_select_100_+", "rollband_auf",
            "door_kinderwagenwunsch", "ramplift_wunsch", "cp_novitus_kasetka", "rbl_rueck", "klappsitzL", "Fahrertuer_Rechts",
        ] {
            let t = n.control(name).to_lowercase();
            assert!(!german.iter().any(|g| t.contains(g)), "{name} -> {t}");
        }
    }

    #[test]
    fn german_setting_keeps_german() {
        let n = ControlNames::from_table("deutsch", &[]);
        assert_eq!(n.lang, "DEU");
        assert_eq!(n.control("cp_spiegelheizung_toggle"), "Spiegelheizung");
    }
}
