//! The interface in every language: a test that each text the launcher and the game's
//! overlays show has its translation in the six languages the interface is kept in.
//!
//! It reads the interface's source (the launcher and its pages, the bus company, the line
//! editor, the livery studio, the phone companion, the navigator and the overlays over the
//! drive), finds every string literal handed to `tr` or to a control that translates its own
//! label (`Ui::button`, `label`, `toggle`, `paragraph`, the settings rows, the game menu's
//! rows ...) and looks it up in the tables the game is built with (`locales/*.yml`, through
//! `_rust_i18n_try_translate`, as `omsi_ui::tr` does). Test code is left out. A text built
//! with `format!` or chosen in an `if` is not seen: those are written as a whole text with
//! `%{name}` placeholders (see `pages::sessions`) and checked where they are a literal.

use std::path::{Path, PathBuf};

/// The languages every text of the interface has (the launcher's language menu).
const LANGUAGES: [&str; 6] = ["nl", "de", "fr", "ru", "uk", "pl"];

/// What is checked, under `src`: directories (everything in them) and files.
const SCOPE: &[&str] = &[
    "launcher",
    "companion",
    "navigator.rs",
    "nav_duty.rs",
    "nav_panel.rs",
    "nav_pins.rs",
    "nav_signon.rs",
    "vr_navigator.rs",
    "trip_report.rs",
    "drive_watch.rs",
    "game_lists.rs",
    "ui.rs",
    "touch.rs",
];

/// The calls whose string literal arguments are shown, translated: (the file they are
/// checked in, "" for every file; the function or method; the arguments shown). A file's own
/// line for a name stands instead of the lines for every file (a helper of the same name).
const SHOWN: &[(&str, &str, &[usize])] = &[
    ("", "tr", &[0]),
    ("", "tr_with", &[0]),
    ("", "count_text", &[1, 2]),
    // `launcher::ui::Ui` (and the painters: their text comes third)
    ("launcher/", "button", &[2]),
    ("", "label", &[1]),
    ("", "toggle", &[3]),
    ("", "heading", &[1]),
    ("", "paragraph", &[0]),
    ("", "paragraph_height", &[0]),
    ("", "tooltip", &[1]),
    ("", "icon_button", &[4]),
    ("", "slider", &[6]),
    ("", "slider_on_release", &[6]),
    ("", "text_input", &[3]),
    ("", "segmented", &[3]),
    ("", "segmented_some", &[3]),
    ("", "chips", &[3]),
    ("", "badge", &[1]),
    ("", "set_status", &[0]),
    ("", "text_in", &[0, 2]),
    ("", "text", &[0, 2]),
    // the launcher's settings rows and sections
    ("launcher/pages.rs", "toggle_setting", &[4]),
    ("launcher/pages.rs", "sel_setting", &[5]),
    ("launcher/pages.rs", "section", &[1, 2]),
    // the game menu's rows (`game_lists`)
    ("game_lists.rs", "row", &[0, 3]),
    ("game_lists.rs", "opens", &[0, 1]),
    ("game_lists.rs", "button", &[0, 1, 2]),
    ("game_lists.rs", "switch_row", &[2, 3]),
    ("game_lists.rs", "slider_row", &[2, 3]),
    ("game_lists.rs", "select_row", &[2, 3]),
    ("game_lists.rs", "preset_row", &[1, 2]),
    ("game_lists.rs", "pick", &[1, 2]),
    ("game_lists.rs", "head", &[0]),
    ("game_lists.rs", "title", &[0]),
    // the shift sheet's and the map list's own
    ("launcher/shiftsheet.rs", "count", &[1, 2]),
    ("launcher/mapchoice.rs", "t", &[0]),
    // the dealer's chips: (label, id, ...), not `Ui::chips`
    ("launcher/company/dealer.rs", "chips", &[2]),
];

/// Texts shown as they are in every language, and why.
const AS_IS: &[(&str, &str)] = &[
    ("Aa", "a sample of the chosen font"),
    ("OMSI", "the game's name"),
    ("OMSI 2", "the game's name"),
    ("openOMSI", "the game's name"),
    ("MOD", "the same word in every language"),
    ("Esc", "a key's name"),
    ("Discord Rich Presence", "a product's name"),
    ("github.com/Luc-nbr/openOMSI", "an address"),
    ("Stadtbus Grundorf", "an example company name"),
    ("Shuttleverkehr Altenfeld - Wurzbach", "an example line name"),
    ("MM-DD", "a date's form"),
    ("/path/to/OMSI 2", "an example folder"),
    ("C:\\Program Files (x86)\\Steam\\steamapps\\common\\OMSI 2", "an example folder"),
    ("12 Hauptbahnhof", "a destination display's sample"),
    ("Bus 42", "an example of a livery's lettering"),
    ("REAR", "an example of a livery's lettering"),
    ("ICAO", "the airports' code"),
    ("IBIS", "the ticket machine's name"),
    ("openomsi", "the program's file name"),
    ("min", "a unit"),
    ("h:mm", "a time's form"),
];

/// A string literal of the source: its text, its line, and the call it is an argument of.
struct Literal {
    text: String,
    line: usize,
    call: Option<(String, usize)>,
}

/// An open bracket while reading: a call's (or macro's) arguments, an array, or a block.
enum Frame {
    Call { name: String, arg: usize, transparent: bool },
    Array,
    Block,
}

/// Every string literal of `src` outside test code, with the call it is handed to: arrays
/// (`&["A", "B"]`) and `vec!` between them and the call are looked through, a block (an
/// `if`, a `match`, a closure's body) is not.
fn literals(src: &str) -> Vec<Literal> {
    let b: Vec<char> = src.chars().collect();
    let n = b.len();
    let mut out = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut line = 1;
    let mut i = 0;
    let mut ident = String::new();
    let mut bang = false;
    // the stack's depth where `#[cfg(test)]` code began, and an attribute waiting for its item
    let mut test_at: Option<usize> = None;
    let mut test_next = false;
    let starts = |i: usize, s: &str| s.chars().enumerate().all(|(k, c)| b.get(i + k) == Some(&c));
    while i < n {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if starts(i, "//") {
            while i < n && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if starts(i, "/*") {
            let mut depth = 0;
            while i < n {
                if starts(i, "/*") {
                    depth += 1;
                    i += 2;
                } else if starts(i, "*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    line += (b[i] == '\n') as usize;
                    i += 1;
                }
            }
            continue;
        }
        if starts(i, "#[cfg(test)]") || starts(i, "#[cfg(all(test") {
            test_next = true;
            while i < n && b[i] != ']' {
                i += 1;
            }
            i += 1;
            continue;
        }
        let word_before = i > 0 && (b[i - 1].is_alphanumeric() || b[i - 1] == '_');
        // a raw string: r"..", r#".."#
        if (c == 'r' || (c == 'b' && b.get(i + 1) == Some(&'r'))) && !word_before {
            let mut j = i + if c == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while b.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) == Some(&'"') {
                let start = j + 1;
                let mut k = start;
                while k < n && !(b[k] == '"' && (0..hashes).all(|h| b.get(k + 1 + h) == Some(&'#'))) {
                    k += 1;
                }
                let text: String = b[start..k.min(n)].iter().collect();
                let at = line;
                line += text.matches('\n').count();
                push(&mut out, text, at, &stack, test_at.is_some() || test_next);
                i = k + 1 + hashes;
                ident.clear();
                continue;
            }
        }
        if c == '"' || (c == 'b' && b.get(i + 1) == Some(&'"') && !word_before) {
            let mut j = i + if c == 'b' { 2 } else { 1 };
            let at = line;
            let mut text = String::new();
            while j < n && b[j] != '"' {
                if b[j] == '\\' {
                    let e = b.get(j + 1).copied().unwrap_or(' ');
                    match e {
                        '\n' => {
                            // a continued line: its leading blanks are not part of the text
                            line += 1;
                            j += 2;
                            while j < n && b[j].is_whitespace() {
                                line += (b[j] == '\n') as usize;
                                j += 1;
                            }
                            continue;
                        }
                        'n' => text.push('\n'),
                        't' => text.push('\t'),
                        'r' => text.push('\r'),
                        '0' => text.push('\0'),
                        'u' => {
                            let end = (j..n).find(|&k| b[k] == '}').unwrap_or(j);
                            let hex: String = b[j + 3..end].iter().collect();
                            text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                            j = end + 1;
                            continue;
                        }
                        'x' => {
                            let hex: String = b[j + 2..(j + 4).min(n)].iter().collect();
                            text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                            j += 4;
                            continue;
                        }
                        other => text.push(other),
                    }
                    j += 2;
                    continue;
                }
                line += (b[j] == '\n') as usize;
                text.push(b[j]);
                j += 1;
            }
            push(&mut out, text, at, &stack, test_at.is_some() || test_next);
            i = j + 1;
            ident.clear();
            continue;
        }
        if c == '\'' {
            // a character ('a', '\n', '\u{1f}', '"') or a lifetime ('a)
            let close = if b.get(i + 1) == Some(&'\\') { (i + 3..(i + 12).min(n)).find(|&k| b[k] == '\'') } else if b.get(i + 2) == Some(&'\'') { Some(i + 2) } else { None };
            match close {
                Some(k) => i = k + 1,
                None => {
                    i += 1;
                    while i < n && (b[i].is_alphanumeric() || b[i] == '_') {
                        i += 1;
                    }
                }
            }
            ident.clear();
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            ident.clear();
            while i < n && (b[i].is_alphanumeric() || b[i] == '_') {
                ident.push(b[i]);
                i += 1;
            }
            bang = b.get(i) == Some(&'!');
            if bang {
                i += 1;
            }
            continue;
        }
        match c {
            '(' => stack.push(Frame::Call { name: std::mem::take(&mut ident), arg: 0, transparent: false }),
            '[' | '{' if bang => stack.push(Frame::Call { name: std::mem::take(&mut ident), arg: 0, transparent: true }),
            // (`x[k]`, an index, is no list of texts: what is in it is not shown)
            '[' if b[..i].iter().rev().find(|c| !c.is_whitespace()).is_some_and(|p| p.is_alphanumeric() || matches!(p, '_' | ')' | ']')) => stack.push(Frame::Block),
            '[' => stack.push(Frame::Array),
            '{' => {
                if test_next && test_at.is_none() {
                    test_at = Some(stack.len());
                    test_next = false;
                }
                stack.push(Frame::Block);
            }
            ')' | ']' | '}' => {
                stack.pop();
                if c == '}' && test_at == Some(stack.len()) {
                    test_at = None;
                }
            }
            ',' => {
                if let Some(Frame::Call { arg, .. }) = stack.last_mut() {
                    *arg += 1;
                }
            }
            ';' if test_next && test_at.is_none() && !stack.iter().any(|f| matches!(f, Frame::Call { .. })) => test_next = false,
            _ => {}
        }
        if !c.is_whitespace() && c != ':' && c != '.' && c != '&' {
            ident.clear();
            bang = false;
        }
        if c == '(' || c == '[' || c == '{' {
            bang = false;
        }
        i += 1;
    }
    out
}

/// A literal found: the call it is handed to, through arrays and `vec!`.
fn push(out: &mut Vec<Literal>, text: String, line: usize, stack: &[Frame], in_test: bool) {
    if in_test {
        return;
    }
    let mut call = None;
    for f in stack.iter().rev() {
        match f {
            Frame::Array => continue,
            Frame::Call { name, transparent: true, .. } if name == "vec" => continue,
            Frame::Call { name, arg, .. } => {
                call = Some((name.clone(), *arg));
                break;
            }
            Frame::Block => break,
        }
    }
    out.push(Literal { text, line, call });
}

/// The `.rs` files of the interface, as (path under `src` with `/`, file).
fn sources() -> Vec<(String, PathBuf)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut todo: Vec<PathBuf> = SCOPE.iter().map(|s| src.join(s)).collect();
    while let Some(p) = todo.pop() {
        if p.is_dir() {
            todo.extend(std::fs::read_dir(&p).unwrap().flatten().map(|e| e.path()));
        } else if p.extension().is_some_and(|e| e == "rs") {
            let rel = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, p));
        }
    }
    out.sort();
    out
}

/// Whether `text` is words a player reads (not an icon's name, a key, a path or a sign).
fn is_words(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphabetic()).count() >= 2 && !text.starts_with(['\u{0}', '\u{1}', '#'])
}

/// The `%{name}` placeholders of a text, sorted.
fn placeholders(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text.match_indices("%{").filter_map(|(k, _)| text[k..].find('}').map(|e| text[k..k + e + 1].to_string())).collect();
    out.sort();
    out
}

#[test]
fn every_text_the_interface_shows_is_translated_into_its_six_languages() {
    let mut checked = 0;
    let mut missing: Vec<String> = Vec::new();
    for (rel, path) in sources() {
        let src = std::fs::read_to_string(&path).unwrap();
        for lit in literals(&src) {
            let Some((name, arg)) = &lit.call else { continue };
            let own = SHOWN.iter().any(|(file, call, _)| call == name && !file.is_empty() && rel.starts_with(file));
            let shown = SHOWN.iter().any(|(file, call, args)| call == name && args.contains(arg) && if own { !file.is_empty() && rel.starts_with(file) } else { file.is_empty() || rel.starts_with(file) });
            if !shown || !is_words(&lit.text) || AS_IS.iter().any(|(t, _)| *t == lit.text) {
                continue;
            }
            checked += 1;
            for lang in LANGUAGES {
                match crate::_rust_i18n_try_translate(lang, &lit.text) {
                    None => missing.push(format!("{rel}:{} {lang}: {:?}", lit.line, lit.text)),
                    Some(t) if placeholders(&t) != placeholders(&lit.text) => missing.push(format!("{rel}:{} {lang} has other placeholders: {:?} -> {:?}", lit.line, lit.text, t)),
                    _ => {}
                }
            }
        }
    }
    assert!(checked > 1500, "only {checked} texts found: is the reading of the source broken?");
    assert!(missing.is_empty(), "{} translations missing (add them to locales/vertalingen.yml):\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn the_new_table_has_every_text_in_six_languages_once() {
    let table = include_str!("../locales/vertalingen.yml");
    let mut keys: Vec<(String, Vec<String>)> = Vec::new();
    for l in table.lines() {
        if l.starts_with('"') {
            keys.push((l.to_string(), Vec::new()));
        } else if let (Some(k), Some((lang, _))) = (keys.last_mut(), l.strip_prefix("  ").and_then(|l| l.split_once(':'))) {
            k.1.push(lang.to_string());
        }
    }
    assert!(keys.len() > 100);
    let mut seen = std::collections::HashSet::new();
    for (key, langs) in &keys {
        assert!(seen.insert(key.clone()), "{key} is in the table twice");
        for lang in LANGUAGES {
            assert!(langs.iter().filter(|l| *l == lang).count() == 1, "{key} has no {lang} (or two)");
        }
    }
}

#[test]
fn the_reading_finds_the_calls_and_leaves_tests_out() {
    let src = "fn a(ui: &mut Ui) {\n    ui.button(\"id\", r, \"Save\", None, K);\n    ui.segmented(\"s\", r, &mut k, &[\"One\", \"Two\"]);\n    let x = if y { \"Not seen\" } else { tr(\"Seen\") };\n    let c = '\"'; tr(\"After a quote\");\n}\n#[cfg(test)]\nmod tests {\n    fn t() { tr(\"Test only\"); }\n}\nfn b() { tr(r#\"Raw \"one\"\"#); }\n";
    let found: Vec<(String, Option<(String, usize)>)> = literals(src).into_iter().map(|l| (l.text, l.call)).collect();
    let call = |n: &str, a: usize| Some((n.to_string(), a));
    assert_eq!(found[1], ("Save".to_string(), call("button", 2)));
    assert_eq!(found[3], ("One".to_string(), call("segmented", 3)));
    assert_eq!(found[4], ("Two".to_string(), call("segmented", 3)));
    assert_eq!(found[5], ("Not seen".to_string(), None));
    assert_eq!(found[6], ("Seen".to_string(), call("tr", 0)));
    assert_eq!(found[7], ("After a quote".to_string(), call("tr", 0)));
    assert!(!found.iter().any(|f| f.0 == "Test only"));
    assert_eq!(found.last().unwrap(), &("Raw \"one\"".to_string(), call("tr", 0)));
    assert_eq!(placeholders("%{n} of %{total}"), vec!["%{n}".to_string(), "%{total}".to_string()]);
}
