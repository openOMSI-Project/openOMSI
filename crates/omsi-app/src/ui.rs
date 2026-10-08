//! The game's own interface, drawn over the picture in Roboto with a dark outline (the
//! HUD's `.oft` fonts are OMSI's and stay for the time and speed): the chat of a LAN
//! session, the name of what the cursor points at next to the cursor, and the other
//! players' name tags above their buses.
//!
//! Every text is rendered once into a small texture and kept while it is shown; the
//! overlays are rectangles in physical pixels (`Scene::overlays`).

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, VariableFont};
use omsi_render::{Renderer, Scene, TextureId};

/// Roboto (Apache 2.0), the interface font.
const ROBOTO: &[u8] = include_bytes!("../../../assets/fonts/Roboto-VariableFont_wdth,wght.ttf");

/// A rendered text: its texture and size in pixels.
#[derive(Clone, Copy)]
struct Label {
    tex: TextureId,
    w: u32,
    h: u32,
    used: u64,
}

/// Or'ed into a label's pixel size: the text in the bold weight.
const BOLD: u32 = 1 << 31;

/// Texts rendered into textures, kept while they are used.
pub struct TextCache {
    font: FontVec,
    /// The same Roboto at 700: the menu's titles and its primary button, as the launcher
    /// draws them (`Weight::Bold`); asked for with `BOLD` in the size.
    bold: FontVec,
    labels: hashbrown::HashMap<(String, u32, [u8; 4]), Label>,
    frame: u64,
    /// How strongly the background plates are drawn this frame (`backdrop`).
    backdrop: f32,
    /// No outline round the texts that ask for none (the game menu's: flat text on its card).
    flat: bool,
}

impl TextCache {
    pub fn new() -> Option<TextCache> {
        let mut font = FontVec::try_from_vec(ROBOTO.to_vec()).ok()?;
        // a little heavier than the regular 400: light text on a dark panel is thin and
        // greyish at menu sizes otherwise
        let _ = font.set_variation(b"wght", 500.0);
        let mut bold = FontVec::try_from_vec(ROBOTO.to_vec()).ok()?;
        let _ = bold.set_variation(b"wght", 700.0);
        Some(TextCache { font, bold, labels: hashbrown::HashMap::new(), frame: 0, backdrop: 1.0, flat: false })
    }

    /// The texture of `text` at `px` pixels in `color` (alpha = opacity of the outline), and
    /// its size.
    fn label(&mut self, r: &Renderer, scene: &mut Scene, text: &str, px: u32, color: [u8; 4]) -> Label {
        let color = [color[0], color[1], color[2], outline_for(color, if self.flat { 1.0 } else { self.backdrop })];
        // (in the interface's language: the menu, the notes, the windows; a letter and its
        // combining mark as one, as macOS gives file names - the weather "Eiseska\u{308}lte"
        // showed a box after its "a" in the menu's list)
        let text = omsi_ui::tr(text);
        let text = &*omsi_ui::text::composed(&text);
        let key = (text.to_string(), px, color);
        if let Some(l) = self.labels.get_mut(&key) {
            l.used = self.frame;
            return *l;
        }
        let font = if px & BOLD != 0 { &self.bold } else { &self.font };
        let img = render_text(font, text, (px & !BOLD) as f32, color);
        let tex = r.add_texture(scene, &img, false);
        let l = Label { tex, w: img.width, h: img.height, used: self.frame };
        self.labels.insert(key, l);
        l
    }

    /// Text width in pixels, without rendering it.
    pub fn width(&self, text: &str, px: f32) -> f32 {
        let text = &*omsi_ui::tr(text);
        self.width_raw(text, px)
    }

    /// `width` of a text drawn in the bold weight (`BOLD`).
    pub fn width_bold(&self, text: &str, px: f32) -> f32 {
        let text = &*omsi_ui::tr(text);
        self.width_in(&self.bold, text, px)
    }

    fn width_raw(&self, text: &str, px: f32) -> f32 {
        self.width_in(&self.font, text, px)
    }

    fn width_in(&self, base: &FontVec, text: &str, px: f32) -> f32 {
        let text = omsi_ui::text::composed(text);
        let mut w = 0.0;
        let mut prev: Option<(ab_glyph::GlyphId, *const FontVec)> = None;
        for c in text.chars() {
            let font = font_for(base, c);
            let f = font.as_scaled(PxScale::from(px));
            let id = f.glyph_id(c);
            if let Some((p, pf)) = prev {
                if std::ptr::eq(pf, font) {
                    w += f.kern(p, id);
                }
            }
            w += f.h_advance(id);
            prev = Some((id, font as *const FontVec));
        }
        w + outline_px(px) * 2.0 + 2.0
    }

    /// End of a frame: labels not used for a few seconds are released.
    pub fn end_frame(&mut self, r: &Renderer, scene: &mut Scene) {
        self.frame += 1;
        if self.frame % 120 == 0 {
            let old: Vec<_> = self.labels.iter().filter(|(_, l)| self.frame.saturating_sub(l.used) > 240).map(|(k, _)| k.clone()).collect();
            for k in old {
                if let Some(l) = self.labels.remove(&k) {
                    r.free_texture(scene, l.tex);
                }
            }
        }
    }
}

/// How opaque a label's dark outline is (the alpha of its `color`): as asked, or - for a
/// panel's text, asked without one - more as the opacity setting thins the panels
/// (`backdrop` below 1): on a see-through panel over a bright sky the dim lines could not be
/// read. Light texts only: round a dark one (the highlighted menu line's, on the solid
/// accent) the outline is as dark as the glyphs and smeared them, like a shadow.
fn outline_for(color: [u8; 4], backdrop: f32) -> u8 {
    let light = 0.299 * color[0] as f32 + 0.587 * color[1] as f32 + 0.114 * color[2] as f32 > 100.0;
    match color[3] {
        0 if backdrop < 1.0 && light => (((1.0 - backdrop) * 1.6).min(0.9) * 255.0) as u8,
        a => a,
    }
}

/// The outline (UIStroke) around the glyphs, in pixels.
fn outline_px(px: f32) -> f32 {
    (px / 9.0).clamp(1.0, 3.0)
}

/// The font that draws `c`: Roboto, else the system's font for the script (Chinese,
/// Japanese, Korean, Thai, Hindi - the menu was a column of boxes in those languages).
fn font_for(roboto: &FontVec, c: char) -> &FontVec {
    if omsi_ui::text::needs_fallback(roboto, c) {
        if let Some(f) = omsi_ui::text::fallback_font(c) {
            return f;
        }
    }
    roboto
}

/// `text` as straight-alpha RGBA: the glyphs in `color` over a dark outline.
fn render_text(font: &FontVec, text: &str, px: f32, color: [u8; 4]) -> omsi_texture::Image {
    let f = font.as_scaled(PxScale::from(px));
    let stroke = outline_px(px);
    let pad = stroke.ceil() as i32 + 1;
    let asc = f.ascent();
    let h = (asc - f.descent()).ceil() as i32 + pad * 2;
    // the baseline and every glyph on whole pixels: glyphs drawn between pixels came out
    // different from letter to letter and soft (the pen itself still runs on fractions, so
    // the text keeps its width)
    let base = (pad as f32 + asc).round();
    // lay the glyphs out
    let mut glyphs: Vec<(&FontVec, ab_glyph::Glyph)> = Vec::new();
    let mut x = pad as f32;
    let mut prev: Option<(ab_glyph::GlyphId, *const FontVec)> = None;
    for c in text.chars() {
        let gf = font_for(font, c);
        let sf = gf.as_scaled(PxScale::from(px));
        let id = sf.glyph_id(c);
        if let Some((p, pf)) = prev {
            if std::ptr::eq(pf, gf) {
                x += sf.kern(p, id);
            }
        }
        glyphs.push((gf, id.with_scale_and_position(PxScale::from(px), ab_glyph::point(x.round(), base))));
        x += sf.h_advance(id);
        prev = Some((id, gf as *const FontVec));
    }
    let w = (x.ceil() as i32 + pad).max(1);
    let (wu, hu) = (w as usize, h.max(1) as usize);
    let mut cov = vec![0f32; wu * hu];
    for (gf, g) in glyphs {
        if let Some(o) = gf.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, c| {
                let xx = b.min.x as i32 + gx as i32;
                let yy = b.min.y as i32 + gy as i32;
                if xx >= 0 && yy >= 0 && (xx as usize) < wu && (yy as usize) < hu {
                    let i = yy as usize * wu + xx as usize;
                    cov[i] = (cov[i] + c).min(1.0);
                }
            });
        }
    }
    // the texture is sRGB and blended in linear light: light text on a dark panel comes out
    // too fat and blurry, dark text on a light one too thin - bend the coverage to match
    let light = 0.299 * color[0] as f32 + 0.587 * color[1] as f32 + 0.114 * color[2] as f32 > 100.0;
    let gamma = if light { 1.45 } else { 0.8 };
    for c in cov.iter_mut() {
        if *c > 0.0 && *c < 1.0 {
            *c = c.powf(gamma);
        }
    }
    // the outline: the coverage grown by the stroke radius
    let r = stroke;
    let ri = r.ceil() as i32;
    let mut edge = vec![0f32; wu * hu];
    for y in 0..hu as i32 {
        for x in 0..wu as i32 {
            let mut m = 0f32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    if d > r + 0.5 {
                        continue;
                    }
                    let (sx, sy) = (x + dx, y + dy);
                    if sx < 0 || sy < 0 || sx >= wu as i32 || sy >= hu as i32 {
                        continue;
                    }
                    let k = (r + 0.5 - d).clamp(0.0, 1.0);
                    m = m.max(cov[sy as usize * wu + sx as usize] * k);
                }
            }
            edge[y as usize * wu + x as usize] = m;
        }
    }
    let oa = color[3] as f32 / 255.0;
    let mut rgba = vec![0u8; wu * hu * 4];
    for i in 0..wu * hu {
        let a_text = cov[i];
        let a_edge = edge[i] * oa;
        let a = a_text + a_edge * (1.0 - a_text);
        if a <= 0.0 {
            continue;
        }
        for c in 0..3 {
            let v = (color[c] as f32 * a_text + 12.0 * a_edge * (1.0 - a_text)) / a;
            rgba[i * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
        }
        rgba[i * 4 + 3] = (a * 255.0).round() as u8;
    }
    omsi_texture::Image { width: wu as u32, height: hu as u32, rgba, has_alpha: true }
}

// ---------------------------------------------------------------------------------------
// the chat

/// How many rows the chat shows (a long line takes several), and how many lines it keeps to
/// scroll back.
const CHAT_SHOWN: usize = 8;
pub const CHAT_KEEP: usize = 200;

/// A notification from the server (`notify`): a card over the navigator that goes by itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
    /// Seconds it still shows, of `total`.
    pub left: f32,
    pub total: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoticeKind {
    Info,
    Warn,
    Alert,
    /// A new openOMSI version (`update_watch`).
    Update,
}

/// How many notifications show at once (the newest; an older one makes room).
pub const NOTICES_SHOWN: usize = 3;

impl Notice {
    /// `notify`'s argument: `<id> <seconds> <info|warn|alert> <text>` → the id and the notice
    /// (seconds held to 2 .. 30; an unknown kind is information).
    pub fn parse(arg: &str) -> Option<(String, Notice)> {
        let mut it = arg.trim().splitn(4, ' ');
        let id = it.next().filter(|s| !s.is_empty())?.to_string();
        let secs: f32 = it.next()?.parse().ok().filter(|s: &f32| s.is_finite())?;
        let kind = match it.next()? {
            "warn" => NoticeKind::Warn,
            "alert" => NoticeKind::Alert,
            _ => NoticeKind::Info,
        };
        let text = it.next().map(str::trim).filter(|t| !t.is_empty())?.to_string();
        let total = secs.clamp(2.0, 30.0);
        Some((id, Notice { kind, text, left: total, total }))
    }

    /// How opaque it is: fading in for a quarter second, out for its last half second.
    pub fn alpha(&self) -> f32 {
        (self.left / 0.5).min((self.total - self.left) / 0.25).clamp(0.0, 1.0)
    }
}

/// A new notice in the list, the oldest going when there are more than `NOTICES_SHOWN`.
pub fn push_notice(list: &mut Vec<Notice>, n: Notice) {
    list.push(n);
    while list.len() > NOTICES_SHOWN {
        list.remove(0);
    }
}

/// What the chat widget needs from the session each frame.
pub struct ChatView<'a> {
    /// Every line, oldest first ("Name: text" or "* notice").
    pub lines: &'a [String],
    /// The line being typed (the input box is open).
    pub typing: Option<&'a str>,
    /// Why the last line was not sent.
    pub error: Option<&'a str>,
}

/// The chat's own state: shown or hidden (V), the scroll position and whether the cursor is
/// over it.
#[derive(Default)]
pub struct ChatWidget {
    pub hidden: bool,
    /// Lines scrolled back from the newest.
    pub scroll: usize,
    /// The chat's box on the screen (physical pixels) as drawn last: hovering over it shows
    /// the input box, a click there opens it.
    pub rect: [f32; 4],
    pub hovered: bool,
    pub caret_t: f32,
}

impl ChatWidget {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.rect[0] && x <= self.rect[2] && y >= self.rect[1] && y <= self.rect[3]
    }

    /// The mouse wheel over the chat (or while typing) scrolls the history.
    pub fn wheel(&mut self, lines: usize, amount: f32) {
        let max = lines.saturating_sub(CHAT_SHOWN);
        let s = self.scroll as i32 + amount.round() as i32;
        self.scroll = s.clamp(0, max as i32) as usize;
    }
}

/// How much larger than designed the interface is drawn, on top of the screen's scale
/// `dpi`: on a window taller than 1080 logical pixels as much as it is taller (up to twice),
/// times the player's `size` (`Settings::ui_scale`). Laid out for 1080p, the texts were half
/// their size on a 4K screen at 100 % display scaling; a window of 1080 lines or fewer is
/// drawn as it always was. With `window` off (`Settings::ui_scale_window`) it does not grow
/// with the window at all.
pub fn size_factor(height_px: f32, dpi: f32, size: f32, window: bool) -> f32 {
    let grown = if window { (height_px / dpi.max(0.5) / 1080.0).clamp(1.0, 2.0) } else { 1.0 };
    grown * size
}

/// How strongly the interface's backgrounds are drawn for the opacity setting
/// (`Settings::ui_opacity`): 1 at its default of 85 %, as designed; lower, the picture shows
/// through them (never less than 0.3), higher a little darker. The texts stay solid.
pub fn backdrop(opacity: f32) -> f32 {
    (opacity / 0.85).clamp(0.3, 1.3)
}

/// Which menu is open: its layout follows from it.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum MenuKind {
    /// The game menu itself.
    #[default]
    Game,
    /// A settings window (options, vehicle, world): a sidebar of pages and rows with switches,
    /// sliders and buttons. A row's label is `name\u{1f}kind\u{1f}value\u{1f}description\u{1f}fraction`
    /// (kind: `s` switch, `v` slider, `c` stepper, `o` opens a list, `a` button, `i` information).
    Options,
    /// The lines of the map's timetable.
    Lines,
    /// One line's tours.
    Tours,
    /// Any other list (drivers, fleet numbers, destinations, liveries ...).
    List,
}

/// The timetable beside a list of lines or tours: a title, a line of facts and rows of
/// (what, time).
pub struct Preview {
    pub title: String,
    pub meta: String,
    pub rows: Vec<(String, String)>,
    /// The row chosen, when the rows can be chosen (the stops to start a tour from).
    pub chosen: Option<usize>,
    /// The button under the rows, when there is one.
    pub button: Option<String>,
    /// The time of the timetable the rows are of, with buttons to put it ahead (the trip of
    /// a tour to take on later), when it can be set.
    pub time: Option<String>,
}


/// A drop-down open over a row of a settings window: the entries, the one the keyboard is
/// on, the first one shown and the one in force now.
pub struct DropdownView<'a> {
    pub row: usize,
    pub items: Vec<&'a str>,
    pub sel: usize,
    pub top: usize,
    pub current: Option<usize>,
}

/// Everything the interface draws in a frame.
pub struct Frame<'a> {
    /// Physical pixels per logical one.
    pub scale: f32,
    /// How much larger than designed (`size_factor`): everything below is drawn this much
    /// larger on top of `scale`.
    pub ui_scale: f32,
    /// How strongly the backgrounds are drawn (`backdrop`): the menu's and the timetable's
    /// panels, the chat's box, the plates under the notes, the tooltip and the frame rate.
    pub opacity: f32,
    pub width: f32,
    pub height: f32,
    pub cursor: (f32, f32),
    /// The mouse steering's cross, while the system cursor is hidden and held (where the
    /// crosshair cursor stood before): drawn here instead.
    pub steer_cross: Option<(f32, f32)>,
    /// An OpenXR headset is drawing this frame.
    pub vr: bool,
    /// The name of what the cursor points at (a switch, a part), shown next to it.
    pub tooltip: Option<String>,
    /// The chat, when a LAN session runs and the chat is not switched off.
    pub chat: Option<ChatView<'a>>,
    /// The chat's own size on top of `ui_scale` (`Settings::chat_size`).
    pub chat_size: f32,
    /// What the driver has to act on (why the bus does not move, a passenger's wish, the
    /// change due, a service done), top left.
    pub notes: &'a [String],
    /// The frame rate, top right (the `show_fps` setting).
    pub fps: Option<f32>,
    /// The game stands paused.
    pub paused: bool,
    /// The game menu is open, with this line chosen (labels from `GAME_MENU`).
    pub menu: Option<(usize, &'a [(&'a str, &'a str)])>,
    /// The first line shown when a finger scrolled the menu (`App::menu_top`).
    pub menu_top: Option<f32>,
    /// Ids of the game menu's lines that are greyed out and cannot be chosen (the timetable
    /// without an active route).
    pub menu_disabled: &'a [&'a str],
    /// The timetable window: its title and per stop (name, time, 0 served / 1 next / 2 ahead).
    pub timetable: Option<(String, Vec<(String, String, u8)>)>,
    /// The information bar along the top: its parts between [`INFO_SEP`]s.
    pub info: Option<String>,
    /// The room the information bar has along the top: from x to x, from y down (on a
    /// touch screen the gap between its buttons); none: the window's width.
    pub info_room: Option<[f32; 3]>,
    /// A tutorial page: title, text, picture, page number and count.
    pub tutorial: Option<(&'a str, &'a str, Option<&'a std::path::Path>, usize, usize)>,
    /// Name tags: a screen position (the point above a bus), the name and a second line.
    pub tags: Vec<((f32, f32), String, String, f32)>,
    /// The server's notifications, oldest first.
    pub notices: &'a [Notice],
    /// Where the navigator is on the screen (the notifications stand over it, or under it
    /// in a top corner); none: they go to the top middle.
    pub notice_anchor: Option<[f32; 4]>,
    /// What kind of menu the lines belong to.
    pub menu_kind: MenuKind,
    /// The open list's title and the small line above it (the line a tour list is of).
    pub menu_head: Option<(String, String)>,
    /// The timetable of the chosen line or tour, beside the list.
    pub menu_preview: Option<Preview>,
    /// The first stop shown of the timetable beside the tours, when the wheel has scrolled it
    /// (`None`: the stop chosen is kept in view).
    pub pane_first: Option<usize>,
    /// The pages of an open settings window (their titles) and the one shown.
    pub menu_tabs: Option<(Vec<String>, usize)>,
    /// The keyboard chose the menu's line last: that line is shown lit (else only the one
    /// under the mouse is).
    pub menu_kbd: bool,
    /// The drop-down open over a row of the settings window.
    pub dropdown: Option<DropdownView<'a>>,
}

pub struct Ui {
    origin_x: f32,
    pub text: TextCache,
    pub chat: ChatWidget,
    /// Where the game menu's lines were drawn this frame (physical pixels), for the mouse.
    pub menu_rects: Vec<[f32; 4]>,
    /// Per line of `menu_rects`, where the arrows round its value are (a list's setting,
    /// `game_lists::ADJUST`): `[from, to, plus]` - a click from `from` to `to` steps it
    /// down, one right of `plus` up.
    pub menu_arrows: Vec<Option<[f32; 3]>>,
    pub menu_scroll_thumb: Option<[f32; 4]>,
    pub menu_scroll_track: Option<[f32; 4]>,
    /// Where the controls of the settings rows were drawn (a slider's track, a stepper), one
    /// entry per line in `menu_rects`: a click there sets the value.
    pub menu_ctl: Vec<Option<[f32; 4]>>,
    /// The sidebar of a settings window: one box per page, the way back last.
    pub menu_side: Vec<[f32; 4]>,
    /// The rows of the timetable beside a line's tours (the stops to start from), the stop the
    /// first of them is, and the button that starts the trip.
    pub menu_pane: Vec<[f32; 4]>,
    pub menu_pane_start: usize,
    pub menu_pane_go: Option<[f32; 4]>,
    /// The whole timetable pane beside the tours: the wheel over it scrolls its stops.
    pub menu_pane_box: Option<[f32; 4]>,
    /// Its scroll bar when it has more stops than it shows: the track, the thumb (widened
    /// to be hit), how many stops there are and how many it shows - for the mouse to drag.
    pub menu_pane_scroll: Option<([f32; 4], [f32; 4], usize, usize)>,
    /// The two arrows beside the time of a tour: the trip before, the next one.
    pub menu_time: Vec<[f32; 4]>,
    /// The colours and positions of the menu's parts that ease to their new state (a line's
    /// light, a switch's knob ...), by what they belong to.
    anim: std::collections::HashMap<u64, f32>,
    /// Seconds since the last frame, for `ease`.
    anim_dt: f32,
    /// Overlay entries belonging to the game menu.
    pub menu_overlay_range: std::ops::Range<usize>,
    /// The pointer texture, positioned separately for each headset eye.
    pub vr_cursor_overlay: Option<usize>,
    pub vr_tooltip_overlay: Option<usize>,
    /// The first line of the menu shown (a long menu scrolls: `menu_rects[k]` is line
    /// `menu_start + k`).
    pub menu_start: usize,
    /// How many lines the menu shows at once, and how high one is (physical pixels): a
    /// finger's drag is turned into lines with it.
    pub menu_rows: usize,
    pub menu_row_h: f32,
    /// The entries of the drop-down shown (their rects), the first of them, and how many fit.
    pub dd_rects: Vec<[f32; 4]>,
    pub dd_top: usize,
    pub dd_rows: usize,
    /// The drop-down's scroll bar when it has more entries than it shows: its track and
    /// its thumb (widened to be hit), for the mouse to drag.
    pub dd_scroll: Option<([f32; 4], [f32; 4])>,
    /// Pictures shown in the interface (a tutorial page's), by file.
    images: hashbrown::HashMap<std::path::PathBuf, Option<(TextureId, u32, u32)>>,
    /// Where the information bar was drawn (within the panel, before `origin_x`), for the
    /// navigator to keep out of its way.
    pub info_rect: Option<[f32; 4]>,
}

/// Between the information bar's parts.
pub(crate) const INFO_SEP: &str = "   ·   ";

/// The information bar's parts in rows no wider than `room` (as `width` measures a text),
/// each as many parts as fit: in one line it ran off both sides of a narrow window, and on a
/// phone it lay under the buttons along the top (#1164).
pub(crate) fn info_rows(line: &str, room: f32, width: impl Fn(&str) -> f32) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for part in line.split(INFO_SEP) {
        match rows.last_mut() {
            Some(row) if width(&format!("{row}{INFO_SEP}{part}")) <= room => {
                row.push_str(INFO_SEP);
                row.push_str(part);
            }
            _ => rows.push(part.to_string()),
        }
    }
    rows
}

pub(crate) fn shift_overlays(scene: &mut Scene, start: usize, x: f32) {
    for (_, rect) in &mut scene.overlays[start..] {
        rect[0] += x;
        rect[2] += x;
    }
}

impl Ui {
    /// Lay out at panel resolution, then move both pixels and mouse targets to
    /// that panel's position in the spanning window.
    pub fn draw_at(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame, dt: f32, origin_x: f32) {
        self.shift_hitboxes(-self.origin_x);
        let start = scene.overlays.len();
        self.draw(r, scene, f, dt);
        shift_overlays(scene, start, origin_x);
        self.shift_hitboxes(origin_x);
        self.origin_x = origin_x;
    }

    fn shift_hitboxes(&mut self, x: f32) {
        for rect in self
            .menu_rects
            .iter_mut()
            .chain(self.menu_side.iter_mut())
            .chain(self.menu_pane.iter_mut())
            .chain(self.menu_time.iter_mut())
            .chain(self.dd_rects.iter_mut())
            .chain(self.menu_ctl.iter_mut().flatten())
            .chain(self.menu_scroll_thumb.iter_mut())
            .chain(self.menu_scroll_track.iter_mut())
            .chain(self.menu_pane_go.iter_mut())
            .chain(self.menu_pane_box.iter_mut())
        {
            rect[0] += x;
            rect[2] += x;
        }
        for arrows in self.menu_arrows.iter_mut().flatten() {
            for at in arrows {
                *at += x;
            }
        }
        if let Some((track, thumb)) = &mut self.dd_scroll {
            for rect in [track, thumb] {
                rect[0] += x;
                rect[2] += x;
            }
        }
        if let Some((track, thumb, _, _)) = &mut self.menu_pane_scroll {
            for rect in [track, thumb] {
                rect[0] += x;
                rect[2] += x;
            }
        }
        self.chat.rect[0] += x;
        self.chat.rect[2] += x;
    }
    pub fn new() -> Option<Ui> {
        Some(Ui { origin_x: 0.0, text: TextCache::new()?, chat: ChatWidget::default(), menu_rects: Vec::new(), menu_arrows: Vec::new(), menu_scroll_thumb: None, menu_scroll_track: None, menu_ctl: Vec::new(), dd_rects: Vec::new(), dd_top: 0, dd_rows: 8, dd_scroll: None, menu_side: Vec::new(), menu_pane: Vec::new(), menu_pane_start: 0, menu_pane_go: None, menu_pane_box: None, menu_pane_scroll: None, menu_time: Vec::new(), anim: Default::default(), anim_dt: 0.0, menu_overlay_range: 0..0, vr_cursor_overlay: None, vr_tooltip_overlay: None, menu_start: 0, menu_rows: 0, menu_row_h: 1.0, images: Default::default(), info_rect: None })
    }

    /// Draw the frame's interface: its overlays go after the HUD's in `scene.overlays`.
    pub fn draw(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame, dt: f32) {
        let s = f.scale.max(0.5) * f.ui_scale;
        self.text.backdrop = f.opacity;
        // --- name tags above the other players' buses
        for ((x, y), name, sub, alpha) in &f.tags {
            let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
            let l = self.text.label(r, scene, name, (19.0 * s) as u32, [255, 255, 255, 220]);
            let x0 = x - l.w as f32 * 0.5;
            let y0 = y - l.h as f32;
            if a > 0 {
                scene.overlays.push((l.tex, [x0, y0, x0 + l.w as f32, y0 + l.h as f32]));
                if !sub.is_empty() {
                    let m = self.text.label(r, scene, sub, (12.0 * s) as u32, [210, 225, 255, 200]);
                    let mx = x - m.w as f32 * 0.5;
                    scene.overlays.push((m.tex, [mx, y0 + l.h as f32 - 3.0 * s, mx + m.w as f32, y0 + l.h as f32 - 3.0 * s + m.h as f32]));
                }
            }
        }
        // the top of the windows in the corners: the notes on the left start on the line of
        // the timetable and a tutorial page on the right (14 px down, they stood higher)
        let corner_top = 60.0 * s;
        // --- notes, top left: white on a dark outline, one line each, on a dim plate (the
        // outline alone left a note hard to read over a bright sky)
        let notes_bottom = {
            let px = (16.0 * s) as u32;
            let x0 = 16.0 * s;
            // (below the on-screen buttons of a phone, which keep their size when the
            // interface is made smaller)
            let mut y = if crate::platform::touch_controls() { (80.0 * f.scale.max(0.5) * f.ui_scale.max(1.0)).max(corner_top) } else { corner_top };
            for n in f.notes.iter().filter(|n| !n.trim().is_empty()).take(8) {
                let text = omsi_ui::tr(n);
                let text = clip_to(&self.text, &text, px as f32, f.width * 0.6);
                let l = self.text.label(r, scene, &text, px, [255, 255, 255, 235]);
                let plate = self.text.plate(r, scene, 7);
                scene.overlays.push((plate, [x0 - 5.0 * s, y, x0 + l.w as f32 + 5.0 * s, y + l.h as f32]));
                scene.overlays.push((l.tex, [x0, y, x0 + l.w as f32, y + l.h as f32]));
                y += l.h as f32 + 2.0 * s;
            }
            y
        };
        // --- the chat, top left under the notes, as Roblox has it (from the fourth note
        // on they ran into it)
        if let Some(c) = f.chat.as_ref().filter(|_| !self.chat.hidden) {
            // (its own size on top of the interface's: Ctrl + the wheel over it)
            let s = s * f.chat_size.clamp(0.5, 3.0);
            let px = (17.0 * s) as u32;
            let lh = px as f32 * 1.35;
            let x0 = 14.0 * s;
            let y0 = (96.0 * s).max(notes_bottom + 10.0 * s);
            let width = (460.0 * s).min(f.width * 0.6);
            let open = c.typing.is_some();
            let n = c.lines.len();
            let end = n.saturating_sub(if open || self.chat.hovered { self.chat.scroll } else { 0 });
            let box_h = lh * CHAT_SHOWN as f32 + lh * 1.6;
            self.chat.rect = [x0 - 6.0 * s, y0 - 6.0 * s, x0 + width, y0 + box_h];
            self.chat.hovered = self.chat.contains(f.cursor.0, f.cursor.1);
            let show_box = open || self.chat.hovered;
            // the lines, newest at the bottom of the history area; a line too long for the box
            // goes on in rows under it, indented (the oldest line shown may lose its first rows)
            let indent = 14.0 * s;
            let mut rows: Vec<(f32, String, [u8; 4])> = Vec::new();
            {
                let tc = &self.text;
                let measure = |t: &str| tc.width_raw(t, px as f32);
                let mut i = end;
                while i > 0 && rows.len() < CHAT_SHOWN {
                    i -= 1;
                    let line = &c.lines[i];
                    let color = if line.starts_with("* ") { [255, 226, 140, 230] } else { [255, 255, 255, 230] };
                    for (k, t) in wrap_rows(line, width, indent, &measure).into_iter().enumerate().rev() {
                        if rows.len() == CHAT_SHOWN {
                            break;
                        }
                        rows.push((if k == 0 { 0.0 } else { indent }, t, color));
                    }
                }
                rows.reverse();
            }
            let mut y = y0 + lh * (CHAT_SHOWN - rows.len()) as f32;
            for (dx, text, color) in rows {
                let l = self.text.label(r, scene, &text, px, color);
                scene.overlays.push((l.tex, [x0 + dx, y, x0 + dx + l.w as f32, y + l.h as f32]));
                y += lh;
            }
            if show_box {
                // the input box: shown when the cursor is over the chat or it is typed into
                let by = y0 + lh * CHAT_SHOWN as f32 + lh * 0.2;
                let bh = lh * 1.25;
                let plate = self.text.plate(r, scene, 0);
                scene.overlays.push((plate, [x0 - 4.0 * s, by, x0 + width, by + bh]));
                self.chat.caret_t += dt;
                let caret = if open && (self.chat.caret_t % 1.0) < 0.55 { "|" } else { "" };
                let (text, color) = match c.typing {
                    Some(t) => (format!("{t}{caret}"), [255, 255, 255, 240]),
                    None => ("Click here or press / to chat".to_string(), [190, 190, 190, 200]),
                };
                let text = clip_left(&self.text, &text, px as f32, width - 10.0 * s);
                let l = self.text.label(r, scene, &text, px, color);
                let ty = by + (bh - l.h as f32) * 0.5;
                scene.overlays.push((l.tex, [x0 + 2.0 * s, ty, x0 + 2.0 * s + l.w as f32, ty + l.h as f32]));
                if self.chat.scroll > 0 && (open || self.chat.hovered) {
                    // (the words translated apart from the number: whole, the text was no key)
                    let m = self.text.label(r, scene, &format!("{} {}", self.chat.scroll, omsi_ui::tr("newer below")), (11.0 * s) as u32, [200, 200, 200, 200]);
                    scene.overlays.push((m.tex, [x0 + width - m.w as f32, by - m.h as f32, x0 + width, by]));
                }
            }
            if let Some(e) = c.error {
                let l = self.text.label(r, scene, &format!("{}: {}", omsi_ui::tr("Not sent"), omsi_ui::tr(e)), (12.0 * s) as u32, [255, 150, 150, 220]);
                let ey = y0 + box_h;
                scene.overlays.push((l.tex, [x0, ey, x0 + l.w as f32, ey + l.h as f32]));
            }
        } else {
            self.chat.hovered = false;
            self.chat.rect = [0.0; 4];
        }
        // --- the server's notifications: cards over the navigator (under it when it is in a
        // top corner; at the top middle without it), the newest nearest to it
        if !f.notices.is_empty() {
            let pad = 10.0 * s;
            let stripe = 4.0 * s;
            let gap = 6.0 * s;
            let (x0, w, mut y, up) = match f.notice_anchor {
                Some(a) => {
                    let w = (a[2] - a[0]).max(300.0 * s).min(f.width - 16.0 * s);
                    let x0 = if a[0] + w > f.width { f.width - w - 8.0 * s } else { a[0] }.max(8.0 * s);
                    if a[1] > f.height * 0.5 { (x0, w, a[1] - gap, true) } else { (x0, w, a[3] + gap, false) }
                }
                None => {
                    let w = (420.0 * s).min(f.width - 32.0 * s);
                    ((f.width - w) * 0.5, w, 70.0 * s, false)
                }
            };
            let title_px = (13.0 * s) as u32;
            let px = (16.0 * s) as u32;
            let lh = px as f32 * 1.3;
            for n in f.notices.iter().rev() {
                // (the text steps in quarters as it fades: a label is made for each colour)
                let a = (n.alpha() * 4.0).ceil() / 4.0;
                if a <= 0.0 {
                    continue;
                }
                let (accent, title) = match n.kind {
                    NoticeKind::Info => ([90, 160, 255], "Server"),
                    NoticeKind::Warn => ([240, 170, 50], "Warning"),
                    NoticeKind::Alert => ([235, 80, 70], "Alert"),
                    NoticeKind::Update => ([46, 160, 67], "Update"),
                };
                let rows = {
                    let tc = &self.text;
                    wrap_rows(&n.text, w - stripe - pad * 2.0, 0.0, &|t: &str| tc.width_raw(t, px as f32))
                };
                // (five rows at the most; a longer text says it goes on)
                let mut rows: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
                if rows.len() > 5 {
                    rows.truncate(5);
                    rows[4].push('…');
                }
                let th = title_px as f32 * 1.35;
                let h = pad + th + lh * rows.len() as f32 + pad * 0.7;
                let top = if up { y - h } else { y };
                // (a stack that would leave the window ends there)
                if top < 0.0 || top + h > f.height {
                    break;
                }
                let bg = self.text.solid(r, scene, [14, 16, 20, (225.0 * a * self.text.backdrop) as u8]);
                scene.overlays.push((bg, [x0, top, x0 + w, top + h]));
                let bar = self.text.solid(r, scene, [accent[0], accent[1], accent[2], (255.0 * a) as u8]);
                scene.overlays.push((bar, [x0, top, x0 + stripe, top + h]));
                // what is left of its time: a thin line along the bottom
                let left = (n.left / n.total).clamp(0.0, 1.0);
                let line = self.text.solid(r, scene, [accent[0], accent[1], accent[2], (150.0 * a) as u8]);
                scene.overlays.push((line, [x0 + stripe, top + h - 2.0 * s, x0 + stripe + (w - stripe) * left, top + h]));
                let tx = x0 + stripe + pad;
                let t = self.text.label(r, scene, title, title_px, [accent[0], accent[1], accent[2], (255.0 * a) as u8]);
                scene.overlays.push((t.tex, [tx, top + pad * 0.6, tx + t.w as f32, top + pad * 0.6 + t.h as f32]));
                let mut ry = top + pad * 0.6 + th;
                for row in &rows {
                    let l = self.text.label(r, scene, row, px, [255, 255, 255, (240.0 * a) as u8]);
                    scene.overlays.push((l.tex, [tx, ry, tx + l.w as f32, ry + l.h as f32]));
                    ry += lh;
                }
                y = if up { top - gap } else { top + h + gap };
            }
        }
        if let Some(fps) = f.fps {
            let l = self.text.label(r, scene, &format!("{fps:.0} fps"), (13.0 * s) as u32, [255, 255, 255, 200]);
            let x = f.width - l.w as f32 - 12.0 * s;
            // (on a plate, as the notes are: over a bright sky the outline alone was not enough)
            let plate = self.text.plate(r, scene, 7);
            scene.overlays.push((plate, [x - 5.0 * s, 10.0 * s, x + l.w as f32 + 5.0 * s, 10.0 * s + l.h as f32]));
            scene.overlays.push((l.tex, [x, 10.0 * s, x + l.w as f32, 10.0 * s + l.h as f32]));
        }
        // --- the information bar, along the top in the middle: in as many rows as the room
        // it has needs
        self.info_rect = None;
        // (not under the pause menu's rail, from under which it stood out)
        let rail = f.menu.is_some() && f.menu_kind == MenuKind::Game && f.menu_head.is_none() && !f.vr;
        if let Some(info) = f.info.as_ref().filter(|_| !rail) {
            let px = (15.0 * s) as u32;
            let pad = 10.0 * s;
            let [x0, x1, y] = f.info_room.unwrap_or([12.0 * s, f.width - 12.0 * s, 8.0 * s]);
            let rows = info_rows(info, (x1 - x0 - pad * 2.0).max(1.0), |t| self.text.width(t, px as f32));
            let labels: Vec<Label> = rows.iter().map(|t| self.text.label(r, scene, t, px, [255, 255, 255, 0])).collect();
            let lw = labels.iter().map(|l| l.w).max().unwrap_or(0) as f32;
            let lh = labels.iter().map(|l| l.h).max().unwrap_or(0) as f32;
            let (w, h) = (lw + pad * 2.0, lh * labels.len() as f32 + pad * 0.8);
            let x = (x0 + x1 - w) * 0.5;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, y, x + w, y + h]));
            for (k, l) in labels.iter().enumerate() {
                let (lx, ly) = (x + (w - l.w as f32) * 0.5, y + pad * 0.4 + k as f32 * lh);
                scene.overlays.push((l.tex, [lx, ly, lx + l.w as f32, ly + l.h as f32]));
            }
            self.info_rect = Some([x, y, x + w, y + h]);
        }
        let tutorial_w = (420.0 * s).min(f.width * 0.42);
        // --- the timetable window, on the right
        if let Some((title, rows)) = f.timetable.as_ref() {
            let px = (14.0 * s) as u32;
            let lh = px as f32 * 1.55;
            let w = (340.0 * s).min(f.width * 0.4);
            let shown = rows.len().min(((f.height * 0.7) / lh) as usize).max(1);
            // (from the stop before the next one on)
            let next = rows.iter().position(|r| r.2 == 1).unwrap_or(0);
            let first = next.saturating_sub(1).min(rows.len().saturating_sub(shown));
            let h = lh * (shown as f32 + 1.6);
            // (left of a tutorial page, which has the same corner)
            let beside = if f.tutorial.is_some() { tutorial_w + 12.0 * s } else { 0.0 };
            let x = (f.width - w - 16.0 * s - beside).max(16.0 * s);
            let y = corner_top;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, y, x + w, y + h]));
            let t = self.text.label(r, scene, &clip_to(&self.text, title, px as f32 * 1.1, w - 20.0 * s), (px as f32 * 1.1) as u32, [255, 255, 255, 0]);
            scene.overlays.push((t.tex, [x + 10.0 * s, y + 6.0 * s, x + 10.0 * s + t.w as f32, y + 6.0 * s + t.h as f32]));
            // (the names start after the widest time: "12:03-05" of a stop with a wait)
            let time_w = rows.iter().map(|r| self.text.width(&r.1, px as f32)).fold(0.0f32, f32::max).max(40.0 * s);
            let name_x = x + 10.0 * s + time_w + 12.0 * s;
            for (k, (name, time, state)) in rows.iter().skip(first).take(shown).enumerate() {
                let ry = y + lh * (k as f32 + 1.3);
                if *state == 1 {
                    let hl = self.text.plate(r, scene, 5);
                    scene.overlays.push((hl, [x + 4.0 * s, ry - 2.0 * s, x + w - 4.0 * s, ry + lh - 4.0 * s]));
                }
                let color = match state {
                    0 => [140, 140, 140, 0],
                    1 => [255, 200, 110, 0],
                    _ => [235, 235, 235, 0],
                };
                let tl = self.text.label(r, scene, time, px, color);
                scene.overlays.push((tl.tex, [x + 10.0 * s, ry, x + 10.0 * s + tl.w as f32, ry + tl.h as f32]));
                let nl = self.text.label(r, scene, &clip_to(&self.text, name, px as f32, x + w - name_x - 10.0 * s), px, color);
                scene.overlays.push((nl.tex, [name_x, ry, name_x + nl.w as f32, ry + nl.h as f32]));
            }
        }
        // --- a tutorial page, on the right
        if let Some((title, text, image, at, count)) = f.tutorial {
            let w = tutorial_w;
            let x = f.width - w - 16.0 * s;
            let mut y = corner_top;
            let top = y;
            let pad = 14.0 * s;
            let mut items: Vec<(TextureId, [f32; 4])> = Vec::new();
            if let Some(p) = image {
                let entry = self.images.entry(p.to_path_buf()).or_insert_with(|| {
                    omsi_texture::decode_file(p).ok().map(|img| {
                        let (iw, ih) = (img.width, img.height);
                        (r.add_texture(scene, &img, false), iw, ih)
                    })
                });
                if let Some((tex, iw, ih)) = *entry {
                    let dw = w - pad * 2.0;
                    let dh = dw * ih as f32 / iw.max(1) as f32;
                    items.push((tex, [x + pad, y + pad, x + pad + dw, y + pad + dh]));
                    y += dh + pad;
                }
            }
            y += pad * 0.6;
            let tp = (18.0 * s) as u32;
            if !title.is_empty() {
                for line in wrap(&self.text, title, tp as f32, w - pad * 2.0) {
                    let l = self.text.label(r, scene, &line, tp, [255, 200, 110, 0]);
                    items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
                    y += l.h as f32;
                }
                y += 6.0 * s;
            }
            let bp = (14.0 * s) as u32;
            let max_y = f.height - 60.0 * s;
            'text: for para in text.lines() {
                for line in wrap(&self.text, para, bp as f32, w - pad * 2.0) {
                    if y > max_y {
                        break 'text;
                    }
                    let l = self.text.label(r, scene, &line, bp, [235, 235, 235, 0]);
                    items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
                    y += l.h as f32 * 0.95;
                }
                y += 5.0 * s;
            }
            let tr = |t: &str| omsi_ui::tr(t).into_owned();
            let foot = format!("{} {}/{}   ·   {}   ·   {}   ·   {}", tr("Page"), at + 1, count, tr("Enter next"), tr("Page Up back"), tr("Ctrl+T hide"));
            let l = self.text.label(r, scene, &foot, (12.0 * s) as u32, [150, 150, 150, 0]);
            y += 4.0 * s;
            items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
            y += l.h as f32 + pad;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, top, x + w, y]));
            scene.overlays.extend(items);
        }
        // --- the game menu and its lists, in the middle over a dimmed picture
        self.anim_dt = dt.clamp(0.0, 0.1);
        self.text.flat = true;
        self.draw_menu(r, scene, f);
        self.text.flat = false;
        // --- the mouse-over name, right of the cursor
        self.vr_tooltip_overlay = None;
        if let Some(t) = f.tooltip.as_ref().filter(|t| !t.is_empty()) {
            let l = self.text.label(r, scene, t, (14.0 * s) as u32, [255, 255, 255, 235]);
            let mut x = f.cursor.0.clamp(0.0, f.width) + 16.0 * s;
            let mut y = f.cursor.1 + 2.0 * s;
            if x + l.w as f32 > f.width {
                x = (f.cursor.0.clamp(0.0, f.width) - 8.0 * s - l.w as f32).max(5.0 * s);
            }
            if y + l.h as f32 > f.height {
                y = f.height - l.h as f32;
            }
            if f.vr {
                self.vr_tooltip_overlay = Some(scene.overlays.len());
            } else {
                // (in a headset the text alone is placed in front of the eye)
                let plate = self.text.plate(r, scene, 7);
                scene.overlays.push((plate, [x - 5.0 * s, y, x + l.w as f32 + 5.0 * s, y + l.h as f32]));
            }
            scene.overlays.push((l.tex, [x, y, x + l.w as f32, y + l.h as f32]));
        }
        if let (Some((x, y)), false) = (f.steer_cross, f.vr) {
            // (the size of the system's crosshair cursor: 17 logical pixels)
            let tex = self.text.steer_cross(r, scene);
            let half = 8.5 * f.scale;
            scene.overlays.push((tex, [x - half, y - half, x + half, y + half]));
        }
        if f.vr {
            let pointer = self.text.vr_pointer(r, scene);
            self.vr_cursor_overlay = Some(scene.overlays.len());
            scene.overlays.push((pointer, [0.0, 0.0, 7.0 * s, 7.0 * s]));
        } else {
            self.vr_cursor_overlay = None;
        }
        self.text.end_frame(r, scene);
    }
}

/// The pixels (RGBA, `n`×`n`) of the steering cross: arms two pixels wide across the whole
/// picture, white, with a one-pixel dark edge so it shows on a bright sky and a dark road
/// alike, and a small gap in the middle that leaves the point itself visible.
fn steer_cross_pixels(n: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; n * n * 4];
    let c = n as f32 * 0.5;
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = ((x as f32 + 0.5 - c).abs(), (y as f32 + 0.5 - c).abs());
            let (along, across) = if dx <= dy { (dy, dx) } else { (dx, dy) };
            let gap = along < 3.0;
            let px = if gap || along > c - 1.0 {
                [0, 0, 0, 0]
            } else if across <= 1.0 {
                [255, 255, 255, 255]
            } else if across <= 2.0 {
                [0, 0, 0, 200]
            } else {
                [0, 0, 0, 0]
            };
            rgba[(y * n + x) * 4..(y * n + x + 1) * 4].copy_from_slice(&px);
        }
    }
    rgba
}

impl TextCache {
    /// The mouse steering's cross: thin white arms with a dark edge, as a crosshair cursor,
    /// centred on the point that steers and receives a click.
    fn steer_cross(&mut self, r: &Renderer, scene: &mut Scene) -> TextureId {
        let key = ("\u{0}steer_cross".to_string(), 0, [0, 0, 0, 0]);
        if let Some(label) = self.labels.get_mut(&key) {
            label.used = self.frame;
            return label.tex;
        }
        const N: usize = 34;
        let rgba = steer_cross_pixels(N);
        let image = omsi_texture::Image { width: N as u32, height: N as u32, rgba, has_alpha: true };
        let tex = r.add_texture(scene, &image, false);
        self.labels.insert(key, Label { tex, w: N as u32, h: N as u32, used: self.frame });
        tex
    }

    /// A small white circle, centred on the point that receives the click.
    fn vr_pointer(&mut self, r: &Renderer, scene: &mut Scene) -> TextureId {
        let key = ("\u{0}vr_pointer_dot".to_string(), 0, [0, 0, 0, 0]);
        if let Some(label) = self.labels.get_mut(&key) {
            label.used = self.frame;
            return label.tex;
        }
        const W: usize = 32;
        const H: usize = 32;
        let mut rgba = vec![0u8; W * H * 4];
        for y in 0..H {
            for x in 0..W {
                let dx = x as f32 + 0.5 - W as f32 * 0.5;
                let dy = y as f32 + 0.5 - H as f32 * 0.5;
                let radius = (dx * dx + dy * dy).sqrt();
                let alpha = (16.0 - radius).clamp(0.0, 1.0);
                let white = radius < 13.0;
                let color = if white {
                    [255, 255, 255, (alpha * 255.0) as u8]
                } else {
                    [0, 0, 0, (alpha * 220.0) as u8]
                };
                rgba[(y * W + x) * 4..(y * W + x + 1) * 4].copy_from_slice(&color);
            }
        }
        let image = omsi_texture::Image { width: W as u32, height: H as u32, rgba, has_alpha: true };
        let tex = r.add_texture(scene, &image, false);
        self.labels.insert(key, Label { tex, w: W as u32, h: H as u32, used: self.frame });
        tex
    }

    /// A plate of one colour: 0 the chat's dark translucent input box, 1 the loading
    /// screen's bar track, 2 its fill, 3 .. 7 as said below.
    fn plate(&mut self, r: &Renderer, scene: &mut Scene, kind: u8) -> TextureId {
        let mut rgba = match kind {
            1 => vec![255, 255, 255, 38],
            2 => vec![235, 238, 242, 255],
            // an opaque panel as designed (translucent dark panels show the sky through
            // them), unless the opacity setting asks for less
            3 => vec![22, 22, 22, 255],
            // the chosen line: the interface's accent
            4 => vec![232, 160, 48, 255],
            // a line the mouse is over, the next stop
            5 => vec![255, 255, 255, 28],
            // a hairline between groups (the card's border colour)
            9 => vec![255, 255, 255, 15],
            // the picture dimmed behind the menu
            6 => vec![0, 0, 0, 120],
            // behind a note or the tooltip (the old HUD's 40 %)
            7 => vec![0, 0, 0, 102],
            _ => vec![10, 12, 16, 150],
        };
        // the backgrounds - the panels, the chat's box, the plates - as the opacity setting
        // has them (`backdrop`); the accent, the lines and the dimming stay as they are
        if matches!(kind, 0 | 3 | 7) {
            rgba[3] = (rgba[3] as f32 * self.backdrop).round().clamp(0.0, 255.0) as u8;
        }
        let key = ("\u{0}plate".to_string(), kind as u32, [rgba[0], rgba[1], rgba[2], rgba[3]]);
        if let Some(l) = self.labels.get_mut(&key) {
            l.used = self.frame;
            return l.tex;
        }
        let img = omsi_texture::Image { width: 1, height: 1, rgba, has_alpha: true };
        let tex = r.add_texture(scene, &img, false);
        self.labels.insert(key, Label { tex, w: 1, h: 1, used: self.frame });
        tex
    }

    /// A plate of one RGBA colour, any size (stretched).
    fn solid(&mut self, r: &Renderer, scene: &mut Scene, rgba: [u8; 4]) -> TextureId {
        let key = ("\u{0}solid".to_string(), 0, rgba);
        if let Some(l) = self.labels.get_mut(&key) {
            l.used = self.frame;
            return l.tex;
        }
        let img = omsi_texture::Image { width: 1, height: 1, rgba: rgba.to_vec(), has_alpha: true };
        let tex = r.add_texture(scene, &img, false);
        self.labels.insert(key, Label { tex, w: 1, h: 1, used: u64::MAX / 2 });
        tex
    }

    /// One anti-aliased quarter disc of `rad` pixels: the rounded corner `idx` (0 top left,
    /// 1 top right, 2 bottom left, 3 bottom right) of a rounded plate.
    fn corner(&mut self, r: &Renderer, scene: &mut Scene, rad: u32, idx: u32, rgba: [u8; 4]) -> TextureId {
        let key = ("\u{0}corner".to_string(), (rad << 8) | idx, rgba);
        if let Some(l) = self.labels.get_mut(&key) {
            l.used = self.frame;
            return l.tex;
        }
        let n = rad as usize;
        let (cx, cy) = (if idx & 1 == 0 { rad as f32 } else { 0.0 }, if idx & 2 == 0 { rad as f32 } else { 0.0 });
        let mut data = vec![0u8; n * n * 4];
        for py in 0..n {
            for px in 0..n {
                let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
                let cover = (rad as f32 - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                let o = (py * n + px) * 4;
                data[o..o + 3].copy_from_slice(&rgba[..3]);
                data[o + 3] = (rgba[3] as f32 * cover).round() as u8;
            }
        }
        let img = omsi_texture::Image { width: rad, height: rad, rgba: data, has_alpha: true };
        let tex = r.add_texture(scene, &img, false);
        self.labels.insert(key, Label { tex, w: rad, h: rad, used: u64::MAX / 2 });
        tex
    }

    /// A texture made once for `key` (kept while it is used, like a text).
    fn cached(&mut self, r: &Renderer, scene: &mut Scene, key: (String, u32, [u8; 4]), size: (u32, u32), build: impl FnOnce() -> Vec<u8>) -> TextureId {
        if let Some(l) = self.labels.get_mut(&key) {
            l.used = self.frame;
            return l.tex;
        }
        let img = omsi_texture::Image { width: size.0, height: size.1, rgba: build(), has_alpha: true };
        let tex = r.add_texture(scene, &img, false);
        self.labels.insert(key, Label { tex, w: size.0, h: size.1, used: self.frame });
        tex
    }

    /// A Material icon (`omsi_ui::icons`, as the launcher draws them) of `px` pixels in
    /// `rgba`; `None` for a name there is no icon of.
    fn icon(&mut self, r: &Renderer, scene: &mut Scene, name: &str, px: u32, rgba: [u8; 4]) -> Option<TextureId> {
        omsi_ui::icons::svg(name)?;
        let px = px.max(8);
        let key = (format!("\u{0}icon:{name}"), px, rgba);
        Some(self.cached(r, scene, key, (px, px), || {
            let mask = omsi_ui::icons::rasterize(name, px).unwrap_or_else(|| vec![0; (px * px) as usize]);
            mask.iter().flat_map(|&a| [rgba[0], rgba[1], rgba[2], (a as u32 * rgba[3] as u32 / 255) as u8]).collect()
        }))
    }

    /// A whole rounded plate of `w` x `h` pixels in one texture, anti-aliased.
    fn rrect(&mut self, r: &Renderer, scene: &mut Scene, w: u32, h: u32, rad: f32, rgba: [u8; 4]) -> TextureId {
        let key = (format!("\u{0}rr{w}x{h}r{}", rad as u32), 0, rgba);
        self.cached(r, scene, key, (w, h), || {
            let mut data = vec![0u8; (w * h * 4) as usize];
            let ri = rad as u32;
            for py in 0..h {
                let band_y = py >= ri && py + ri < h;
                for px in 0..w {
                    let cover = if band_y || (px >= ri && px + ri < w) {
                        1.0
                    } else {
                        let d = rr_dist(px as f32 + 0.5, py as f32 + 0.5, 0.0, 0.0, w as f32, h as f32, rad);
                        (0.5 - d).clamp(0.0, 1.0)
                    };
                    let o = ((py * w + px) * 4) as usize;
                    data[o..o + 3].copy_from_slice(&rgba[..3]);
                    data[o + 3] = (rgba[3] as f32 * cover).round() as u8;
                }
            }
            data
        })
    }

    /// A soft shadow under the rounded rectangle `rect`: it fades out over `spread` pixels
    /// and lies `dy` lower.
    fn shadow(&mut self, r: &Renderer, scene: &mut Scene, rect: [f32; 4], radius: f32, spread: f32, dy: f32, alpha: u8) {
        let [x0, y0, x1, y1] = [rect[0].round(), rect[1].round(), rect[2].round(), rect[3].round()];
        let (w, h) = ((x1 - x0).max(0.0) as u32, (y1 - y0).max(0.0) as u32);
        let m = spread.round().max(1.0) as u32;
        let (tw, th) = (w + 2 * m, h + 2 * m);
        if w < 2 || h < 2 || tw as u64 * th as u64 > 4_000_000 {
            return;
        }
        let rad = radius.round().min((w as f32 * 0.5).floor()).min((h as f32 * 0.5).floor()).max(0.0);
        let key = (format!("\u{0}shadow{w}x{h}r{}m{m}", rad as u32), 0, [0, 0, 0, alpha]);
        let tex = self.cached(r, scene, key, (tw, th), || {
            let mut data = vec![0u8; (tw * th * 4) as usize];
            for py in 0..th {
                for px in 0..tw {
                    let d = rr_dist(px as f32 + 0.5, py as f32 + 0.5, m as f32, m as f32, w as f32, h as f32, rad);
                    let k = (1.0 - d / m as f32).clamp(0.0, 1.0);
                    data[((py * tw + px) * 4 + 3) as usize] = (alpha as f32 * k * k).round() as u8;
                }
            }
            data
        });
        scene.overlays.push((tex, [x0 - m as f32, y0 - m as f32 + dy, x1 + m as f32, y1 + m as f32 + dy]));
    }

    /// A rectangle with rounded corners of `radius` pixels in `rgba`, on whole pixels so
    /// that translucent colours show no seams: one texture for a plate of some size, else
    /// (a thin bar, a thumb that grows and shrinks) three bands and four corner discs.
    fn rounded(&mut self, r: &Renderer, scene: &mut Scene, rect: [f32; 4], radius: f32, rgba: [u8; 4]) {
        let [x0, y0, x1, y1] = [rect[0].round(), rect[1].round(), rect[2].round(), rect[3].round()];
        let (w, h) = (x1 - x0, y1 - y0);
        if w < 1.0 || h < 1.0 {
            return;
        }
        let rad = radius.round().min((w * 0.5).floor()).min((h * 0.5).floor()).max(0.0);
        if rad < 1.0 {
            let solid = self.solid(r, scene, rgba);
            scene.overlays.push((solid, [x0, y0, x1, y1]));
            return;
        }
        if w.min(h) >= 12.0 && w * h <= 4_000_000.0 {
            let tex = self.rrect(r, scene, w as u32, h as u32, rad, rgba);
            scene.overlays.push((tex, [x0, y0, x1, y1]));
            return;
        }
        let solid = self.solid(r, scene, rgba);
        let ri = rad as u32;
        // the middle band over the whole height, the side bands between the corners
        if x1 - rad > x0 + rad {
            scene.overlays.push((solid, [x0 + rad, y0, x1 - rad, y1]));
        }
        if h > rad * 2.0 {
            scene.overlays.push((solid, [x0, y0 + rad, x0 + rad, y1 - rad]));
            scene.overlays.push((solid, [x1 - rad, y0 + rad, x1, y1 - rad]));
        }
        let tl = self.corner(r, scene, ri, 0, rgba);
        let tr = self.corner(r, scene, ri, 1, rgba);
        let bl = self.corner(r, scene, ri, 2, rgba);
        let br = self.corner(r, scene, ri, 3, rgba);
        scene.overlays.push((tl, [x0, y0, x0 + rad, y0 + rad]));
        scene.overlays.push((tr, [x1 - rad, y0, x1, y0 + rad]));
        scene.overlays.push((bl, [x0, y1 - rad, x0 + rad, y1]));
        scene.overlays.push((br, [x1 - rad, y1 - rad, x1, y1]));
    }
}

impl Ui {
    /// The loading screen over a plain dark picture: the map's name in the middle, a thin
    /// bar of how far the start area got under it, and one quiet line (what is being done).
    pub fn loading(&mut self, r: &Renderer, scene: &mut Scene, width: f32, height: f32, scale: f32, title: &str, caption: &str, progress: f32) {
        let s = scale.max(0.5);
        let t = self.text.label(r, scene, title, (30.0 * s) as u32, [255, 255, 255, 0]);
        let cy = height * 0.5;
        let tx = (width - t.w as f32) * 0.5;
        let ty = cy - t.h as f32 - 14.0 * s;
        scene.overlays.push((t.tex, [tx, ty, tx + t.w as f32, ty + t.h as f32]));
        let bw = (320.0 * s).min(width * 0.6);
        let bh = (3.0 * s).max(2.0);
        let bx = (width - bw) * 0.5;
        let by = cy + 4.0 * s;
        let track = self.text.plate(r, scene, 1);
        scene.overlays.push((track, [bx, by, bx + bw, by + bh]));
        let fill = self.text.plate(r, scene, 2);
        let p = progress.clamp(0.0, 1.0);
        if p > 0.0 {
            scene.overlays.push((fill, [bx, by, bx + bw * p, by + bh]));
        }
        if !caption.is_empty() {
            let c = self.text.label(r, scene, caption, (13.0 * s) as u32, [200, 204, 210, 0]);
            let cx = (width - c.w as f32) * 0.5;
            let cy2 = by + bh + 14.0 * s;
            scene.overlays.push((c.tex, [cx, cy2, cx + c.w as f32, cy2 + c.h as f32]));
        }
        self.text.end_frame(r, scene);
    }
}

// ---------------------------------------------------------------------------------------
// the game menu and its lists: flat near-black greys and one amber accent, as the launcher

/// How long a fade of the menu takes (a line lit, a switch turned over): a short moment.
const FADE_SECS: f32 = 0.15;
const BAR_SECS: f32 = 0.19;

// (the launcher's values, see `launcher/theme.rs`: neutral greys, no tint)
const PANEL: [u8; 4] = [22, 22, 22, 255];
const PANEL_ALT: [u8; 4] = [31, 31, 31, 255];
const BORDER: [u8; 4] = [255, 255, 255, 15];
const ACCENT: [u8; 4] = [232, 160, 48, 255];
const ACCENT_SOFT: [u8; 4] = [232, 160, 48, 34];
const DANGER: [u8; 4] = [222, 78, 68, 255];
/// A line under the mouse, and the line chosen: the launcher's `HOVER` and `SELECTED`, as
/// solid greys. (They were white at 6 % and 9 %, which the overlays blend in linear light:
/// on the 22-grey card that came out 73 and 92, twice as light as the launcher's 38 and 44.)
const LIT: [u8; 4] = [38, 38, 38, 255];
const SELECTED: [u8; 4] = [44, 44, 44, 255];
/// "End the session" under the mouse: the danger colour at 14 % on the card, solid.
const LIT_DANGER: [u8; 4] = [50, 30, 28, 255];
/// A stepper's or a button's field: the launcher's `FIELD`.
const CHIP: [u8; 4] = [31, 31, 31, 255];
/// A switch's track when off, a slider's empty track, their knob (the launcher's).
const TRACK_OFF: [u8; 4] = [62, 62, 62, 255];
const SLIDER_TRACK: [u8; 4] = [58, 58, 58, 255];
const KNOB: [u8; 4] = [240, 240, 240, 255];
/// A scroll bar's thumb, idle and with the mouse over its list (the launcher's white at
/// 10 % and 35 %; no track, no accent).
const THUMB: [u8; 4] = [255, 255, 255, 26];
const THUMB_HOT: [u8; 4] = [255, 255, 255, 90];
/// The small capitals over a title (the launcher's section headings: 11 px, bold, dim).
const DIM: [u8; 4] = [142, 142, 142, 0];
/// The accent's fill under the mouse, and the ink on it (the launcher's primary button).
const ACCENT_HOT: [u8; 4] = [246, 182, 84, 255];
const ON_ACCENT: [u8; 4] = [18, 14, 8, 0];
/// Text colours (alpha 0: no outline on the flat panel).
const WHITE: [u8; 4] = [236, 236, 236, 0];
const SOFT: [u8; 4] = [200, 200, 200, 0];
const MUTED: [u8; 4] = [142, 142, 142, 0];
const AMBER: [u8; 4] = [255, 200, 110, 0];

/// The ink of a greyed-out line and its small hint.
const OFF_INK: [u8; 4] = [96, 96, 96, 0];
const OFF_HINT: [u8; 4] = [96, 96, 96, 0];

/// The card's radius, a line's, and the inset of lines from the card's edge and of their
/// text from the line's edge (all times the scale).
const CARD_R: f32 = 8.0;
const ROW_R: f32 = 6.0;
const PAD: f32 = 12.0;
const TEXT_IN: f32 = 16.0;

/// `v` (0 to 1) in eighths.
fn quant(v: f32) -> f32 {
    (v.clamp(0.0, 1.0) * 8.0).round() / 8.0
}

/// The colour `t` (0 to 1) of the way from `a` to `b`.
fn mix(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]), m(a[3], b[3])]
}

/// `c` with its opacity times `k` (0 to 1).
fn fade(c: [u8; 4], k: f32) -> [u8; 4] {
    [c[0], c[1], c[2], (c[3] as f32 * k.clamp(0.0, 1.0)).round() as u8]
}

/// `c` as a text colour (no outline).
fn txt(c: [u8; 4]) -> [u8; 4] {
    [c[0], c[1], c[2], 0]
}

/// The game menu's size: the screen's scale times the interface size (`Frame::ui_scale`, the
/// "Interface size" setting and the growth with tall windows, as the rest of the HUD has it),
/// held where the settings window (880 x 600 as designed) would no longer fit the window -
/// but never below the screen's scale alone, the size it always had.
fn menu_scale(f: &Frame) -> f32 {
    menu_scale_for(f.scale, f.ui_scale, f.width, f.height, f.vr)
}

fn menu_scale_for(scale: f32, ui_scale: f32, width: f32, height: f32, vr: bool) -> f32 {
    let base = scale.max(0.5);
    if vr {
        return base;
    }
    let fit = (width / (880.0 + 24.0)).min(height * 0.94 / 600.0);
    (base * ui_scale.max(0.5)).min(fit.max(base))
}

/// A list line `"<code>  <name>"` (a number, maybe padded in front, two spaces, a name): its
/// code and name.
fn code_and_name(label: &str) -> Option<(&str, &str)> {
    let (code, name) = label.trim_start().split_once("  ")?;
    (!code.is_empty() && code.chars().all(|c| c.is_ascii_digit()) && !name.trim().is_empty()).then(|| (code, name.trim()))
}

/// The cursor is in `rect`.
fn over_rect(c: (f32, f32), rect: [f32; 4]) -> bool {
    c.0 >= rect[0] && c.0 <= rect[2] && c.1 >= rect[1] && c.1 <= rect[3]
}

/// `text` clipped to `width` pixels in the bold weight (with "...").
fn clip_bold(tc: &TextCache, text: &str, px: f32, width: f32) -> String {
    let text = &*omsi_ui::tr(text);
    if tc.width_bold(text, px) <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().collect();
    while !out.is_empty() && tc.width_bold(&format!("{out}…"), px) > width {
        out.pop();
    }
    format!("{}…", out.trim_end())
}

/// The distance of the point (`px`, `py`) from the rounded rectangle at (`x0`, `y0`) of
/// `w` x `h` with corners of `rad`: negative inside.
fn rr_dist(px: f32, py: f32, x0: f32, y0: f32, w: f32, h: f32, rad: f32) -> f32 {
    let (cx, cy) = (x0 + w * 0.5, y0 + h * 0.5);
    let qx = (px - cx).abs() - (w * 0.5 - rad);
    let qy = (py - cy).abs() - (h * 0.5 - rad);
    (qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0)).sqrt() + qx.max(qy).min(0.0) - rad
}

/// A line that opens another list ends in dots: the text without them, and whether it did.
fn strip_more(label: &str) -> (&str, bool) {
    match label.strip_suffix("...").or_else(|| label.strip_suffix('…')) {
        Some(t) => (t.trim_end(), true),
        None => (label, false),
    }
}

impl Ui {
    /// `text` at `x`, its middle on `cy`; returns its width.
    fn put(&mut self, r: &Renderer, scene: &mut Scene, text: &str, px: u32, color: [u8; 4], x: f32, cy: f32) -> f32 {
        let l = self.text.label(r, scene, text, px, color);
        let y = cy - l.h as f32 * 0.5;
        scene.overlays.push((l.tex, [x, y, x + l.w as f32, y + l.h as f32]));
        l.w as f32
    }

    /// The value that eases to `target` (a part of the menu changing its colour or place):
    /// each frame it goes `speed` per second of the way (exponentially). One value per `key`;
    /// the first time it is asked for it is there already.
    fn ease(&mut self, key: (u8, &str, usize), target: f32, speed: f32) -> f32 {
        // (`speed`: the share of the whole way it goes in a second: it takes 1 / speed seconds)
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        let dt = self.anim_dt;
        let v = self.anim.entry(h.finish()).or_insert(target);
        let step = speed * dt;
        if (target - *v).abs() <= step {
            *v = target;
        } else if target > *v {
            *v += step;
        } else {
            *v -= step;
        }
        *v
    }

    /// `ease`, in eighths: a colour or a light that fades is a texture made once per step
    /// (a plate or a text of that colour), so a fade goes in a few steps, not in hundreds.
    fn easeq(&mut self, key: (u8, &str, usize), target: f32, speed: f32) -> f32 {
        quant(self.ease(key, target, speed))
    }

    /// `text` ending at `right`, its middle on `cy`; returns its width.
    fn put_right(&mut self, r: &Renderer, scene: &mut Scene, text: &str, px: u32, color: [u8; 4], right: f32, cy: f32) -> f32 {
        let l = self.text.label(r, scene, text, px, color);
        let y = cy - l.h as f32 * 0.5;
        scene.overlays.push((l.tex, [right - l.w as f32, y, right, y + l.h as f32]));
        l.w as f32
    }

    /// A scroll bar's thumb in `track` for `shown` of `n` lines from `first`, as the
    /// launcher's: no track, a thin white bar that lightens while the mouse is over its list
    /// (`hot`), at least 28 px long. Returns the rect drawn.
    #[allow(clippy::too_many_arguments)]
    fn thumb(&mut self, r: &Renderer, scene: &mut Scene, track: [f32; 4], first: usize, shown: usize, n: usize, hot: bool, s: f32) -> [f32; 4] {
        let th = track[3] - track[1];
        let n = n.max(1) as f32;
        let len = (th * shown as f32 / n).max((28.0 * s).min(th));
        let t0 = track[1] + (th - len) * (first as f32 / (n - shown as f32).max(1.0)).clamp(0.0, 1.0);
        let thumb = [track[0], t0, track[2], t0 + len];
        let a = self.easeq((16, "thumb", track[0] as usize), if hot { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
        self.text.rounded(r, scene, thumb, 2.0 * s, mix(THUMB, THUMB_HOT, a));
        thumb
    }

    /// The accent bar at the left edge of a line, growing from its middle as `k` goes 0 to 1.
    fn accent_bar(&mut self, r: &Renderer, scene: &mut Scene, rect: [f32; 4], k: f32, danger: bool, s: f32) {
        let full = (rect[3] - rect[1] - 20.0 * s).max(10.0 * s);
        let bh = full * (0.4 + 0.6 * k);
        let by = (rect[1] + rect[3]) * 0.5 - bh * 0.5;
        self.text.rounded(r, scene, [rect[0], by, rect[0] + 2.0 * s, by + bh], 1.0 * s, fade(if danger { DANGER } else { ACCENT }, k));
    }

    /// The header of the card at (`x`, `y`) of `w` wide: what the list is of, small and in
    /// capitals, the title large under it, a hairline under both. Its text starts where the
    /// text of the lines does. Returns the middle of the header.
    fn menu_header(&mut self, r: &Renderer, scene: &mut Scene, x: f32, y: f32, w: f32, header_h: f32, title: &str, sub: &str, s: f32) -> f32 {
        let left = x + (PAD + TEXT_IN) * s;
        // (the game's name in the accent; the list names above are in capitals)
        // (as the launcher's section headings: small bold capitals, dim; the title bold)
        let eyebrow = if sub.is_empty() { "OPENOMSI".to_string() } else { sub.to_uppercase() };
        let room = w - (PAD + TEXT_IN) * 2.0 * s;
        let eyebrow = clip_bold(&self.text, &eyebrow, 11.0 * s, room);
        let e = self.text.label(r, scene, &eyebrow, (11.0 * s) as u32 | BOLD, DIM);
        let title = clip_bold(&self.text, title, 22.0 * s, room);
        let t = self.text.label(r, scene, &title, (22.0 * s) as u32 | BOLD, WHITE);
        let band = header_h - 6.0 * s;
        let top = y + (band - (e.h as f32 + t.h as f32 - 2.0 * s)) * 0.5;
        scene.overlays.push((e.tex, [left, top, left + e.w as f32, top + e.h as f32]));
        let ty = top + e.h as f32 - 2.0 * s;
        scene.overlays.push((t.tex, [left, ty, left + t.w as f32, ty + t.h as f32]));
        y + band * 0.5
    }

    /// A pill (`cap`: a key cap) of `fill` with `text` in it, ending at `right`; returns
    /// its left edge.
    fn chip(&mut self, r: &Renderer, scene: &mut Scene, text: &str, px: u32, color: [u8; 4], fill: [u8; 4], cap: bool, right: f32, cy: f32, s: f32) -> f32 {
        let l = self.text.label(r, scene, text, px, color);
        let (cw, ch) = (l.w as f32 + 18.0 * s, l.h as f32 + 6.0 * s);
        let x0 = right - cw;
        if cap {
            // (a key cap: a fine light edge round it)
            self.text.rounded(r, scene, [x0 - 1.0, cy - ch * 0.5 - 1.0, right + 1.0, cy + ch * 0.5 + 1.0], 5.0 * s, [96, 96, 96, 255]);
        }
        self.text.rounded(r, scene, [x0, cy - ch * 0.5, right, cy + ch * 0.5], if cap { 4.0 * s } else { ROW_R * s }, fill);
        let (tx, ty) = (x0 + 9.0 * s, cy - l.h as f32 * 0.5);
        scene.overlays.push((l.tex, [tx, ty, tx + l.w as f32, ty + l.h as f32]));
        x0
    }

    /// The game menu and its lists (options, lines, tours ...): a card in the middle of a
    /// dimmed picture. Options are settings lines with switches and values; lines and tours
    /// are bigger lines with the timetable of the chosen one beside them.
    fn draw_menu(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame) {
        self.menu_rects.clear();
        self.menu_ctl.clear();
        self.menu_side.clear();
        self.menu_pane.clear();
        self.menu_pane_start = 0;
        self.menu_pane_go = None;
        self.menu_pane_box = None;
        self.menu_pane_scroll = None;
        self.menu_time.clear();
        self.menu_scroll_thumb = None;
        self.menu_scroll_track = None;
        self.dd_rects.clear();
        self.dd_scroll = None;
        let overlay_start = scene.overlays.len();
        let Some((sel, items)) = f.menu else {
            self.menu_overlay_range = overlay_start..overlay_start;
            self.anim.clear();
            return;
        };
        // a settings window has its own layout
        if f.menu_kind == MenuKind::Options && f.menu_tabs.is_some() {
            self.draw_settings(r, scene, f, sel, items);
            self.menu_overlay_range = overlay_start..scene.overlays.len();
            return;
        }
        // the pause menu itself: the launcher's rail (not in a headset, whose menu is a card)
        if f.menu_kind == MenuKind::Game && f.menu_head.is_none() && !f.vr {
            self.draw_game_menu(r, scene, f, sel, items);
            self.menu_overlay_range = overlay_start..scene.overlays.len();
            return;
        }
        let s = menu_scale(f);
        let kind = f.menu_kind;
        // the picture dimmed behind the menu (the first overlay: the headset's menu takes it
        // for the backdrop)
        let dim = self.text.plate(r, scene, 6);
        let sep = self.text.plate(r, scene, 9);
        scene.overlays.push((dim, [0.0, 0.0, f.width, f.height]));
        // key hints only where there is a keyboard
        let keys = !f.vr && !crate::platform::touch_controls();
        // the timetable beside the list, where the window is wide enough
        let preview = f.menu_preview.as_ref().filter(|_| !f.vr && f.width >= 760.0 * s);
        let timetable_kind = matches!(kind, MenuKind::Lines | MenuKind::Tours);
        let pane_w = if preview.is_some() { (if timetable_kind { 320.0 } else { 340.0 }) * s } else { 0.0 };
        let want = if timetable_kind { 360.0 * s } else { 380.0 * s };
        let w = (want + pane_w).min(f.width - 24.0 * s).max(200.0 * s);
        let list_w = w - pane_w;
        let header_h = 72.0 * s;
        let pad = PAD * s;
        let tin = TEXT_IN * s;
        // as many lines as fit at a readable height; a longer menu scrolls (the wheel, the
        // arrow keys), the chosen line kept in view
        // (lines and tours: a card of one fixed height, a share of the screen's)
        // "Back" of lines, tours and the other lists stands alone under the list (not a line of
        // it: no scrolling down to it), and their card keeps one size
        let back_txt = omsi_ui::tr("Back").into_owned();
        let back_footer = (timetable_kind || kind == MenuKind::List) && items.last().is_some_and(|&(id, l)| id == "back" && l == back_txt.as_str());
        let nl = items.len() - back_footer as usize;
        let foot_h = if back_footer { 48.0 * s } else { 0.0 };
        let fixed_h = matches!(kind, MenuKind::Lines | MenuKind::Tours | MenuKind::List).then(|| if f.vr { f.height * 0.60 } else { (520.0 * s).min(f.height * 0.94) });
        let room = match fixed_h {
            Some(fh) => fh - header_h - pad - foot_h,
            None => f.height * (if f.vr { 0.60 } else { 0.92 }) - header_h - pad - 8.0 * s,
        };
        let base = match (f.vr, kind) {
            (true, _) => 40.0,
            (_, MenuKind::Lines) => 44.0,
            (_, MenuKind::Tours) => 40.0,
            _ => 42.0,
        } * s;
        let row_h = base.min(room / nl.max(1) as f32).max(34.0 * s);
        let rows = ((room / row_h).floor() as usize).clamp(1, nl.max(1));
        let sel_l = sel.min(nl.saturating_sub(1));
        let start = match (nl > rows, f.menu_top) {
            (false, _) => 0,
            (true, Some(top)) => (top.max(0.0).round() as usize).min(nl - rows),
            (true, None) => sel_l.saturating_sub(rows / 2).min(nl - rows),
        };
        self.menu_start = start;
        self.menu_rows = rows;
        self.menu_row_h = row_h;
        let px = (((if timetable_kind { 14.0 } else { 16.0 }) * s).min(row_h * 0.45)) as u32;
        let mut h = header_h + row_h * rows as f32 + pad + foot_h;
        if let Some(fh) = fixed_h {
            // (lines and tours: one size, whatever the list holds, like the settings window)
            h = fh;
        } else if preview.is_some() {
            h = h.max((420.0 * s).min(f.height * 0.92));
        }
        let x = ((f.width - w) * 0.5).round();
        let y = ((f.height - h) * 0.5).round();
        let list_r = x + list_w;
        let radius = CARD_R * s;
        // the card: a soft shadow, a 1 px hairline round it, the flat panel
        self.text.shadow(r, scene, [x, y, x + w, y + h], radius, 28.0 * s, 10.0 * s, 110);
        self.text.rounded(r, scene, [x - 1.0, y - 1.0, x + w + 1.0, y + h + 1.0], radius + 1.0, BORDER);
        self.text.rounded(r, scene, [x, y, x + w, y + h], radius, PANEL);
        // the header: what the list is of (or the game's name), and the title
        let (title, sub): (String, String) = match f.menu_head.as_ref() {
            Some((t, u)) => (t.clone(), u.clone()),
            None => {
                let t = if f.paused { "Paused" } else { "Menu" };
                (t.to_string(), String::new())
            }
        };
        self.menu_header(r, scene, x, y, w, header_h, &title, &sub, s);
        // the scroll bar: where the lines shown lie in the whole menu
        let scrolls = nl > rows;
        if scrolls {
            let top = y + header_h;
            let track = [list_r - 12.0 * s, top, list_r - 8.0 * s, top + row_h * rows as f32 - 4.0 * s];
            self.menu_scroll_track = Some(track);
            let hot = over_rect(f.cursor, [x, y, list_r, y + h]);
            let thumb = self.thumb(r, scene, track, start, rows, nl, hot, s);
            // (a wider grip than the drawn thumb: three pixels are hard to hit)
            self.menu_scroll_thumb = Some([thumb[0] - 6.0 * s, thumb[1], thumb[2] + 6.0 * s, thumb[3]]);
        }
        // (the line under the mouse is the one lit; the keyboard's choice only while the
        // mouse is off the lines - both lit at once read as two choices)
        let over = |rect: [f32; 4]| f.cursor.0 >= rect[0] && f.cursor.0 <= rect[2] && f.cursor.1 >= rect[1] && f.cursor.1 <= rect[3];
        // (the whole list: in the gaps between the lines the keyboard's choice, the top line,
        // lit up for a moment as the mouse went down the list)
        let any_hovered = over([x, y, list_r, y + h]);
        let right = list_r - if scrolls { 20.0 * s } else { pad };
        let line_pre = format!("{} ", omsi_ui::tr("Line"));
        let tour_pre = format!("{} ", omsi_ui::tr("Tour"));
        // (all line signs as wide as the widest, so that the texts after them line up)
        let mut sign_w = 48.0 * s;
        if kind == MenuKind::Lines {
            for &(_, label) in items.iter() {
                if let Some(n) = label.strip_prefix(line_pre.as_str()).and_then(|rest| rest.rsplit_once("  (")).map(|(n, _)| n) {
                    sign_w = sign_w.max(self.text.width(n, 15.0 * s) + 22.0 * s);
                }
            }
        }
        // (a list of codes and names - the destinations, `"{code:>3}  {name}"` - in two
        // columns: the codes right-aligned in one as wide as the widest, the names after it.
        // Padded with spaces, the proportional font left them ragged.)
        let mut code_w = 0.0f32;
        if kind == MenuKind::List {
            for &(_, label) in items[..nl].iter() {
                if let Some((c, _)) = code_and_name(label) {
                    code_w = code_w.max(self.text.width(c, px as f32));
                }
            }
        }
        for (k, &(id, label)) in items[..nl].iter().enumerate().skip(start).take(rows) {
            let ry = y + header_h + row_h * (k - start) as f32;
            let gap = 4.0 * s;
            let rect = [x + pad, ry, right, ry + row_h - gap];
            // (a greyed-out line is never lit: not by the mouse, not by the keyboard)
            let off = kind == MenuKind::Game && f.menu_disabled.contains(&id);
            let lit = !off && (over(rect) || (k == sel && f.menu_kbd && !any_hovered));
            let danger = id == "quit";
            let is_back = id == "back" && label == back_txt.as_str();
            // a thin line between the groups: the everyday lines apart from the rarer ones
            // (none in the other lists: "Back" there stands apart under them, without a line)
            let apart = k > start
                && match kind {
                MenuKind::Game => matches!(id, "save" | "admin" | "quit"),
                MenuKind::Lines => id == "free",
                _ => false,
            };
            if apart {
                let sy = (ry - 2.0 * s).round();
                scene.overlays.push((sep, [x + pad + tin, sy, right - tin, sy + 1.0]));
            }
            // (the light of the line eases in and out)
            let glow = self.easeq((7, id, k), if lit { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
            // (the tour chosen stays marked, whatever the mouse is over)
            let active = kind == MenuKind::Tours && k == sel && !is_back;
            let a_act = self.easeq((14, id, k), if active { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
            let bar = self.easeq((8, id, k), if lit || active { 1.0 } else { 0.0 }, 1.0 / BAR_SECS);
            if a_act > 0.0 {
                self.text.rounded(r, scene, rect, ROW_R * s, fade(SELECTED, a_act));
            }
            if glow > 0.0 {
                self.text.rounded(r, scene, rect, ROW_R * s, fade(if danger { LIT_DANGER } else { LIT }, glow));
            }
            if bar > 0.0 {
                // the accent bar at the left edge of the lit line, growing from its middle
                self.accent_bar(r, scene, rect, bar, danger, s);
            }
            let ink = if off {
                OFF_INK
            } else {
                if danger {
                    mix([232, 138, 128, 0], [255, 176, 166, 0], glow)
                } else {
                    mix(SOFT, WHITE, glow.max(a_act))
                }
            };
            let cy = (rect[1] + rect[3]) * 0.5;
            let lx = rect[0] + tin;
            let rx = rect[2] - tin;
            let mut done = false;
            match kind {
                // a line of the timetable: its number on a solid sign, like a line sign on a bus
                MenuKind::Lines => {
                    let parsed = label.strip_prefix(line_pre.as_str()).and_then(|rest| rest.rsplit_once("  (")).map(|(n, t)| (n, t.trim_end_matches(')')));
                    if let Some((name, info)) = parsed {
                        let bl = self.text.label(r, scene, name, (15.0 * s) as u32, ON_ACCENT);
                        let (bw, bh) = (sign_w, 28.0 * s);
                        let bx = lx;
                        let sign = mix(mix(ACCENT, [150, 104, 30, 255], 0.30), ACCENT_HOT, glow);
                        self.text.rounded(r, scene, [bx, cy - bh * 0.5, bx + bw, cy + bh * 0.5], 7.0 * s, sign);
                        let (lx0, ly0) = (bx + (bw - bl.w as f32) * 0.5, cy - bl.h as f32 * 0.5);
                        scene.overlays.push((bl.tex, [lx0, ly0, lx0 + bl.w as f32, ly0 + bl.h as f32]));
                        let tx = bx + bw + 14.0 * s;
                        let d = 22.0 * s;
                        self.text.rounded(r, scene, [rx - d, cy - d * 0.5, rx, cy + d * 0.5], d * 0.5, fade(ACCENT, 0.10 + 0.30 * glow));
                        let aw = self.text.width("›", (px + 2) as f32);
                        self.put(r, scene, "›", px + 2, mix(MUTED, ACCENT_HOT, glow), rx - d * 0.5 - aw * 0.5, cy - 1.0 * s);
                        let info = clip_to(&self.text, info, px as f32, rx - d - 12.0 * s - tx);
                        self.put(r, scene, &info, px, ink, tx, cy);
                        done = true;
                    }
                }
                MenuKind::List if code_w > 0.0 => {
                    if let Some((code, name)) = code_and_name(label) {
                        self.put_right(r, scene, code, px, mix(MUTED, ACCENT_HOT, glow), lx + code_w, cy);
                        let nx = lx + code_w + 12.0 * s;
                        let name = clip_to(&self.text, name, px as f32, rx - nx);
                        self.put(r, scene, &name, px, ink, nx, cy);
                        done = true;
                    }
                }
                // a tour: its name (the time is chosen beside the list)
                MenuKind::Tours => {
                    if let Some(rest) = label.strip_prefix(tour_pre.as_str()) {
                        let num = rest.split_once("  ").map(|(n, _)| n).unwrap_or(rest).trim();
                        let name = clip_to(&self.text, &format!("{tour_pre}{num}"), px as f32, rx - lx);
                        self.put(r, scene, &name, px, ink, lx, cy);
                        done = true;
                    }
                }
                _ => {}
            }
            if !done {
                let (text, more) = strip_more(label);
                let mut avail = rx - lx;
                if is_back {
                    let plain = matches!(kind, MenuKind::Lines | MenuKind::Tours);
                    if plain {
                        self.text.rounded(r, scene, rect, ROW_R * s, fade(LIT, 0.55));
                    }
                    let idle = if plain { SOFT } else { MUTED };
                    self.put(r, scene, &format!("‹  {text}"), px, if lit { WHITE } else { idle }, lx, cy);
                } else {
                    if off {
                        // (why the line cannot be chosen)
                        let cw = self.put_right(r, scene, "No active route", (12.0 * s) as u32, OFF_HINT, rx, cy);
                        avail -= cw + 10.0 * s;
                    }
                    if more {
                        // (the line opens another list)
                        let cw = self.put_right(r, scene, "›", px + 6, if lit { WHITE } else { MUTED }, rx, cy);
                        avail -= cw + 10.0 * s;
                    } else if id == "resume" && keys {
                        // ("Resume" shows the key that does the same)
                        let left = self.chip(r, scene, "Esc", (12.0 * s) as u32, WHITE, [52, 52, 52, 255], true, rx, cy, s);
                        avail = left - 10.0 * s - lx;
                    }
                    let text = clip_to(&self.text, text, px as f32, avail);
                    self.put(r, scene, &text, px, ink, lx, cy);
                }
            }
            self.menu_rects.push(rect);
        }
        if back_footer {
            // ("Back" pinned to the bottom of the list: the rects of the lines
            // out of view are empty ones, so that a rect stays at the index of its line)
            let bk = items.len() - 1;
            let fr = [x + pad, y + h - pad - 36.0 * s, right, y + h - pad];
            let lit = over(fr) || (sel == bk && f.menu_kbd && !any_hovered);
            let glow = self.easeq((7, "back", bk), if lit { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
            self.text.rounded(r, scene, fr, ROW_R * s, fade(LIT, 0.55 + 0.45 * glow));
            if glow > 0.0 {
                self.accent_bar(r, scene, fr, glow, false, s);
            }
            let (text, _) = strip_more(back_txt.as_str());
            self.put(r, scene, &format!("‹  {text}"), px, mix(SOFT, WHITE, glow), fr[0] + tin, (fr[1] + fr[3]) * 0.5);
            while self.menu_rects.len() < bk - start {
                self.menu_rects.push([-1.0e9; 4]);
            }
            self.menu_rects.push(fr);
        }
        // the timetable of the chosen line or tour, beside the list
        if let Some(p) = preview {
            let (px0, py0) = (list_r + 4.0 * s, y + header_h);
            let (px1, py1) = (x + w - pad, y + h - pad);
            self.text.rounded(r, scene, [px0 - 1.0, py0 - 1.0, px1 + 1.0, py1 + 1.0], CARD_R * s + 1.0, BORDER);
            self.text.rounded(r, scene, [px0, py0, px1, py1], CARD_R * s, PANEL_ALT);
            let pad = tin;
            let inner = px1 - px0 - pad * 2.0;
            let mut cy = py0 + 28.0 * s;
            let head = clip_to(&self.text, &p.title, 16.0 * s, inner);
            self.put(r, scene, &head, (16.0 * s) as u32, WHITE, px0 + pad, cy);
            cy += 22.0 * s;
            let meta = clip_to(&self.text, &p.meta, 11.0 * s, inner);
            self.put(r, scene, &meta, (11.0 * s) as u32, MUTED, px0 + pad, cy);
            cy += 18.0 * s;
            scene.overlays.push((sep, [px0 + pad, cy.round(), px1 - pad, cy.round() + 1.0]));
            let rpx = (13.0 * s) as u32;
            let lh = 24.0 * s;
            let mut top = cy + 10.0 * s;
            // the tour, and the time of the trip, each between arrows that step through them
            let nav_rows: Vec<(&String, f32, usize)> = p.time.iter().map(|t| (t, 20.0f32, 0usize)).collect();
            for (time, fs, base) in nav_rows {
                let bh = 30.0 * s;
                let bw = 46.0 * s;
                let by = top;
                let right = px1 - pad;
                let left = px0 + pad;
                for j in 0..2usize {
                    let rect = if j == 0 { [left, by, left + bw, by + bh] } else { [right - bw, by, right, by + bh] };
                    let a = self.easeq((14, "time", j + base), if over(rect) { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
                    self.text.rounded(r, scene, rect, ROW_R * s, mix([232, 160, 48, 60], ACCENT, a * 0.7));
                    let col = mix(ACCENT_HOT, [18, 14, 8, 255], a);
                    let (cx, cy) = ((rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5);
                    // (a real arrow: a shaft and a head of stacked strips)
                    let dir = if j == 0 { -1.0 } else { 1.0 };
                    let (half, head_w, head_h) = (9.0 * s, 7.0 * s, 7.0 * s);
                    let t = (2.0 * s).max(2.0);
                    self.text.rounded(r, scene, [cx - half, cy - t * 0.5, cx + half, cy + t * 0.5], 0.0, col);
                    let tip = cx + dir * half;
                    let strips = 7;
                    for k in 0..strips {
                        // (strip k, from the head's base towards the tip, narrowing)
                        let bx = tip - dir * head_w * (1.0 - k as f32 / strips as f32);
                        let ex = tip - dir * head_w * (1.0 - (k as f32 + 1.0) / strips as f32);
                        let h = head_h * (1.0 - (k as f32 + 0.5) / strips as f32);
                        self.text.rounded(r, scene, [bx.min(ex), cy - h, bx.max(ex), cy + h], 0.0, col);
                    }
                    self.menu_time.push(rect);
                }
                let mid_l = left + bw + 10.0 * s;
                let mid_r = right - bw - 10.0 * s;
                let time = clip_to(&self.text, time, fs * s, mid_r - mid_l);
                let tw = self.text.width(&time, fs * s);
                self.put(r, scene, &time, (fs * s) as u32, if base == 0 { AMBER } else { WHITE }, mid_l + (mid_r - mid_l - tw) * 0.5, by + bh * 0.5);
                top = by + bh + 8.0 * s;
            }
            let n = p.rows.len();
            if let (Some(chosen), Some(button)) = (p.chosen, p.button.as_ref()) {
                // the stops to start from: the one chosen marked, a click chooses another
                let go_h = 34.0 * s;
                let go = [px0 + pad, py1 - 12.0 * s - go_h, px1 - pad, py1 - 12.0 * s];
                let fit = (((go[1] - 10.0 * s) - top) / lh).floor().max(1.0) as usize;
                let first = match f.pane_first {
                    Some(p) if n > fit => p.min(n - fit),
                    _ if n > fit => chosen.saturating_sub(fit / 2).min(n - fit),
                    _ => 0,
                };
                self.menu_pane_start = first;
                self.menu_pane_box = Some([px0, py0, px1, py1]);
                // (a long list of stops: a thin scroll bar at the pane's edge)
                if n > fit {
                    let hot = over([px0, py0, px1, py1]);
                    let track = [px1 - 7.0 * s, top, px1 - 3.0 * s, go[1] - 10.0 * s];
                    let thumb = self.thumb(r, scene, track, first, fit, n, hot, s);
                    self.menu_pane_scroll = Some((track, [thumb[0] - 6.0 * s, thumb[1], thumb[2] + 3.0 * s, thumb[3]], n, fit));
                }
                let time_w = p.rows.iter().skip(first).take(fit).map(|row| self.text.width(&row.1, rpx as f32)).fold(0.0f32, f32::max);
                for (i, (what, when)) in p.rows.iter().enumerate().skip(first).take(fit) {
                    let ry = top + lh * (i - first) as f32 + lh * 0.5;
                    let rect = [px0 + 8.0 * s, ry - lh * 0.5, px1 - 8.0 * s, ry + lh * 0.5];
                    let on = i == chosen;
                    let hov = over(rect) && !on;
                    let a_on = self.easeq((11, "stop", i), if on { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
                    let a_hov = self.easeq((12, "stop", i), if hov { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
                    if a_hov > 0.0 {
                        self.text.rounded(r, scene, rect, ROW_R * s, fade(LIT, a_hov));
                    }
                    if a_on > 0.0 {
                        self.text.rounded(r, scene, rect, ROW_R * s, fade(SELECTED, a_on));
                        self.text.rounded(r, scene, [rect[0], rect[1] + 7.0 * s, rect[0] + 2.0 * s, rect[3] - 7.0 * s], 1.0 * s, fade(ACCENT, a_on));
                    }
                    self.put_right(r, scene, when, rpx, AMBER, px1 - pad, ry);
                    let what = clip_to(&self.text, what, rpx as f32, inner - time_w - 14.0 * s);
                    self.put(r, scene, &what, rpx, mix(SOFT, WHITE, a_on), px0 + pad, ry);
                    self.menu_pane.push(rect);
                }
                // the button that starts the trip
                let a_go = self.easeq((13, "go", 0), if over(go) { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
                self.text.rounded(r, scene, go, ROW_R * s, mix(ACCENT, ACCENT_HOT, a_go));
                let l = self.text.label(r, scene, button, (14.0 * s) as u32 | BOLD, ON_ACCENT);
                let (gx, gy) = (go[0] + (go[2] - go[0] - l.w as f32) * 0.5, (go[1] + go[3]) * 0.5 - l.h as f32 * 0.5);
                scene.overlays.push((l.tex, [gx, gy, gx + l.w as f32, gy + l.h as f32]));
                self.menu_pane_go = Some(go);
            } else {
                let fit = ((py1 - 12.0 * s - top) / lh).floor().max(1.0) as usize;
                // (a list too long for the pane ends in how many more there are)
                let shown = if n > fit { fit.saturating_sub(1) } else { n };
                let time_w = p.rows.iter().take(shown).map(|row| self.text.width(&row.1, rpx as f32)).fold(0.0f32, f32::max);
                let tp = format!("{} ", omsi_ui::tr("Tour"));
                // (the tour numbers in tiles of one width, the destination after them)
                let tile_w = p.rows.iter().take(shown).filter_map(|row| row.0.strip_prefix(tp.as_str()).and_then(|x| x.split_once("  ›  ").map(|(n, _)| n).or(Some(x)))).map(|n| self.text.width(n.trim(), rpx as f32) + 16.0 * s).fold(30.0 * s, f32::max);
                for (i, (what, when)) in p.rows.iter().take(shown).enumerate() {
                    let ry = top + lh * i as f32 + lh * 0.5;
                    self.put_right(r, scene, when, rpx, AMBER, px1 - pad, ry);
                    if let Some(rest) = what.strip_prefix(tp.as_str()) {
                        let (num, dest) = rest.split_once("  ›  ").unwrap_or((rest, ""));
                        let th = lh - 6.0 * s;
                        self.text.rounded(r, scene, [px0 + pad, ry - th * 0.5, px0 + pad + tile_w, ry + th * 0.5], 5.0 * s, ACCENT_SOFT);
                        let nw = self.text.width(num.trim(), rpx as f32);
                        self.put(r, scene, num.trim(), rpx, txt(ACCENT), px0 + pad + (tile_w - nw) * 0.5, ry);
                        let dx = px0 + pad + tile_w + 10.0 * s;
                        let dest = clip_to(&self.text, dest, rpx as f32, px1 - pad - time_w - 14.0 * s - dx);
                        self.put(r, scene, &dest, rpx, SOFT, dx, ry);
                    } else {
                        let what = clip_to(&self.text, what, rpx as f32, inner - time_w - 14.0 * s);
                        self.put(r, scene, &what, rpx, SOFT, px0 + pad, ry);
                    }
                }
                if n > shown {
                    let ry = top + lh * shown as f32 + lh * 0.5;
                    self.put(r, scene, &omsi_ui::tr("+{} more").replacen("{}", &(n - shown).to_string(), 1), (12.0 * s) as u32, MUTED, px0 + pad, ry);
                }
            }
        }
        self.menu_overlay_range = overlay_start..scene.overlays.len();
    }
}

/// The launcher's rail and its separator line (`launcher/theme.rs` `RAIL`, `EDGE`).
const RAIL: [u8; 4] = [18, 18, 18, 255];
const EDGE: [u8; 4] = [255, 255, 255, 15];

/// The Material icon of a line of the game menu (the launcher's icon set).
fn game_menu_icon(id: &str) -> &'static str {
    match id {
        "resume" => "play_arrow",
        "tobus" => "directions_bus",
        "options" => "settings",
        "controls" => "sports_esports",
        "camera" => "videocam",
        "vehicle" => "airport_shuttle",
        "world" => "partly_cloudy_day",
        "map" => "map",
        "duty" => "departure_board",
        "skipstop" => "near_me",
        "endduty" => "sports_score",
        "save" => "save",
        "saveslot" => "add",
        "load" => "history",
        "shot" => "photo_camera",
        "admin" => "key",
        "quit" => "logout",
        _ => "chevron_right",
    }
}

impl Ui {
    /// The pause menu as the launcher's rail: a full-height panel on the left over the
    /// dimmed picture - the game's name and version on top, "Resume" as the launcher's
    /// amber primary button, the other lines as its pages (an icon, the name, the chosen one
    /// on a lighter grey with the accent bar), "End the session" in the danger colour at the
    /// end. A menu longer than the screen scrolls, as the card did.
    fn draw_game_menu(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame, sel: usize, items: &[(&str, &str)]) {
        let s = menu_scale(f);
        let dim = self.text.plate(r, scene, 6);
        scene.overlays.push((dim, [0.0, 0.0, f.width, f.height]));
        let keys = !crate::platform::touch_controls();
        // (a phone draws to its edges, round the camera's hole in the screen: on the rail's
        // side in one of the two landscape turns - its contents keep clear of it)
        let cut = if crate::platform::MOBILE || omsi_cfg::flags::OMSI_MOBILE.is_set() { 18.0 * s } else { 0.0 };
        let rail_w = (264.0 * s + cut).min(f.width * 0.86).round();
        // the rail: flat, a hairline at its edge, a soft shadow over the picture
        self.text.shadow(r, scene, [-40.0 * s, -40.0 * s, rail_w, f.height + 40.0 * s], 0.0, 30.0 * s, 0.0, 120);
        self.text.rounded(r, scene, [0.0, 0.0, rail_w, f.height], 0.0, RAIL);
        self.text.rounded(r, scene, [rail_w - 1.0, 0.0, rail_w, f.height], 0.0, EDGE);
        // the game's name and version, as at the top of the launcher's rail
        let bx = 24.0 * s + cut;
        let name_w = self.put(r, scene, "openOMSI", (20.0 * s) as u32 | BOLD, WHITE, bx, 34.0 * s);
        self.put(r, scene, crate::startup::VERSION, (12.0 * s) as u32, MUTED, bx, 54.0 * s);
        // what state the game is in, a small amber tag beside the name
        let tag = if f.paused { "PAUSED" } else { "MENU" };
        let tl = self.text.label(r, scene, tag, (10.0 * s) as u32 | BOLD, txt(ACCENT));
        let (tw, th) = (tl.w as f32 + 12.0 * s, tl.h as f32 + 4.0 * s);
        let tx = (bx + name_w + 10.0 * s).min(rail_w - tw - 12.0 * s);
        let ty = 34.0 * s - th * 0.5;
        self.text.rounded(r, scene, [tx, ty, tx + tw, ty + th], 4.0 * s, ACCENT_SOFT);
        scene.overlays.push((tl.tex, [tx + 6.0 * s, ty + 2.0 * s, tx + 6.0 * s + tl.w as f32, ty + 2.0 * s + tl.h as f32]));
        // the lines: as many as fit at the launcher's 42 px pitch (down to 32), the rest
        // scrolled to
        let n = items.len();
        let top = 80.0 * s;
        let foot = if keys { 40.0 * s } else { 12.0 * s };
        let room = (f.height - top - foot).max(32.0 * s);
        let pitch = (42.0 * s).min(room / n.max(1) as f32).max(32.0 * s);
        let rows = ((room / pitch).floor() as usize).clamp(1, n.max(1));
        let start = match (n > rows, f.menu_top) {
            (false, _) => 0,
            (true, Some(t)) => (t.max(0.0).round() as usize).min(n - rows),
            (true, None) => sel.min(n.saturating_sub(1)).saturating_sub(rows / 2).min(n - rows),
        };
        self.menu_start = start;
        self.menu_rows = rows;
        self.menu_row_h = pitch;
        let scrolls = n > rows;
        let x0 = 12.0 * s + cut;
        let x1 = rail_w - if scrolls { 20.0 * s } else { 12.0 * s };
        if scrolls {
            let track = [rail_w - 10.0 * s, top, rail_w - 6.0 * s, top + pitch * rows as f32 - 4.0 * s];
            self.menu_scroll_track = Some(track);
            let hot = over_rect(f.cursor, [0.0, 0.0, rail_w, f.height]);
            let thumb = self.thumb(r, scene, track, start, rows, n, hot, s);
            self.menu_scroll_thumb = Some([thumb[0] - 6.0 * s, thumb[1], thumb[2] + 6.0 * s, thumb[3]]);
        }
        let any_hovered = over_rect(f.cursor, [0.0, top, rail_w, top + pitch * rows as f32]);
        let px = (13.5 * s).round() as u32;
        let icon_px = (18.0 * s).round() as u32;
        for (k, &(id, label)) in items.iter().enumerate().skip(start).take(rows) {
            let ry = top + pitch * (k - start) as f32;
            let rect = [x0, ry, x1, ry + pitch - 4.0 * s];
            let off = f.menu_disabled.contains(&id);
            let hov = !off && over_rect(f.cursor, rect);
            let chosen = !off && k == sel && f.menu_kbd && !any_hovered;
            let lit = hov || chosen;
            let danger = id == "quit";
            let primary = id == "resume";
            // a hairline between the groups: the everyday lines, saving, the way out
            if k > start && matches!(id, "save" | "admin" | "quit") {
                let sy = (ry - 2.0 * s).round();
                self.text.rounded(r, scene, [x0 + 8.0 * s, sy, x1 - 8.0 * s, sy + 1.0], 0.0, EDGE);
            }
            let glow = self.easeq((7, id, k), if lit { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
            let cy = (rect[1] + rect[3]) * 0.5;
            let (ink, icon_ink) = if primary {
                // the launcher's primary button: amber, dark bold text, lighter under the mouse
                self.text.rounded(r, scene, rect, 6.0 * s, mix(ACCENT, ACCENT_HOT, glow));
                ([18, 14, 8, 0], [18, 14, 8, 255])
            } else {
                if glow > 0.0 {
                    let fill = if danger { LIT_DANGER } else if chosen { SELECTED } else { LIT };
                    self.text.rounded(r, scene, rect, 6.0 * s, fade(fill, glow));
                    self.accent_bar(r, scene, rect, glow, danger, s);
                }
                let c = if off {
                    OFF_INK
                } else if danger {
                    mix([222, 110, 100, 0], [255, 176, 166, 0], glow)
                } else {
                    mix(MUTED, WHITE, glow)
                };
                (c, [c[0], c[1], c[2], 255])
            };
            if let Some(t) = self.text.icon(r, scene, game_menu_icon(id), icon_px, icon_ink) {
                let (ix, iy) = ((rect[0] + 20.0 * s - icon_px as f32 * 0.5).round(), (cy - icon_px as f32 * 0.5).round());
                scene.overlays.push((t, [ix, iy, ix + icon_px as f32, iy + icon_px as f32]));
            }
            let (text, more) = strip_more(label);
            let lx = rect[0] + 40.0 * s;
            let mut right = rect[2] - 12.0 * s;
            if more {
                if let Some(t) = self.text.icon(r, scene, "chevron_right", icon_px, fade(icon_ink, if off { 1.0 } else { 0.55 + 0.45 * glow })) {
                    let (ix, iy) = ((right - icon_px as f32 + 4.0 * s).round(), (cy - icon_px as f32 * 0.5).round());
                    scene.overlays.push((t, [ix, iy, ix + icon_px as f32, iy + icon_px as f32]));
                }
                right -= icon_px as f32;
            } else if primary && keys {
                // ("Resume" shows the key that does the same)
                let left = self.chip(r, scene, "Esc", (11.0 * s) as u32, [18, 14, 8, 0], [255, 214, 150, 255], false, right + 4.0 * s, cy, s);
                right = left - 6.0 * s;
            } else if off {
                let cw = self.put_right(r, scene, "No active route", (11.0 * s) as u32, OFF_HINT, right, cy);
                right -= cw + 8.0 * s;
            }
            let size = if primary { (14.0 * s).round() as u32 | BOLD } else { px };
            let text = clip_to(&self.text, text, (size & !BOLD) as f32, right - lx);
            self.put(r, scene, &text, size, ink, lx, cy);
            self.menu_rects.push(rect);
        }
        // the keys, quietly at the bottom, where there is a keyboard
        if keys {
            let hint = omsi_ui::tr("Esc resumes  ·  P pauses").into_owned();
            let hint = clip_to(&self.text, &hint, 11.0 * s, rail_w - bx * 2.0);
            self.put(r, scene, &hint, (11.0 * s) as u32, OFF_INK, bx, f.height - 20.0 * s);
        }
    }
}

impl Ui {
    /// A settings window (options, vehicle, world): a sidebar of pages on the left, the rows
    /// of the page shown on the right. A row is `name\u{1f}kind\u{1f}value\u{1f}description\u{1f}fraction`:
    /// `s` a switch (value "on"/"off"), `v` a slider (value text, fraction of the way),
    /// `c` a stepper (value between arrows), `o` opens a list, `a` a button (its text is the
    /// value), `i` information.
    fn draw_settings(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame, sel: usize, items: &[(&str, &str)]) {
        let s = menu_scale(f);
        let dim = self.text.plate(r, scene, 6);
        let sep = self.text.plate(r, scene, 9);
        scene.overlays.push((dim, [0.0, 0.0, f.width, f.height]));
        let none: Vec<String> = Vec::new();
        let (titles, active) = match f.menu_tabs.as_ref() {
            Some((t, a)) => (t, *a),
            None => (&none, 0),
        };
        // a window of a fixed size (as far as the screen allows): the pages differ in how many
        // rows they have, the window does not - a long page scrolls inside it
        let want_side = if titles.is_empty() { 0.0 } else { 200.0 * s };
        let w = (want_side + 680.0 * s).min(f.width - 24.0 * s).max(260.0 * s);
        let side_w = want_side.min(w * 0.38);
        let header_h = 72.0 * s;
        let pad = PAD * s;
        let tin = TEXT_IN * s;
        let fixed_h = (if f.vr { 440.0 } else { 600.0 }) * s;
        let h = fixed_h.min(f.height * (if f.vr { 0.70 } else { 0.94 })).max(220.0 * s);
        let room = h - header_h - pad;
        let row_h = ((if f.vr { 54.0 } else { 62.0 }) * s).min(room).max(30.0 * s);
        let rows = ((room / row_h).floor() as usize).clamp(1, items.len().max(1));
        let start = match (items.len() > rows, f.menu_top) {
            (false, _) => 0,
            (true, Some(top)) => (top.max(0.0).round() as usize).min(items.len() - rows),
            (true, None) => sel.saturating_sub(rows / 2).min(items.len() - rows),
        };
        self.menu_start = start;
        self.menu_rows = rows;
        self.menu_row_h = row_h;
        let x = ((f.width - w) * 0.5).round();
        let y = ((f.height - h) * 0.5).round();
        let radius = CARD_R * s;
        self.text.shadow(r, scene, [x, y, x + w, y + h], radius, 28.0 * s, 10.0 * s, 110);
        self.text.rounded(r, scene, [x - 1.0, y - 1.0, x + w + 1.0, y + h + 1.0], radius + 1.0, BORDER);
        self.text.rounded(r, scene, [x, y, x + w, y + h], radius, PANEL);
        // the header: the game's name small, the title large
        let (title, sub): (String, String) = match f.menu_head.as_ref() {
            Some((t, u)) => (t.clone(), u.clone()),
            None => ("Options".to_string(), String::new()),
        };
        self.menu_header(r, scene, x, y, w, header_h, &title, &sub, s);
        let over = |rect: [f32; 4]| f.cursor.0 >= rect[0] && f.cursor.0 <= rect[2] && f.cursor.1 >= rect[1] && f.cursor.1 <= rect[3];
        // the sidebar: the pages, and the way back at its foot
        if side_w > 0.0 {
            let (sx0, sx1) = (x + pad, x + side_w);
            self.text.rounded(r, scene, [sx0 - 1.0, y + header_h - 1.0, sx1 + 1.0, y + h - pad + 1.0], CARD_R * s + 1.0, BORDER);
            self.text.rounded(r, scene, [sx0, y + header_h, sx1, y + h - pad], CARD_R * s, PANEL_ALT);
            let inset = 6.0 * s;
            let bottom = y + h - pad - inset;
            let pages_top = y + header_h + inset;
            let step = if f.vr {
                vr_settings_sidebar_step(bottom - 38.0 * s - inset - pages_top, titles.len(), s)
            } else {
                42.0 * s
            };
            let page_h = (step - 4.0 * s).max(1.0);
            let spx = (15.0 * s).min(page_h * 0.6).max(1.0) as u32;
            for (i, title) in titles.iter().enumerate() {
                let top = pages_top + i as f32 * step;
                let rect = [sx0 + inset, top, sx1 - inset, top + page_h];
                let on = i == active;
                let hov = over(rect) && !on;
                // (the page chosen and the page under the mouse fade in and out)
                let a_on = self.easeq((1, title.as_str(), 0), if on { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
                let a_hov = self.easeq((2, title.as_str(), 0), if hov { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
                let a_bar = self.easeq((9, title.as_str(), 0), if on { 1.0 } else { 0.0 }, 1.0 / BAR_SECS);
                if a_hov > 0.0 {
                    self.text.rounded(r, scene, rect, ROW_R * s, fade(LIT, a_hov));
                }
                if a_on > 0.0 {
                    self.text.rounded(r, scene, rect, ROW_R * s, fade(SELECTED, a_on));
                }
                if a_bar > 0.0 {
                    self.accent_bar(r, scene, rect, a_bar, false, s);
                }
                let ink = mix(mix(MUTED, SOFT, a_hov), WHITE, a_on);
                let text = clip_to(&self.text, title, spx as f32, rect[2] - rect[0] - tin * 2.0);
                self.put(r, scene, &text, spx, ink, rect[0] + tin, (rect[1] + rect[3]) * 0.5);
                self.menu_side.push(rect);
            }
            let rect = [sx0 + inset, bottom - 38.0 * s, sx1 - inset, bottom];
            let a_back = self.easeq((3, "back", 0), if over(rect) { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
            if a_back > 0.0 {
                self.text.rounded(r, scene, rect, ROW_R * s, fade(LIT, a_back));
            }
            let back = format!("‹  {}", omsi_ui::tr("Back"));
            self.put(r, scene, &back, spx, mix(MUTED, WHITE, a_back), rect[0] + tin, (rect[1] + rect[3]) * 0.5);
            self.menu_side.push(rect);
        }
        // the scroll bar of a long page
        let scrolls = items.len() > rows;
        let cx0 = x + side_w + if side_w > 0.0 { 8.0 * s } else { pad };
        let cx1 = x + w - pad - if scrolls { 8.0 * s } else { 0.0 };
        if scrolls {
            let top = y + header_h;
            let track = [x + w - 12.0 * s, top, x + w - 8.0 * s, top + row_h * rows as f32 - 4.0 * s];
            self.menu_scroll_track = Some(track);
            let hot = over_rect(f.cursor, [x + side_w, y + header_h, x + w, y + h]);
            let thumb = self.thumb(r, scene, track, start, rows, items.len(), hot, s);
            self.menu_scroll_thumb = Some([thumb[0] - 6.0 * s, thumb[1], thumb[2] + 6.0 * s, thumb[3]]);
        }
        // the rows
        let any_hovered = over([x + side_w, y + header_h, x + w, y + h]);
        let px = ((15.0 * s).min(row_h * 0.34)) as u32;
        // (the light of the row above: the line between two rows is hidden when either is lit)
        let mut prev_a = 0.0f32;
        for (k, &(id, label)) in items.iter().enumerate().skip(start).take(rows) {
            let ry = y + header_h + row_h * (k - start) as f32;
            let rect = [cx0, ry, cx1, ry + row_h - 4.0 * s];
            // (lit by the mouse over it; by the keyboard's choice only when the keyboard chose)
            let lit = f.dropdown.is_none() && (over(rect) || (k == sel && f.menu_kbd && !any_hovered));
            // the light of the row eases in and out
            let a = self.easeq((4, id, k), if lit { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
            let a_bar = self.easeq((10, id, k), if lit { 1.0 } else { 0.0 }, 1.0 / BAR_SECS);
            if a > 0.0 {
                self.text.rounded(r, scene, rect, ROW_R * s, fade(LIT, a));
            }
            if a_bar > 0.0 {
                self.accent_bar(r, scene, rect, a_bar, false, s);
            }
            if k > start && a.max(prev_a) < 0.5 {
                let sy = (rect[1] - 2.0 * s).round();
                scene.overlays.push((sep, [rect[0] + tin, sy, rect[2] - tin, sy + 1.0]));
            }
            prev_a = a;
            let ink = mix(SOFT, WHITE, a);
            let cy = (rect[1] + rect[3]) * 0.5;
            let nx = rect[0] + tin;
            let rx = rect[2] - tin;
            let mut parts = label.split('\u{1f}');
            let name = parts.next().unwrap_or("");
            let kind = parts.next().unwrap_or("a");
            let value = parts.next().unwrap_or("");
            let desc = parts.next().unwrap_or("");
            let frac: Option<f32> = parts.next().and_then(|p| p.parse().ok());
            let mut ctl: Option<[f32; 4]> = None;
            let left: f32 = match kind {
                // a switch
                "s" => {
                    let on = value == "on";
                    // (the launcher's switch - 34 x 18, a white knob 6 px smaller than the
                    // track - a tenth larger for the menu's larger text)
                    let (tw, th) = (38.0 * s, 20.0 * s);
                    let tx = rx - tw;
                    let track = [tx, cy - th * 0.5, tx + tw, cy + th * 0.5];
                    // (the knob slides over and the track changes its colour)
                    let t = self.ease((5, id, k), if on { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
                    let tq = quant(t);
                    self.text.rounded(r, scene, track, th * 0.5, mix(TRACK_OFF, ACCENT, tq));
                    let kn = th - 6.0 * s;
                    let kx = tx + 3.0 * s + (tw - kn - 6.0 * s) * t;
                    self.text.rounded(r, scene, [kx, cy - kn * 0.5, kx + kn, cy + kn * 0.5], kn * 0.5, KNOB);
                    tx
                }
                // a slider: the track with its knob, the value right of it
                "v" => {
                    let vw = 64.0 * s;
                    self.put_right(r, scene, value, (14.0 * s) as u32, mix(SOFT, WHITE, a), rx, cy);
                    let tw = (200.0 * s).min((rx - nx) * 0.45);
                    let x1 = rx - vw - 10.0 * s;
                    let x0 = x1 - tw;
                    let th = 4.0 * s;
                    self.text.rounded(r, scene, [x0, cy - th * 0.5, x1, cy + th * 0.5], th * 0.5, SLIDER_TRACK);
                    // (the knob glides to its place and grows a little under the mouse)
                    let fr = self.ease((6, id, k), frac.unwrap_or(0.0).clamp(0.0, 1.0), 2.0 / FADE_SECS);
                    let fx = x0 + tw * fr;
                    if fx - x0 >= 1.0 {
                        self.text.rounded(r, scene, [x0, cy - th * 0.5, fx, cy + th * 0.5], th * 0.5, ACCENT);
                    }
                    let kn = (13.0 + 2.0 * a) * s;
                    self.text.rounded(r, scene, [fx - kn * 0.5, cy - kn * 0.5, fx + kn * 0.5, cy + kn * 0.5], kn * 0.5, KNOB);
                    ctl = Some([x0, rect[1], x1, rect[3]]);
                    x0
                }
                // a stepper: the value between two arrows, the left half steps back
                "c" => {
                    let text = format!("‹  {value}  ›");
                    let l = self.chip(r, scene, &text, (13.0 * s) as u32, mix(SOFT, WHITE, a), mix(CHIP, SELECTED, a), false, rx, cy, s);
                    ctl = Some([l, rect[1], rx, rect[3]]);
                    l
                }
                // opens another list
                "o" => {
                    let cw = self.put_right(r, scene, "›", px + 6, mix(MUTED, WHITE, a), rx, cy);
                    if value.is_empty() {
                        rx - cw
                    } else {
                        // (the value now, as the stepper showed it)
                        let vw = self.put_right(r, scene, value, (14.0 * s) as u32, SOFT, rx - cw - 8.0 * s, cy);
                        rx - cw - 8.0 * s - vw
                    }
                }
                "i" => {
                    let cw = self.put_right(r, scene, value, (14.0 * s) as u32, SOFT, rx, cy);
                    rx - cw
                }
                // a value being typed: lit in the accent
                "E" => self.chip(r, scene, value, (13.0 * s) as u32, AMBER, ACCENT_SOFT, false, rx, cy, s),
                // a button
                _ => {
                    if value.is_empty() {
                        rx
                    } else {
                        self.chip(r, scene, value, (13.0 * s) as u32, mix(SOFT, AMBER, a), mix(CHIP, ACCENT_SOFT, a), false, rx, cy, s)
                    }
                }
            };
            let avail = left - 16.0 * s - nx;
            if desc.is_empty() {
                let n = clip_to(&self.text, name, px as f32, avail);
                self.put(r, scene, &n, px, ink, nx, cy);
            } else {
                let n = clip_to(&self.text, name, px as f32, avail);
                self.put(r, scene, &n, px, ink, nx, cy - 9.0 * s);
                let dpx = (12.0 * s) as u32;
                let d = clip_to(&self.text, desc, dpx as f32, avail);
                self.put(r, scene, &d, dpx, MUTED, nx, cy + 10.0 * s);
            }
            self.menu_rects.push(rect);
            self.menu_ctl.push(ctl);
        }
        // a drop-down over a row (the weather preset, the clouds): the entries under the
        // row's value as a select's in a page - over it where there is no room under
        if let Some(dd) = f.dropdown.as_ref().filter(|d| d.row >= start && d.row < start + rows && !d.items.is_empty()) {
            let ry = y + header_h + row_h * (dd.row - start) as f32;
            let row_b = ry + row_h - 4.0 * s;
            let item_h = (36.0 * s).min(row_h);
            let inner = 4.0 * s;
            let below = y + h - pad - row_b - 4.0 * s;
            let above = ry - (y + header_h) - 4.0 * s;
            let want = dd.items.len().min(8);
            let fits = |room: f32| (((room - 2.0 * inner) / item_h).floor().max(0.0) as usize).min(want);
            let down = fits(below) >= want || fits(below) >= fits(above);
            let n_vis = (if down { fits(below) } else { fits(above) }).max(1);
            let ph = n_vis as f32 * item_h + 2.0 * inner;
            let pw = (300.0 * s).min(cx1 - cx0);
            let (px1, py0) = (cx1 - 6.0 * s, if down { row_b + 4.0 * s } else { ry - 4.0 * s - ph });
            let px0 = px1 - pw;
            let panel = [px0, py0, px1, py0 + ph];
            let top = dd.top.min(dd.items.len() - n_vis.min(dd.items.len()));
            self.dd_top = top;
            self.dd_rows = n_vis;
            let rad = (CARD_R * s).min(10.0 * s);
            self.text.shadow(r, scene, panel, rad, 18.0 * s, 6.0 * s, 150);
            // (the launcher's popup: the field's grey, a hairline round it)
            self.text.rounded(r, scene, [panel[0] - 1.0, panel[1] - 1.0, panel[2] + 1.0, panel[3] + 1.0], rad + 1.0, BORDER);
            self.text.rounded(r, scene, panel, rad, PANEL_ALT);
            let more = dd.items.len() > n_vis;
            let dpx = (14.0 * s) as u32;
            let tin = TEXT_IN * s;
            for i in 0..n_vis {
                let idx = top + i;
                let rect = [px0 + inner, py0 + inner + i as f32 * item_h, px1 - inner - if more { 8.0 * s } else { 0.0 }, py0 + inner + (i + 1) as f32 * item_h];
                let cur = dd.current == Some(idx);
                let hot = over(rect) || (idx == dd.sel && f.menu_kbd && !over(panel));
                let a = self.easeq((15, "dropdown", idx), if hot { 1.0 } else { 0.0 }, 1.0 / FADE_SECS);
                if cur {
                    self.text.rounded(r, scene, rect, ROW_R * s, LIT);
                }
                if a > 0.0 {
                    self.text.rounded(r, scene, rect, ROW_R * s, fade(SELECTED, a));
                }
                if cur {
                    self.accent_bar(r, scene, rect, 1.0, false, s);
                }
                let text = clip_to(&self.text, dd.items[idx], dpx as f32, rect[2] - rect[0] - tin * 2.0);
                self.put(r, scene, &text, dpx, if cur { WHITE } else { mix(SOFT, WHITE, a) }, rect[0] + tin, (rect[1] + rect[3]) * 0.5);
                self.dd_rects.push(rect);
            }
            if more {
                let track = [px1 - 8.0 * s, py0 + inner, px1 - 4.0 * s, py0 + ph - inner];
                let thumb = self.thumb(r, scene, track, top, n_vis, dd.items.len(), over(panel), s);
                self.dd_scroll = Some((track, [thumb[0] - 6.0 * s, thumb[1], thumb[2] + 4.0 * s, thumb[3]]));
            }
        }
    }
}

/// `text` broken into lines of at most `width` pixels, between words.
fn wrap(tc: &TextCache, text: &str, px: f32, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let try_line = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if tc.width(&try_line, px) > width && !line.is_empty() {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = try_line;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// `text` cut at the end to fit `width` pixels ("…").
fn clip_to(tc: &TextCache, text: &str, px: f32, width: f32) -> String {
    if tc.width(text, px) <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let head = |n: usize| {
        let mut t: String = chars[..n].iter().collect();
        t.push('…');
        t
    };
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if tc.width_raw(&head(mid), px) <= width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    head(lo)
}

/// `text` in rows that fit `width` (as `measure` measures): broken between words, and inside a
/// word too long for a row. The rows after the first fit `width - indent` (they are drawn
/// indented, under the first).
fn wrap_rows(text: &str, width: f32, indent: f32, measure: &impl Fn(&str) -> f32) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut cur = String::new();
    let room = |rows: &Vec<String>| if rows.is_empty() { width } else { width - indent };
    for word in text.split(' ') {
        let joined = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if measure(&joined) <= room(&rows) {
            cur = joined;
            continue;
        }
        if !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
        }
        // the word alone, cut where a row is full (a long link, a run of letters)
        let mut w: Vec<char> = word.chars().collect();
        while w.len() > 1 && measure(&w.iter().collect::<String>()) > room(&rows) {
            let mut k = w.len() - 1;
            while k > 1 && measure(&w[..k].iter().collect::<String>()) > room(&rows) {
                k -= 1;
            }
            rows.push(w[..k].iter().collect());
            w.drain(..k);
        }
        cur = w.into_iter().collect();
    }
    if !cur.is_empty() || rows.is_empty() {
        rows.push(cur);
    }
    rows
}

/// `text` cut at the start to fit (the end of what is being typed stays visible).
fn clip_left(tc: &TextCache, text: &str, px: f32, width: f32) -> String {
    let mut t: Vec<char> = text.chars().collect();
    while t.len() > 1 && tc.width(&t.iter().collect::<String>(), px) > width {
        t.remove(0);
    }
    t.into_iter().collect()
}

/// A chat line with its bad words starred out (rustrict: profanity, slurs and the usual
/// ways of writing around a filter, without a word list to keep).
pub fn filter_chat(text: &str) -> String {
    use rustrict::CensorStr;
    text.censor()
}

/// Fit all VR page buttons above the separately reserved Back button.
fn vr_settings_sidebar_step(available: f32, pages: usize, scale: f32) -> f32 {
    (available.max(0.0) / pages.max(1) as f32).min(42.0 * scale)
}

#[cfg(test)]
mod tests {
    /// The steering cross: white arms with a dark edge, open in the middle, nothing in the
    /// corners - the same in all four directions.
    #[test]
    fn the_steering_cross_is_a_white_cross_with_a_dark_edge() {
        let n = 34;
        let px = super::steer_cross_pixels(n);
        let at = |x: usize, y: usize| &px[(y * n + x) * 4..(y * n + x) * 4 + 4];
        let c = n / 2;
        assert_eq!(at(c, c)[3], 0, "the middle is open");
        assert_eq!(at(0, 0)[3], 0, "the corners are empty");
        for (x, y) in [(c, 6), (c, n - 7), (6, c), (n - 7, c)] {
            assert_eq!(at(x, y), [255, 255, 255, 255], "arm at {x},{y}");
        }
        assert_eq!(at(c + 2, 6)[..3], [0, 0, 0], "the arm's edge is dark");
    }

    use super::*;

    /// The information bar is broken between its parts into rows that fit the room it has,
    /// as many parts to a row as go in; a part wider than the room alone is a row of its own
    /// (#1164).
    #[test]
    fn the_information_bar_breaks_into_rows_that_fit() {
        let line = ["09:00:07", "0 km/h", "EXT 15 °C / INT 15 °C", "tank 80 %", "0 Passengers", "76 › Krankenhaus", "next: Bauernhof", "−6:52"].join(INFO_SEP);
        let w = |t: &str| t.chars().count() as f32 * 10.0;
        assert_eq!(info_rows(&line, 5000.0, w), vec![line.clone()]);
        let rows = info_rows(&line, 500.0, w);
        assert_eq!(rows, vec![
            format!("09:00:07{INFO_SEP}0 km/h{INFO_SEP}EXT 15 °C / INT 15 °C"),
            format!("tank 80 %{INFO_SEP}0 Passengers"),
            format!("76 › Krankenhaus{INFO_SEP}next: Bauernhof{INFO_SEP}−6:52"),
        ]);
        assert!(rows.iter().all(|r| w(r) <= 500.0));
        assert_eq!(rows.join(INFO_SEP), line);
        assert_eq!(info_rows(&line, 50.0, w).len(), 8);
    }

    #[test]
    fn a_server_notice_is_read_and_fades() {
        let (id, n) = Notice::parse("a1b2 5 warn Service 15/5: departure at 06:07 | depot").unwrap();
        assert_eq!(id, "a1b2");
        assert_eq!((n.kind, n.text.as_str(), n.total, n.left), (NoticeKind::Warn, "Service 15/5: departure at 06:07 | depot", 5.0, 5.0));
        // seconds held to 2 .. 30, an unknown kind is information
        assert_eq!(Notice::parse("x 0.5 info hi").unwrap().1.total, 2.0);
        assert_eq!(Notice::parse("x 600 whatever hi").unwrap().1, Notice { kind: NoticeKind::Info, text: "hi".into(), left: 30.0, total: 30.0 });
        // no text, no seconds, nothing: not a notice
        for bad in ["", "x", "x 5", "x 5 info", "x 5 info   ", "x five info hi", "x NaN info hi"] {
            assert!(Notice::parse(bad).is_none(), "{bad}");
        }
        // fades in, stays, fades out
        let mut n = Notice::parse("x 5 alert hi").unwrap().1;
        assert_eq!(n.alpha(), 0.0);
        n.left = 4.0;
        assert_eq!(n.alpha(), 1.0);
        n.left = 0.25;
        assert!((n.alpha() - 0.5).abs() < 1e-6);
        // three at most: the oldest goes
        let mut list = Vec::new();
        for k in 0..5 {
            push_notice(&mut list, Notice::parse(&format!("{k} 5 info n{k}")).unwrap().1);
        }
        assert_eq!(list.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["n2", "n3", "n4"]);
    }

    #[test]
    fn centre_hud_moves_menu_controls_and_chat_hitboxes_together() {
        let mut ui = Ui::new().unwrap();
        ui.menu_rects.push([20.0, 100.0, 900.0, 140.0]);
        ui.menu_ctl.push(Some([600.0, 110.0, 860.0, 130.0]));
        ui.menu_side.push([30.0, 100.0, 200.0, 140.0]);
        ui.dd_rects.push([650.0, 150.0, 850.0, 180.0]);
        ui.menu_arrows.push(Some([600.0, 640.0, 820.0]));
        ui.chat.rect = [10.0, 800.0, 400.0, 1000.0];
        ui.dd_scroll = Some(([850.0, 150.0, 860.0, 300.0], [844.0, 160.0, 864.0, 200.0]));
        ui.menu_pane_scroll = Some((
            [880.0, 150.0, 890.0, 700.0],
            [874.0, 160.0, 894.0, 220.0],
            40,
            10,
        ));
        ui.shift_hitboxes(1920.0);
        let track = ui.menu_ctl[0].unwrap();
        assert_eq!((2650.0 - track[0]) / (track[2] - track[0]), 0.5);
        assert!(ui.chat.contains(2020.0, 900.0));
        assert!(!ui.chat.contains(100.0, 900.0));
        assert_eq!(ui.dd_rects[0], [2570.0, 150.0, 2770.0, 180.0]);
        assert_eq!(ui.menu_arrows[0], Some([2520.0, 2560.0, 2740.0]));
        assert_eq!(ui.dd_scroll.unwrap().0[0], 2770.0);
        assert_eq!(ui.menu_pane_scroll.unwrap().1[0], 2794.0);
        ui.shift_hitboxes(-1920.0);
        assert_eq!(ui.menu_ctl[0], Some([600.0, 110.0, 860.0, 130.0]));
    }

    #[test]
    fn a_long_chat_line_goes_on_in_rows() {
        // (10 px a character)
        let m = |t: &str| t.chars().count() as f32 * 10.0;
        // fits: one row, as it was
        assert_eq!(wrap_rows("Anton: hello", 200.0, 20.0, &m), ["Anton: hello"]);
        assert_eq!(wrap_rows("", 200.0, 20.0, &m), [""]);
        // between words; the rows after the first are 20 px narrower
        let rows = wrap_rows("Admin (private): take tour 13/1 at 04:47 from the depot", 200.0, 20.0, &m);
        assert_eq!(rows, ["Admin (private):", "take tour 13/1 at", "04:47 from the", "depot"]);
        assert!(rows[0].chars().count() <= 20 && rows[1..].iter().all(|r| r.chars().count() <= 18));
        // nothing lost: the words come back in order
        assert_eq!(rows.join(" "), "Admin (private): take tour 13/1 at 04:47 from the depot");
        // a word longer than a row is cut inside
        let rows = wrap_rows("x: https://example.org/a/very/long/link", 100.0, 20.0, &m);
        assert_eq!(rows, ["x:", "https://", "example.", "org/a/ve", "ry/long/", "link"]);
        assert_eq!(rows.concat(), "x:https://example.org/a/very/long/link");
        // a box narrower than a character still ends (a character a row)
        assert_eq!(wrap_rows("abc", 5.0, 0.0, &m), ["a", "b", "c"]);
    }

    #[test]
    fn vr_settings_sidebar_keeps_back_clear() {
        for scale in [0.5, 1.0, 2.0] {
            for height in [220.0, 440.0] {
                let pages_top = (72.0 + 6.0) * scale;
                let back_top = (height - PAD - 6.0 - 38.0) * scale;
                for pages in [7, 8, 10] {
                    let step = vr_settings_sidebar_step(back_top - 6.0 * scale - pages_top, pages, scale);
                    let last_bottom = pages_top + (pages - 1) as f32 * step + (step - 4.0 * scale).max(1.0);
                    assert!(last_bottom < back_top);
                }
            }
        }
        assert_eq!(vr_settings_sidebar_step(1000.0, 8, 1.0), 42.0);
    }

    #[test]
    fn text_renders_with_an_outline() {
        let f = FontVec::try_from_vec(ROBOTO.to_vec()).unwrap();
        let img = render_text(&f, "Savva: hi", 20.0, [255, 255, 255, 220]);
        assert!(img.width > 40 && img.height > 14);
        // white text and dark outline pixels are both there
        let px: Vec<&[u8]> = img.rgba.chunks(4).collect();
        assert!(px.iter().any(|p| p[3] > 200 && p[0] > 200));
        assert!(px.iter().any(|p| p[3] > 100 && p[0] < 40));
    }

    /// The Esc menu in Chinese, Korean and Thai: the system's fonts, not boxes.
    #[test]
    fn scripts_roboto_lacks_come_from_the_system() {
        let f = FontVec::try_from_vec(ROBOTO.to_vec()).unwrap();
        for t in ["继续", "繼續", "계속", "ดำเนินการต่อ"] {
            if t.chars().next().and_then(omsi_ui::text::fallback_font).is_none() {
                continue;
            }
            for c in t.chars() {
                let g = font_for(&f, c);
                assert!(!std::ptr::eq(g, &f) && g.glyph_id(c).0 != 0, "{t}: {c}");
            }
            let img = render_text(&f, t, 20.0, [255, 255, 255, 220]);
            let ink = img.rgba.chunks(4).filter(|p| p[3] > 128 && p[0] > 128).count();
            assert!(ink > 30, "{t}: {ink}");
        }
    }

    /// Laid out for 1080p: a lower window as it always was, a taller one in proportion.
    #[test]
    fn the_interface_grows_with_tall_windows() {
        assert_eq!(size_factor(900.0, 1.0, 1.0, true), 1.0);
        assert_eq!(size_factor(1080.0, 1.0, 1.0, true), 1.0);
        assert_eq!(size_factor(2160.0, 1.0, 1.0, true), 2.0);
        assert_eq!(size_factor(4320.0, 1.0, 1.0, true), 2.0);
        // (4K at 150 % display scaling is 1440 logical lines: with the screen's scale the
        // same size as at 100 %)
        assert!((1.5 * size_factor(2160.0, 1.5, 1.0, true) - 2.0).abs() < 1e-5);
        assert_eq!(size_factor(1080.0, 1.0, 1.5, true), 1.5);
        assert_eq!(size_factor(2160.0, 1.0, 0.5, true), 1.0);
        // (switched off: the size alone)
        assert_eq!(size_factor(2160.0, 1.0, 1.0, false), 1.0);
        assert_eq!(size_factor(2160.0, 1.0, 1.25, false), 1.25);
    }

    /// A thinned panel's light texts get an outline; dark ones - the highlighted menu line's,
    /// on the solid accent - do not (it came out as a smear round them, like a shadow).
    #[test]
    fn panel_texts_get_an_outline_only_where_it_helps() {
        // (full panels: as asked)
        assert_eq!(outline_for([235, 235, 235, 0], 1.0), 0);
        // (thinned: the light text gets one, the dark text none)
        assert!(outline_for([235, 235, 235, 0], 0.47) > 100);
        assert!(outline_for([140, 140, 140, 0], 0.47) > 100);
        assert_eq!(outline_for([20, 20, 20, 0], 0.47), 0);
        // (texts with an outline of their own keep it)
        assert_eq!(outline_for([255, 255, 255, 235], 0.47), 235);
    }

    /// The opacity's default is the design; below it the backgrounds fade, never quite away.
    #[test]
    fn backgrounds_follow_the_opacity_setting() {
        assert_eq!(backdrop(0.85), 1.0);
        assert!((backdrop(0.425) - 0.5).abs() < 1e-6);
        assert_eq!(backdrop(0.2), 0.3);
        assert!(backdrop(1.0) > 1.0);
    }

    #[test]
    fn the_game_menu_grows_with_the_interface_size_as_far_as_it_fits() {
        // (it ignored the setting: 1.5 drew the menu as 1.0 while the HUD grew)
        assert_eq!(super::menu_scale_for(1.0, 1.0, 1280.0, 720.0, false), 1.0);
        let s = super::menu_scale_for(1.0, 1.5, 1280.0, 720.0, false);
        assert!(s > 1.1 && s < 1.5 && 880.0 * s + 24.0 <= 1280.0 && 600.0 * s <= 720.0 * 0.94 + 0.01, "{s}");
        assert_eq!(super::menu_scale_for(1.0, 1.5, 3840.0, 2160.0, false), 1.5);
        // never smaller than it always was, and the headset keeps its own size
        assert_eq!(super::menu_scale_for(1.0, 1.5, 800.0, 500.0, false), 1.0);
        assert_eq!(super::menu_scale_for(1.0, 1.5, 1920.0, 1080.0, true), 1.0);
    }

    #[test]
    fn destinations_split_into_code_and_name() {
        assert_eq!(super::code_and_name(" 13  BETRIEBSFAHRT"), Some(("13", "BETRIEBSFAHRT")));
        assert_eq!(super::code_and_name("1001  Blanko.tga"), Some(("1001", "Blanko.tga")));
        assert_eq!(super::code_and_name("Route number: 5..."), None);
        assert_eq!(super::code_and_name("Line 76  (1 tours)"), None);
    }

    #[test]
    fn chat_filter_stars_out_swearing() {
        assert_ne!(filter_chat("you are a fucking idiot"), "you are a fucking idiot");
        assert_eq!(filter_chat("next stop Rathaus Spandau"), "next stop Rathaus Spandau");
    }
}
