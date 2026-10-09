//! The keyboard page's "Add binding" picker: every action a key can be given - the
//! installed key-language files' and OMSI's events, the triggers of every installed bus's
//! scripts (mods' too: found in the background, and kept in a cache for the next start) and
//! the ones the keyboard file has - searchable by name, by what it does and by the bus that
//! uses it.

use super::*;
use std::collections::HashMap;

/// An action the picker offers: its name, what it does, where it was found (the language
/// files, OMSI's events, a bus's script, the keyboard file) and the bus files that use it.
#[derive(Clone)]
pub(super) struct KeyActionOption {
    action: String,
    label: String,
    sources: Vec<String>,
    bus_paths: Vec<String>,
}

/// Every action the vehicles' section can be given: the language files' texts, OMSI's events
/// (in their own spelling), the buses' script triggers and what the keyboard file has.
fn action_options(names: &crate::describe::ControlNames, script_actions: &HashMap<String, Vec<String>>, bindings: &Value) -> Vec<KeyActionOption> {
    let mut actions: HashMap<String, KeyActionOption> = HashMap::new();
    let mut found = |action: &str, label: Option<&str>, source: Option<String>| {
        let option = actions.entry(action.to_ascii_lowercase()).or_insert_with(|| KeyActionOption {
            action: action.to_string(),
            label: label.map_or_else(|| action_text(names, action), str::to_string),
            sources: Vec::new(),
            bus_paths: Vec::new(),
        });
        if let Some(source) = source.filter(|s| !option.sources.contains(s)) {
            option.sources.push(source);
        }
    };
    for (action, label) in names.actions() {
        found(&action, Some(&label), Some("Installed key-language files".into()));
    }
    for (action, label) in names.events() {
        found(&action, Some(&label), Some("OMSI event".into()));
    }
    for (action, sources) in script_actions {
        found(action, None, None);
        for source in sources {
            found(action, None, Some(format!("Bus script: {source}")));
        }
    }
    for action in bindings.get("vehicles").and_then(Value::as_array).into_iter().flatten().filter_map(|b| b.get("action").and_then(Value::as_str)) {
        found(action, None, Some("Configured binding".into()));
    }
    // OMSI's event browser uses `events()`, which keeps the spelling of the language files'
    // spelling table: the picker shows that one
    let events: HashMap<String, (String, String)> = names.events().into_iter().map(|(a, l)| (a.to_ascii_lowercase(), (a, l))).collect();
    let mut options: Vec<_> = actions.into_iter().map(|(key, mut option)| {
        if let Some((action, label)) = events.get(&key) {
            option.action = action.clone();
            option.label = label.clone();
        }
        option.bus_paths = bus_source_paths(&option.sources);
        option
    }).collect();
    options.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()).then_with(|| a.action.to_lowercase().cmp(&b.action.to_lowercase())));
    options
}

/// The actions a controller's button can be given on the launcher's page, and their labels:
/// the keyboard file's vehicle actions, the H-pattern gates and the game's own (built once
/// and kept, not every frame).
pub(super) fn controller_action_choices(names: &crate::describe::ControlNames, bindings: &Value) -> (Vec<String>, Vec<String>) {
    let mut actions: Vec<String> = vec!["<none>".into()];
    actions.extend(bindings.get("vehicles").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|b| b.get("action").and_then(|x| x.as_str()).map(String::from)).collect::<Vec<_>>()).unwrap_or_default());
    // H-pattern shifters use OMSI's "_fest" actions: pressing the gate selects the gear,
    // releasing it fires "_fest_off", which lets the bus script return to neutral.
    for a in ["kw_s_R_fest", "kw_s_1_fest", "kw_s_2_fest", "kw_s_3_fest", "kw_s_4_fest", "kw_s_5_fest", "kw_s_6_fest", "kw_s_7_fest", "kw_s_8_fest", "kw_s_9_fest", "kw_s_10_fest"] {
        if !actions.iter().any(|x| x.eq_ignore_ascii_case(a)) {
            actions.push(a.to_string());
        }
    }
    // the game's own view actions (looking around while held, the cameras, the views)
    for a in PAD_GAME_ACTIONS {
        if !actions.iter().any(|x| x == a) {
            actions.insert(1, a.to_string());
        }
    }
    actions.dedup();
    let labels: Vec<String> = actions.iter().enumerate().map(|(i, a)| if i == 0 { a.clone() } else { action_text(names, a) }).collect();
    (actions, labels)
}

/// The actions whose name or label has `query` and that a bus matching `source_query` uses.
fn filter_action_options(options: &[KeyActionOption], query: &str, source_query: &str) -> Vec<KeyActionOption> {
    let query = query.trim().to_lowercase();
    let source_query = normalize_source_query(source_query);
    options.iter().filter(|option| {
        (query.is_empty()
            || option.action.to_lowercase().contains(&query)
            || option.label.to_lowercase().contains(&query))
            && (source_query.is_empty()
                || option.sources.iter().any(|source| {
                    let path = source.strip_prefix("Bus script: ").unwrap_or(source);
                    let path = path.split_once(" (").map(|(path, _)| path).unwrap_or(path);
                    normalize_source_query(path).contains(&source_query)
                }))
    }).cloned().collect()
}

/// Lower case, every run of other characters one space: "MAN A26" finds `MAN_A26_3D.bus`.
fn normalize_source_query(text: &str) -> String {
    let mut normalized = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            normalized.extend(c.to_lowercase());
        } else if !normalized.ends_with(' ') {
            normalized.push(' ');
        }
    }
    normalized.trim().to_string()
}

/// The bus folders and files that match, those that start so first.
fn source_suggestions(paths: &[String], query: &str) -> Vec<String> {
    let query = normalize_source_query(query);
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<(bool, String, String)> = paths.iter().filter_map(|path| {
        let normalized = normalize_source_query(path);
        normalized.contains(&query).then(|| (!normalized.starts_with(&query), normalized, path.clone()))
    }).collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    matches.into_iter().map(|(_, _, path)| path).collect()
}

/// The rows of a list scrolled `offset` down that a view `viewport_height` high shows.
fn visible_row_range(offset: f32, viewport_height: f32, count: usize, row_height: f32) -> std::ops::Range<usize> {
    let offset = offset.max(0.0);
    let first = (offset / row_height).floor() as usize;
    let end = ((offset + viewport_height.max(0.0)) / row_height).ceil() as usize;
    first.min(count)..end.min(count).max(first.min(count))
}

/// The bus file of a source (`Bus script: Pack/Bus.bus (Maker Type)`).
fn source_path(source: &str) -> String {
    let source = source.strip_prefix("Bus script: ").unwrap_or(source);
    let source = source.split_once(": ").map(|(_, path)| path).unwrap_or(source);
    source.split_once(" (").map(|(path, _)| path).unwrap_or(source).to_string()
}

/// The bus files among an action's sources, each once.
fn bus_source_paths(sources: &[String]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for source in sources.iter().filter_map(|s| s.strip_prefix("Bus script: ")) {
        let path = source_path(source);
        if seen.insert(path.to_ascii_lowercase()) {
            paths.push(path);
        }
    }
    paths
}

fn bus_usage_label(bus_count: usize, total_bus_count: usize) -> String {
    format!("Used by {bus_count}/{total_bus_count} buses")
}

/// The last scan of the buses' scripts, kept on this computer (the data folder's
/// `cache/key-actions.json`) so the picker is full at once on the next start: the actions
/// found, the bus files that use them and how many were read, for one OMSI folder. Nothing
/// leaves the computer; a new scan replaces it every time the picker is first opened.
#[derive(serde::Deserialize, serde::Serialize)]
struct ScriptActionCache {
    version: u32,
    root: String,
    actions: HashMap<String, Vec<String>>,
    source_paths: Vec<String>,
    total_buses: usize,
}

fn script_action_cache_path() -> std::path::PathBuf {
    core::data_dir().join("cache").join("key-actions.json")
}

fn load_script_action_cache(path: &std::path::Path, root: &str) -> Result<Option<ScriptActionCache>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let cache: ScriptActionCache = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    Ok((cache.version == 1 && cache.root == root).then_some(cache))
}

fn save_script_action_cache(path: &std::path::Path, cache: &ScriptActionCache) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "cache path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(cache).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

/// The picker's actions: the cache read once, the scan started once per session (in the
/// background) and what it sent taken in, the list rebuilt when it changed.
fn ensure_key_action_catalog(l: &mut Launcher) {
    let root = std::path::PathBuf::from(&l.state.config.root);
    if l.pages.kb_script_actions_root.as_ref() != Some(&root) {
        l.pages.kb_script_actions = None;
        l.pages.kb_script_actions_rx = None;
        l.pages.kb_script_cache_loaded = false;
        l.pages.kb_script_scan_actions = None;
        l.pages.kb_script_scan_paths.clear();
        l.pages.kb_script_actions_root = Some(root);
        l.pages.kb_source_paths.clear();
        l.pages.kb_source_path_set.clear();
        l.pages.kb_source_suggestions = None;
        l.pages.kb_script_scan = (0, 0, String::new());
        l.pages.kb_script_total_buses = 0;
        l.pages.kb_script_scan_complete = false;
        l.pages.kb_action_options = None;
        l.pages.kb_filtered_options = None;
        l.pages.controller_action_choices = None;
    }
    if !l.pages.kb_script_cache_loaded {
        l.pages.kb_script_cache_loaded = true;
        match load_script_action_cache(&script_action_cache_path(), &l.state.config.root) {
            Ok(Some(cache)) => {
                l.pages.kb_script_actions = Some(cache.actions);
                l.pages.kb_source_paths = cache.source_paths;
                l.pages.kb_source_path_set = l.pages.kb_source_paths.iter().map(|path| path.to_lowercase()).collect();
                l.pages.kb_script_scan = (cache.total_buses, cache.total_buses, String::new());
                l.pages.kb_script_total_buses = cache.total_buses;
                l.pages.kb_source_suggestions = None;
                l.pages.kb_action_options = None;
                l.pages.kb_filtered_options = None;
                log::info!("key action catalog: loaded {} actions from cache", l.pages.kb_script_actions.as_ref().map_or(0, |actions| actions.len()));
            }
            Ok(None) => {}
            Err(error) => log::warn!("key action catalog cache could not be loaded: {error}"),
        }
    }
    if l.pages.kb_script_actions_rx.is_none() && !l.pages.kb_script_scan_complete {
        let _ = core::content_dir();
        // (unbounded: the scan runs to its end, and is cached, also while the picker is shut
        // and nobody reads it)
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            crate::describe::ControlNames::scan_script_actions(|update| { let _ = tx.send(update); });
        });
        l.pages.kb_script_scan_actions = Some(HashMap::new());
        l.pages.kb_script_scan_paths.clear();
        l.pages.kb_source_path_set.clear();
        l.pages.kb_script_scan = (0, 0, String::new());
        l.pages.kb_script_actions_rx = Some(rx);
    }
    let scan_was_complete = l.pages.kb_script_scan_complete;
    if let Some(rx) = l.pages.kb_script_actions_rx.as_ref() {
        loop {
            let update = match rx.try_recv() {
                Ok(update) => update,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::warn!("key action scan stopped before it completed");
                    l.pages.kb_script_scan_complete = true;
                    break;
                }
            };
            l.pages.kb_script_scan = (update.done, update.total, update.current);
            l.pages.kb_script_total_buses = update.total;
            for (_, source) in &update.discovered {
                let path = source_path(source);
                if l.pages.kb_source_path_set.insert(path.to_lowercase()) {
                    l.pages.kb_script_scan_paths.push(path);
                }
            }
            if let Some(actions) = l.pages.kb_script_scan_actions.as_mut() {
                for (action, source) in update.discovered {
                    let sources = actions.entry(action.to_ascii_lowercase()).or_default();
                    if !sources.iter().any(|existing| existing.eq_ignore_ascii_case(&source)) {
                        sources.push(source);
                    }
                }
            }
            if update.complete {
                l.pages.kb_script_scan_complete = true;
                let actions = l.pages.kb_script_scan_actions.take().unwrap_or_default();
                let source_paths = std::mem::take(&mut l.pages.kb_script_scan_paths);
                l.pages.kb_source_path_set = source_paths.iter().map(|path| path.to_lowercase()).collect();
                l.pages.kb_source_paths = source_paths;
                l.pages.kb_source_paths.sort_by_key(|path| normalize_source_query(path));
                let cache = ScriptActionCache {
                    version: 1,
                    root: l.state.config.root.clone(),
                    actions: actions.clone(),
                    source_paths: l.pages.kb_source_paths.clone(),
                    total_buses: update.total,
                };
                if let Err(error) = save_script_action_cache(&script_action_cache_path(), &cache) {
                    log::warn!("key action catalog cache could not be saved: {error}");
                }
                l.pages.kb_script_actions = Some(actions);
                l.pages.kb_source_suggestions = None;
                // (the scan's thread is done: its channel shut is no failure)
                break;
            }
        }
    }
    if l.pages.kb_script_scan_complete {
        l.pages.kb_script_actions_rx = None;
    }
    if l.pages.kb_script_scan_complete && !scan_was_complete {
        l.pages.kb_action_options = None;
        l.pages.kb_filtered_options = None;
        l.pages.controller_action_choices = None;
    }
    if l.pages.kb_action_options.is_none() {
        let empty = HashMap::new();
        let scripts = l.pages.kb_script_actions.as_ref().unwrap_or(&empty);
        l.pages.kb_action_options = Some(action_options(control_names(l), scripts, &l.state.keybindings));
    }
}


/// The searchable catalog of actions for a new keyboard binding.
pub fn keybind_picker(l: &mut Launcher) {
    let Some(section) = l.pages.kb_picker else {
        return;
    };
    if let Some((action, paths)) = l.pages.kb_source_action.clone() {
        keybind_sources_dialog(l, &action, &paths);
        return;
    }
    ensure_key_action_catalog(l);
    let size = l.ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    l.ui.solid(full);
    l.ui.p().rect(full, Color::rgba(0, 0, 0, 0.68));
    let (w, h) = ((size.x - 32.0).min(720.0), (size.y - 32.0).min(760.0));
    let panel = Rect::new((size.x - w) * 0.5, (size.y - h) * 0.5, w, h);
    l.ui.panel(panel);
    let inner = panel.pad(20.0, 18.0);
    l.ui.text_in("Add a key binding", Rect::new(inner.x, inner.y, inner.w, 28.0), 19.0, Weight::Bold, TEXT, Align::Left);
    let section_name = if section == 0 { "Driving & the bus" } else { "The game" };
    l.ui.text_in(section_name, Rect::new(inner.x, inner.y + 29.0, inner.w, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);

    // what to look for: the action (its name or what it does), and the bus that uses it
    let filter_w = (inner.w - 12.0) * 0.5;
    let mut query = std::mem::take(&mut l.pages.kb_picker_filter);
    let query_changed = l.ui.text_input("kb-picker-search", Rect::new(inner.x, inner.y + 54.0, filter_w, 36.0), &mut query, "Search actions…", Some("search"));
    l.pages.kb_picker_filter = query.clone();
    let mut source_query = std::mem::take(&mut l.pages.kb_picker_source_filter);
    let source_r = Rect::new(inner.x + filter_w + 12.0, inner.y + 54.0, filter_w, 36.0);
    let mut source_changed = l.ui.text_input("kb-picker-source-search", source_r, &mut source_query, "Filter by bus folder or bus file (.bus)", Some("folder_open"));
    let (suggestions_h, picked_source) = source_suggestion_list(l, &source_query, Rect::new(source_r.x, inner.y + 94.0, filter_w, 0.0));
    if let Some(path) = picked_source {
        source_query = path;
        source_changed = true;
    }
    l.pages.kb_picker_source_filter = source_query.clone();
    let filter_changed = l.pages.kb_filtered_options.as_ref().is_none_or(|(a, s, _)| a != &query || s != &source_query);
    if query_changed || source_changed || filter_changed {
        let all = l.pages.kb_action_options.as_deref().unwrap_or_default();
        l.pages.kb_filtered_options = Some((query.clone(), source_query.clone(), filter_action_options(all, &query, &source_query)));
        if query_changed || source_changed {
            let id = id_of("kb-action-picker");
            l.ui.scroll.insert(id, 0.0);
            l.ui.scroll.insert(id ^ 0xabc, 0.0);
        }
    }
    let count = l.pages.kb_filtered_options.as_ref().map_or(0, |(_, _, o)| o.len());
    scan_progress(l, count, Rect::new(inner.x, inner.y + 96.0 + suggestions_h, inner.w, 25.0));

    let list = Rect::new(inner.x - 6.0, inner.y + 130.0 + suggestions_h, inner.w + 12.0, (inner.h - 184.0 - suggestions_h).max(80.0));
    let (picked, show_sources) = action_list(l, list);
    if let Some(request) = show_sources {
        l.pages.kb_source_action = Some(request);
    }
    if l.ui.button("kb-picker-cancel", Rect::new(inner.right() - 110.0, inner.bottom() - 38.0, 110.0, 36.0), "Cancel", None, ButtonKind::Ghost) {
        l.pages.kb_picker = None;
        l.pages.kb_picker_filter.clear();
        l.pages.kb_picker_source_filter.clear();
        return;
    }
    // the action picked: a new entry, waiting for its key on the keyboard page (which shows
    // it alone, the filter set to it, as the direct way of #854 does)
    if let Some(action) = picked {
        let key = if section == 0 { "vehicles" } else { "game" };
        if let Some(bindings) = l.state.keybindings.get_mut(key).and_then(Value::as_array_mut) {
            bindings.push(json!({ "action": action, "scan_code": 0, "modifier": 0 }));
            l.pages.capturing = Some((section, bindings.len() - 1));
        }
        l.pages.kb_filter[section] = action.clone();
        l.state.set_status("Binding added. Press the key you want to use (Escape cancels).", false);
        l.pages.kb_action_options = None;
        l.pages.kb_filtered_options = None;
        l.pages.controller_action_choices = None;
        l.pages.kb_picker = None;
        l.pages.kb_picker_filter.clear();
        l.pages.kb_picker_source_filter.clear();
    }
}

/// The bus folders and files that match what is typed in the bus filter, under it: their
/// height, and the one clicked.
fn source_suggestion_list(l: &mut Launcher, source_query: &str, at: Rect) -> (f32, Option<String>) {
    if l.pages.kb_source_suggestions.as_ref().is_none_or(|(cached, _)| cached != source_query) {
        l.pages.kb_source_suggestions = Some((source_query.to_string(), source_suggestions(&l.pages.kb_source_paths, source_query)));
    }
    let suggestions = l.pages.kb_source_suggestions.as_ref().map(|(_, paths)| paths.as_slice()).unwrap_or(&[]);
    let typed = normalize_source_query(source_query);
    let exact = suggestions.iter().any(|path| normalize_source_query(path) == typed);
    if typed.is_empty() || exact || suggestions.is_empty() {
        return (0.0, None);
    }
    let row_h = 28.0;
    let r = Rect::new(at.x, at.y, at.w, (suggestions.len() as f32 * row_h).min(140.0));
    let mut picked = None;
    l.ui.scroll_area("kb-source-suggestions", r, &mut |ui, view| {
        for i in visible_row_range(r.y - view.y, r.h, suggestions.len(), row_h) {
            let row = Rect::new(view.x + 2.0, view.y + i as f32 * row_h, view.w - 8.0, row_h - 2.0);
            if ui.row(&format!("kb-source-suggestion-{i}"), row, false) {
                picked = Some(suggestions[i].clone());
            }
            ui.text_in(&suggestions[i], row.pad(8.0, 0.0), 11.0, Weight::Regular, TEXT_SOFT, Align::Left);
        }
        suggestions.len() as f32 * row_h
    });
    (r.h, picked)
}

/// How many actions the search finds, and how far the scan of the buses' scripts is.
fn scan_progress(l: &mut Launcher, results: usize, r: Rect) {
    let (done, total, current) = &l.pages.kb_script_scan;
    let status = if l.pages.kb_script_scan_complete {
        format!("{results} results · scanned {total} vehicle files")
    } else if *total > 0 {
        format!("{results} results · scanning {done} / {total} · {current}")
    } else if l.pages.kb_script_actions.is_some() {
        format!("{results} results · checking installed bus scripts…")
    } else {
        format!("{results} results · finding installed bus scripts…")
    };
    let progress = if *total == 0 { 0.0 } else { *done as f32 / *total as f32 };
    l.ui.text_in(&status, Rect::new(r.x, r.y, r.w, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
    let bar = Rect::new(r.x, r.y + 20.0, r.w, 5.0);
    l.ui.p().rounded(bar, 3.0, Color::WHITE.alpha(0.07));
    if progress > 0.0 {
        l.ui.p().rounded(Rect::new(bar.x, bar.y, bar.w * progress, bar.h), 3.0, ACCENT);
    }
}

/// The actions found, only the rows in view drawn (a big installation has thousands): the
/// one clicked, and the one whose buses were asked for.
fn action_list(l: &mut Launcher, list: Rect) -> (Option<String>, Option<(String, Vec<String>)>) {
    let options = l.pages.kb_filtered_options.as_ref().map(|(_, _, o)| o.as_slice()).unwrap_or(&[]);
    let total_buses = l.pages.kb_script_total_buses;
    let mut picked: Option<String> = None;
    let mut show_sources: Option<(String, Vec<String>)> = None;
    l.ui.scroll_area("kb-action-picker", list, &mut |ui, view| {
        if options.is_empty() {
            ui.text_in("No matching actions.", Rect::new(view.x + 8.0, view.y + 8.0, view.w - 20.0, 24.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
            return 40.0;
        }
        let row_h = 48.0;
        for i in visible_row_range(list.y - view.y, list.h, options.len(), row_h) {
            let option = &options[i];
            let row = Rect::new(view.x + 6.0, view.y + i as f32 * row_h, view.w - 18.0, row_h - 3.0);
            // (the buses that use it: a button that lists them)
            let used_by = bus_usage_label(option.bus_paths.len(), total_buses);
            let used_w = if option.bus_paths.is_empty() { 0.0 } else { ui.width(&used_by, 13.0, Weight::Medium) + 13.0 * 1.3 + 24.0 };
            let action_row = Rect::new(row.x, row.y, row.w - used_w, row.h);
            if ui.row(&format!("kb-picker-action-{}", option.action), action_row, false) {
                picked = Some(option.action.clone());
            }
            if used_w > 0.0
                && ui.button(&format!("kb-picker-sources-{}", option.action), Rect::new(row.right() - used_w, row.y + 3.0, used_w - 4.0, row.h - 6.0), &used_by, Some("list"), ButtonKind::Ghost)
            {
                show_sources = Some((option.action.clone(), option.bus_paths.clone()));
            }
            ui.text_in(&option.label, Rect::new(row.x + 12.0, row.y + 1.0, action_row.w - 24.0, 20.0), 13.0, Weight::Medium, TEXT, Align::Left);
            ui.text_in(&option.action, Rect::new(row.x + 12.0, row.y + 21.0, action_row.w - 24.0, 16.0), 10.5, Weight::Regular, TEXT_FAINT, Align::Left);
        }
        options.len() as f32 * row_h
    });
    (picked, show_sources)
}

fn keybind_sources_dialog(l: &mut Launcher, action: &str, paths: &[String]) {
    let size = l.ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    l.ui.solid(full);
    l.ui.p().rect(full, Color::rgba(0, 0, 0, 0.72));
    let w = (size.x - 32.0).min(640.0);
    let h = (size.y - 32.0).min(620.0);
    let panel = Rect::new((size.x - w) * 0.5, (size.y - h) * 0.5, w, h);
    l.ui.panel(panel);
    let inner = panel.pad(20.0, 18.0);
    l.ui.text_in("Bus files using this action", Rect::new(inner.x, inner.y, inner.w, 26.0), 17.0, Weight::Bold, TEXT, Align::Left);
    l.ui.text_in(action, Rect::new(inner.x, inner.y + 28.0, inner.w, 20.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
    let list = Rect::new(inner.x - 6.0, inner.y + 58.0, inner.w + 12.0, (inner.h - 108.0).max(72.0));
    l.ui.scroll_area("kb-source-list", list, &mut |ui, view| {
        let row_h = 30.0;
        for (i, path) in paths.iter().enumerate() {
            let row = Rect::new(view.x + 6.0, view.y + i as f32 * row_h, view.w - 18.0, row_h - 2.0);
            if row.bottom() < list.y || row.y > list.bottom() {
                continue;
            }
            ui.p().rounded(row, 4.0, Color::WHITE.alpha(0.035));
            ui.text_in(path, row.pad(10.0, 0.0), 11.5, Weight::Regular, TEXT_SOFT, Align::Left);
        }
        paths.len() as f32 * row_h
    });
    if l.ui.button("kb-source-close", Rect::new(inner.right() - 110.0, inner.bottom() - 38.0, 110.0, 36.0), "Back", None, ButtonKind::Normal) {
        l.pages.kb_source_action = None;
    }
}


#[cfg(test)]
mod keybind_picker_tests {
    use super::*;

    #[test]
    fn empty_search_shows_the_catalog_and_configured_custom_actions() {
        let names = crate::describe::ControlNames::from_table("ENG", &[
            ("door", "Front door"),
            ("horn", "Horn"),
            ("ivu_ticket_cancel", "IVU: Cancel ticket"),
        ]);
        let bindings = json!({
            "vehicles": [{ "action": "door" }, { "action": "mod_custom_action" }],
            "game": [{ "action": "sim_pause" }],
        });
        let script_sources = HashMap::from([
            ("mod_custom_action".into(), vec![
                "VehiclePack/Vehicle.bus".into(),
                "IVUPack/IVU.bus".into(),
                "AnotherPack/Another.bus".into(),
                "ThirdPack/Third.bus".into(),
                "FourthPack/Fourth.bus".into(),
            ]),
        ]);

        let all = action_options(&names, &script_sources, &bindings);
        let actions: Vec<&str> = all.iter().map(|option| option.action.as_str()).collect();
        assert_eq!(actions.len(), 4);
        assert!(actions.contains(&"door"));
        assert!(actions.contains(&"horn"));
        assert!(actions.contains(&"mod_custom_action"));
        let mod_action = all.iter().find(|option| option.action == "mod_custom_action").unwrap();
        assert!(mod_action.sources.contains(&"Configured binding".into()));
        assert_eq!(mod_action.bus_paths.len(), 5);

        let matches = filter_action_options(&all, "front door", "");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].action, "door");

        let matches = filter_action_options(&all, "vehiclepack", "");
        assert!(matches.is_empty());

        let matches = filter_action_options(&all, "ivu", "");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].action, "ivu_ticket_cancel");

        let matches = filter_action_options(&all, "", "ivupack");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].action, "mod_custom_action");
    }

    #[test]
    fn empty_query_keeps_catalog_entries_past_twelve() {
        let names = crate::describe::ControlNames::from_table("ENG", &[]);
        let script_actions: HashMap<String, Vec<String>> = (0..40)
            .map(|i| (format!("vehicle_action_{i:02}"), Vec::new()))
            .collect();
        let options = action_options(&names, &script_actions, &json!({}));
        assert_eq!(filter_action_options(&options, "", "").len(), 40);
    }

    #[test]
    fn bus_usage_label_shows_the_action_count_out_of_the_scan_total() {
        assert_eq!(bus_usage_label(5, 25), "Used by 5/25 buses");
        assert_eq!(bus_usage_label(1, 25), "Used by 1/25 buses");
    }

    #[test]
    fn script_action_cache_round_trips_and_is_scoped_to_the_install_root() {
        let dir = std::env::temp_dir().join(format!(
            "openomsi_action_cache_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        let path = dir.join("key-actions.json");
        let cache = ScriptActionCache {
            version: 1,
            root: "C:\\OMSI 2".into(),
            actions: HashMap::from([(
                "cruise_control".into(),
                vec!["BusPack/Bus.bus (Example Bus)".into()],
            )]),
            source_paths: vec!["BusPack/Bus.bus".into()],
            total_buses: 12,
        };

        save_script_action_cache(&path, &cache).unwrap();
        let loaded = load_script_action_cache(&path, "C:\\OMSI 2").unwrap().unwrap();
        assert_eq!(loaded.actions, cache.actions);
        assert_eq!(loaded.source_paths, cache.source_paths);
        assert_eq!(loaded.total_buses, cache.total_buses);
        assert!(load_script_action_cache(&path, "D:\\OMSI 2").unwrap().is_none());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn source_search_matches_folder_prefix_and_spaced_bus_filename() {
        let names = crate::describe::ControlNames::from_table("ENG", &[]);
        let script_actions = HashMap::from([
            ("neoman_special_control".into(), vec!["NEOMAN_Overhaul_v3/Vehicle.bus".into()]),
            ("man_a26_cruise".into(), vec!["MAN_A26_3D.bus".into()]),
        ]);
        let options = action_options(&names, &script_actions, &json!({}));

        let neo = filter_action_options(&options, "", "NEO");
        assert_eq!(neo.len(), 1);
        assert_eq!(neo[0].action, "neoman_special_control");

        let a26 = filter_action_options(&options, "", "MAN A26");
        assert_eq!(a26.len(), 1);
        assert_eq!(a26[0].action, "man_a26_cruise");
    }

    #[test]
    fn source_suggestions_match_folder_and_filename_fragments() {
        let paths = [
            "NEOMAN_Overhaul_v3/Vehicle.bus".to_string(),
            "MAN_A26_3D.bus".to_string(),
            "OtherPack/Other.bus".to_string(),
        ];
        assert_eq!(source_suggestions(&paths, "NEO"), ["NEOMAN_Overhaul_v3/Vehicle.bus"]);
        assert_eq!(source_suggestions(&paths, "MAN A26"), ["MAN_A26_3D.bus"]);
    }

    /// A controller's button can be given the keyboard file's vehicle actions (a bus's own
    /// trigger the picker added among them), the H-pattern gates and the game's own actions -
    /// not the game section's keys, which a button would send to the bus.
    #[test]
    fn controller_choices_include_keyboard_catalog_actions() {
        let names = crate::describe::ControlNames::from_table("ENG", &[("door", "Front door")]);
        let bindings = json!({
            "vehicles": [{ "action": "custom_cruise_control" }, { "action": "door" }],
            "game": [{ "action": "chat_open" }],
        });
        let (actions, labels) = controller_action_choices(&names, &bindings);
        assert_eq!(actions.first().map(String::as_str), Some("<none>"));
        let custom = actions.iter().position(|action| action == "custom_cruise_control").unwrap();
        assert!(labels[custom].contains("Custom cruise control"));
        let door = actions.iter().position(|action| action == "door").unwrap();
        assert_eq!(labels[door], action_text(&names, "door"));
        assert!(actions.contains(&"kw_s_1_fest".to_string()));
        assert!(actions.contains(&"doors_all".to_string()));
        assert!(!actions.contains(&"chat_open".to_string()));
        // <none>, the two the file has, the eleven H-pattern gates and the game's own
        assert_eq!(actions.len(), 1 + 2 + 11 + PAD_GAME_ACTIONS.len());
    }

    #[test]
    fn visible_rows_advance_with_scroll_for_large_catalogs() {
        assert_eq!(visible_row_range(0.0, 540.0, 3434, 48.0), 0..12);
        assert_eq!(visible_row_range(20.0 * 48.0, 540.0, 3434, 48.0), 20..32);
        assert_eq!(visible_row_range(20.0 * 48.0, 540.0, 30, 48.0), 20..30);
    }
}
