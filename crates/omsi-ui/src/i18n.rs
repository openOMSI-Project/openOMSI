//! The interface's language. Every text the painter draws or measures goes through [`tr`]:
//! the English text is the key, and the application hands over a lookup (its rust-i18n
//! tables) and the language to look it up in. English, or a text the tables do not have
//! (a bus's name, a number), is drawn as it is.

use std::borrow::Cow;
use std::sync::RwLock;

/// A lookup: (language, English text) → the text in that language.
pub type Lookup = fn(&str, &str) -> Option<String>;

static STATE: RwLock<(Option<Lookup>, String)> = RwLock::new((None, String::new()));

/// What is asked when the tables have no translation (the application's machine
/// translation: a text it has translated already, None while it is still at it).
static FALLBACK: RwLock<Option<Lookup>> = RwLock::new(None);

/// The lookup for texts the tables do not have.
pub fn set_fallback(f: Option<Lookup>) {
    if let Ok(mut s) = FALLBACK.write() {
        *s = f;
    }
}

/// The lookup to translate with.
pub fn set_lookup(f: Lookup) {
    if let Ok(mut s) = STATE.write() {
        s.0 = Some(f);
    }
}

/// The language to show (`ru`, `de`, `fr`; empty or `en` for English).
pub fn set_language(code: &str) {
    if let Ok(mut s) = STATE.write() {
        s.1 = if code.eq_ignore_ascii_case("en") { String::new() } else { code.to_ascii_lowercase() };
    }
}

/// The language shown now (empty for English).
pub fn language() -> String {
    STATE.read().map(|s| s.1.clone()).unwrap_or_default()
}

#[derive(Debug, PartialEq)]
pub enum Piece {
    Text(String),
    Hole(String),
}

pub type Template = (String, Vec<Piece>);

static TEMPLATES: RwLock<Vec<Template>> = RwLock::new(Vec::new());

pub fn set_templates(keys: impl IntoIterator<Item = String>) {
    let t = templates(keys);
    if let Ok(mut s) = TEMPLATES.write() {
        *s = t;
    }
}

pub fn templates(keys: impl IntoIterator<Item = String>) -> Vec<Template> {
    let mut t: Vec<Template> = keys
        .into_iter()
        .filter(|k| k.contains('{') && !k.contains("%{") && !k.contains("{{") && !k.contains("}}"))
        .filter_map(|k| {
            let p = pieces(&k)?;
            let head = matches!(p.first(), Some(Piece::Text(h)) if h.chars().count() >= 2);
            let worded = p.iter().filter(|x| matches!(x, Piece::Text(s) if s.chars().any(char::is_alphanumeric))).count() >= 2;
            let closed = matches!(p.last(), Some(Piece::Text(_)));
            let apart = p.windows(2).all(|w| !matches!(w, [Piece::Hole(_), Piece::Hole(_)]));
            (head && (worded || closed) && apart).then_some((k, p))
        })
        .collect();
    t.sort_by_key(|(_, p)| std::cmp::Reverse(p.iter().map(|x| if let Piece::Text(s) = x { s.len() } else { 0 }).sum::<usize>()));
    t
}

fn pieces(s: &str) -> Option<Vec<Piece>> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}')?;
        let inner = &rest[open + 1..close];
        if inner.contains('{') {
            return None;
        }
        if open > 0 {
            out.push(Piece::Text(rest[..open].to_string()));
        }
        out.push(Piece::Hole(inner.split(':').next().unwrap_or("").to_string()));
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest.to_string()));
    }
    Some(out)
}

fn captures<'a>(pieces: &[Piece], text: &'a str, out: &mut Vec<&'a str>) -> bool {
    match pieces.split_first() {
        None => text.is_empty(),
        Some((Piece::Text(t), rest)) => text.strip_prefix(t.as_str()).is_some_and(|r| captures(rest, r, out)),
        Some((Piece::Hole(_), rest)) => match rest.first() {
            None => {
                out.push(text);
                true
            }
            Some(Piece::Text(t)) => text.match_indices(t.as_str()).any(|(i, _)| {
                out.push(&text[..i]);
                captures(rest, &text[i..], out) || {
                    out.pop();
                    false
                }
            }),
            Some(Piece::Hole(_)) => false,
        },
    }
}

fn fill(key: &[Piece], got: &[&str], translation: &str) -> Option<String> {
    let holes: Vec<(&str, &str)> = key.iter().filter_map(|p| if let Piece::Hole(n) = p { Some(n.as_str()) } else { None }).zip(got.iter().copied()).collect();
    let mut positional = holes.iter().filter(|(n, _)| n.is_empty()).map(|(_, v)| *v);
    let mut out = String::new();
    for p in pieces(translation)? {
        match p {
            Piece::Text(t) => out.push_str(&t),
            Piece::Hole(n) if n.is_empty() => out.push_str(positional.next()?),
            Piece::Hole(n) => out.push_str(holes.iter().find(|(m, _)| *m == n)?.1),
        }
    }
    Some(out)
}

pub fn templated(templates: &[Template], text: &str, lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut got = Vec::new();
    let (key, p) = templates.iter().find(|(_, p)| {
        got.clear();
        matches!(p.first(), Some(Piece::Text(h)) if text.starts_with(h.as_str())) && captures(p, text, &mut got)
    })?;
    let translation = lookup(key)?;
    let values: Vec<String> = got.iter().map(|v| lookup(v).unwrap_or_else(|| v.to_string())).collect();
    fill(p, &values.iter().map(String::as_str).collect::<Vec<_>>(), &translation)
}

/// `text` in the interface's language.
pub fn tr(text: &str) -> Cow<'_, str> {
    let Ok(s) = STATE.read() else { return Cow::Borrowed(text) };
    match (&s.0, s.1.is_empty()) {
        (Some(f), false) if !text.is_empty() => match f(&s.1, text).or_else(|| TEMPLATES.read().ok().and_then(|t| templated(&t, text, |k| f(&s.1, k)))) {
            Some(t) => Cow::Owned(t),
            None => match FALLBACK.read().ok().and_then(|g| *g).and_then(|g| g(&s.1, text)) {
                Some(t) => Cow::Owned(t),
                None => Cow::Borrowed(text),
            },
        },
        _ => Cow::Borrowed(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(key: &str) -> Option<String> {
        match key {
            "Mirror panel added ({} in all)" => Some("Panneau ajouté ({} au total)".into()),
            "Line {line}, tour {}" => Some("Ligne {line}, service {}".into()),
            "Line {name} is there already" => Some("La ligne {name} existe déjà".into()),
            "Panel shows mirror {} of {}" => Some("Le panneau montre le rétroviseur {} sur {}".into()),
            "Object {id} {name}: moved {:+.2} m" => Some("Objet {name} ({id}) : déplacé de {:+.2} m".into()),
            "Air pressure is low ({tank}spring brake {:.1} bar)" => Some("Pression d'air basse ({tank}frein à ressort {:.1} bar)".into()),
            "Line {} {} 3" => Some("Ligne {} {} 3".into()),
            "Driving keys: {name} (Settings)" => Some("Touches de conduite : {name} (Paramètres)".into()),
            "Arrow keys only" => Some("Flèches seulement".into()),
            _ => None,
        }
    }

    fn all() -> Vec<Template> {
        let keys = ["Mirror panel added ({} in all)", "Line {name} is there already", "Line {line}, tour {}", "Line {line} · tour {t}", "Line {} {} 3", "Panel shows mirror {} of {}", "Object {id} {name}: moved {:+.2} m", "Air pressure is low ({tank}spring brake {:.1} bar)", "Driving keys: {name} (Settings)"];
        templates(keys.iter().map(|k| k.to_string()))
    }

    #[test]
    fn a_formatted_text_is_translated_by_its_template() {
        let t = all();
        assert_eq!(templated(&t, "Mirror panel added (3 in all)", fr).as_deref(), Some("Panneau ajouté (3 au total)"));
        assert_eq!(templated(&t, "Panel shows mirror 2 of 5", fr).as_deref(), Some("Le panneau montre le rétroviseur 2 sur 5"));
        assert_eq!(templated(&t, "Object 12 Haus.sco: moved +1.50 m", fr).as_deref(), Some("Objet Haus.sco (12) : déplacé de +1.50 m"));
        assert_eq!(templated(&t, "Air pressure is low (spring brake 4.2 bar)", fr).as_deref(), Some("Pression d'air basse (frein à ressort 4.2 bar)"));
        assert_eq!(templated(&t, "Line M41, tour 3", fr).as_deref(), Some("Ligne M41, service 3"));
        assert_eq!(templated(&t, "Line M41 is there already", fr).as_deref(), Some("La ligne M41 existe déjà"));
    }

    #[test]
    fn a_text_no_template_fits_is_left_alone() {
        let t = all();
        assert_eq!(templated(&t, "Mirror panel added", fr), None);
        assert_eq!(templated(&t, "Mirror panel added (3 in all) now", fr), None);
        assert_eq!(templated(&t, "Line and tour", fr), None);
        assert_eq!(templated(&t, "Panneau ajouté (3 au total)", fr), None);
    }

    #[test]
    fn the_first_template_that_fits_answers_even_without_a_translation() {
        let t = all();
        assert_eq!(templated(&t, "Line 5 · tour 3", fr), None);
        assert_eq!(templated(&t, "Line 5 tour 3", fr).as_deref(), Some("Ligne 5 tour 3"));
    }

    #[test]
    fn a_value_that_is_a_text_of_the_tables_is_translated_too() {
        let t = all();
        assert_eq!(templated(&t, "Driving keys: Arrow keys only (Settings)", fr).as_deref(), Some("Touches de conduite : Flèches seulement (Paramètres)"));
        assert_eq!(templated(&t, "Driving keys: Q W E (Settings)", fr).as_deref(), Some("Touches de conduite : Q W E (Paramètres)"));
    }

    #[test]
    fn only_worded_templates_with_apart_placeholders_count() {
        let keys = ["+{} more", "{} more", "Plain text", "Parts missing: needs %{packs}", "Braces {{}} here {} too", "Weather: {name}", "Clear {}", "Hat {} {}", "Radio {}: {}", "Clock: {:02}:{:02}", "Line {line}{} · {} more", "Joined {name}{how} - choose"];
        let t: Vec<String> = templates(keys.iter().map(|k| k.to_string())).into_iter().map(|(k, _)| k).collect();
        assert!(t.is_empty(), "{t:?}");
        let t: Vec<String> = templates(["Head tracking with opentrack (UDP port {})", "Panel shows mirror {} of {}"].map(String::from)).into_iter().map(|(k, _)| k).collect();
        assert_eq!(t.len(), 2, "{t:?}");
        let t = templates(["Gear {} (H-pattern)".to_string()]);
        assert_eq!(t[0].1, vec![Piece::Text("Gear ".into()), Piece::Hole(String::new()), Piece::Text(" (H-pattern)".into())]);
    }
}
