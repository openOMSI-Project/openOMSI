//! OMSI 2's tutorials (`Tutorials/<n>`): four lessons, each a situation (OMSI
//! TTutorialMan: Strg.osn, Fast.osn, LIN.osn, SPEZ.osn) and a run of pages - `<step>.html`
//! in the language's folder with `<step>.jpg` beside them, stepped through in the order of
//! their numbers. The original moves on by itself when the step's action is done (the key
//! pressed, the switch thrown: TTutorialMan.virtual_00 checks them); here Enter or Page
//! Down moves on and Page Up back.

use std::path::{Path, PathBuf};

pub const SITUATIONS: [&str; 4] = ["Tutorials/STRG.osn", "Tutorials/FAST.osn", "Tutorials/LIN.osn", "Tutorials/SPEZ.osn"];

pub struct Page {
    pub title: String,
    pub text: String,
    pub image: Option<PathBuf>,
}

pub struct Tutorial {
    pub pages: Vec<Page>,
    pub at: usize,
    pub hidden: bool,
}

/// The text of a tutorial page: the heading and the paragraphs, without the markup.
pub fn page_text(html: &str) -> (String, String) {
    let body = html.split_once("</style>").map(|x| x.1).unwrap_or(html);
    let title = body
        .split_once("<h2>")
        .and_then(|(_, r)| r.split_once("</h2>"))
        .map(|(t, _)| strip(t))
        .unwrap_or_default();
    let rest = body.split_once("</h2>").map(|x| x.1).unwrap_or(body);
    let text = strip(&rest.replace("</p>", "\n").replace("<br>", "\n").replace("<li>", "\n• "));
    let lines: Vec<String> = text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect();
    (title, lines.join("\n"))
}

fn strip(s: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    omsi_launcher_lib::decode_html_entities(&out)
}

impl Tutorial {
    /// Tutorial `number` (1..4) in the language (`ENG`, `DEU`, `FRA`; English when the
    /// folder is missing).
    pub fn load(root: &Path, number: usize, lang: &str) -> Option<Tutorial> {
        let dir = root.join("Tutorials").join(number.to_string());
        let lang_dir = [lang, "ENG", "DEU"].iter().map(|l| dir.join(l)).find(|d| d.is_dir())?;
        let mut steps: Vec<(u64, PathBuf)> = std::fs::read_dir(&lang_dir)
            .ok()?
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let n = p.file_stem()?.to_string_lossy().parse::<u64>().ok()?;
                (p.extension()?.eq_ignore_ascii_case("html")).then_some((n, p))
            })
            .collect();
        steps.sort();
        let pages = steps
            .into_iter()
            .filter_map(|(n, p)| {
                let bytes = std::fs::read(&p).ok()?;
                let (title, text) = page_text(&omsi_cfg::codepage::decode(&bytes));
                let img = dir.join(format!("{n}.jpg"));
                Some(Page { title, text, image: img.is_file().then_some(img) })
            })
            .collect::<Vec<_>>();
        log::info!("tutorial {number}: {} pages from {}", pages.len(), lang_dir.display());
        (!pages.is_empty()).then_some(Tutorial { pages, at: 0, hidden: false })
    }

    pub fn next(&mut self) {
        self.at = (self.at + 1).min(self.pages.len().saturating_sub(1));
    }

    pub fn back(&mut self) {
        self.at = self.at.saturating_sub(1);
    }

    pub fn page(&self) -> Option<&Page> {
        self.pages.get(self.at)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn text() {
        let (t, x) = super::page_text("<style>b{}</style><h2>Hello!</h2><p>One &quot;two&quot;</p><p>Three</p>");
        assert_eq!(t, "Hello!");
        assert_eq!(x, "One \"two\"\nThree");
    }

    #[test]
    fn german_html_entities() {
        let (title, text) = super::page_text("<h2>Fahrg&auml;ste</h2><p>Men&uuml; &amp; T&#252;ren &ndash; gr&ouml;&szlig;er</p>");
        assert_eq!(title, "Fahrgäste");
        assert_eq!(text, "Menü & Türen – größer");
        let (_, text) = super::page_text("<h2>x</h2><p>unter 0&deg;C, the AI &#8203;&#8203;vehicles &unknown; R&D</p>");
        assert_eq!(text, "unter 0°C, the AI vehicles &unknown; R&D");
    }
}
