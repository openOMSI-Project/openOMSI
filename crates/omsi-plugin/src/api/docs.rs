//! The reference part of docs/PLUGINS.md, written from the registry: a table per group of
//! functions and one of the events. A test keeps the file's copy the same as this
//! (`OMSI_API_BLESS=1 cargo test -p omsi-plugin api_manifest` writes it).

use super::{events, registry, ApiFn, Perm};

/// The groups in the order of the documentation, with their headings.
pub const GROUPS: &[(&str, &str)] = &[
    ("vehicle", "The player's bus"),
    ("duty", "Duty and timetable"),
    ("map", "The map"),
    ("traffic", "AI traffic"),
    ("people", "People"),
    ("time", "Time"),
    ("weather", "Weather"),
    ("camera", "Camera"),
    ("input", "Input"),
    ("audio", "Sound"),
    ("ui", "On screen"),
    ("events", "Events, timers and watches"),
    ("plugin", "The plugin itself"),
    ("storage", "Storage and files"),
    ("network", "Programs on this computer"),
    ("lan", "LAN games"),
    ("game", "The game"),
    ("util", "Helpers"),
];

/// Where the generated part starts and ends in docs/PLUGINS.md.
pub const BEGIN: &str = "<!-- api:begin (written from the registry: OMSI_API_BLESS=1 cargo test -p omsi-plugin api_manifest) -->";
pub const END: &str = "<!-- api:end -->";

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn signature(f: &ApiFn) -> String {
    let params: Vec<String> = f.params.iter().map(|p| if p.optional { format!("[{}]", p.name) } else { p.name.to_string() }).collect();
    format!("`omsi.{}({})`", f.name, params.join(", "))
}

/// The generated Markdown.
pub fn markdown() -> String {
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push_str("\n\n");
    for (group, title) in GROUPS {
        let fns: Vec<&&ApiFn> = registry().iter().filter(|f| f.group == *group).collect();
        if fns.is_empty() {
            continue;
        }
        out.push_str(&format!("#### {title}\n\n| Function | Returns | What it does | Since |\n| --- | --- | --- | --- |\n"));
        for f in fns {
            let perm = if f.perm == Perm::None { String::new() } else { format!(" *Permission: `{}`.*", f.perm.as_str()) };
            out.push_str(&format!("| {} | {} | {}{} | {} |\n", signature(f), cell(f.returns), cell(f.doc), perm, f.since));
        }
        out.push('\n');
    }
    out.push_str("#### Events\n\n| Event | Arguments | When | Since |\n| --- | --- | --- | --- |\n");
    for e in events::EVENTS {
        let args = if e.args.is_empty() { "-".to_string() } else { e.args.iter().map(|a| format!("`{a}`")).collect::<Vec<_>>().join(", ") };
        out.push_str(&format!("| `{}` | {} | {} | {} |\n", e.name, args, cell(e.doc), e.since));
    }
    out.push('\n');
    out.push_str(END);
    out
}

/// Replace the generated part of a docs text (None: it has no markers).
pub fn splice(doc: &str) -> Option<String> {
    let a = doc.find(BEGIN)?;
    let b = doc[a..].find(END)? + a + END.len();
    Some(format!("{}{}{}", &doc[..a], markdown(), &doc[b..]))
}
