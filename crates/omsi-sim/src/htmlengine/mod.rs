//! The built-in backend of `[htmltexture]`: a small HTML/CSS/JavaScript engine that needs no
//! browser and no extra dependency, so it runs the same on every desktop and on Android.
//!
//! What a page may use:
//! * HTML: nested elements, `<style>`, `<script>`, inline `style=""`, `id`, `class`, entities,
//!   `<img src width height>` (bmp, dds, tga, jpg, png; looked up next to the page, see `images`).
//! * CSS: selectors `tag`, `#id`, `.class`, `*`, compounds (`div.a#b`), descendant chains
//!   and `,` lists; `color`, `background(-color)`, `font-size`, `font-weight`, `text-align`,
//!   `line-height`, `margin*`, `padding*`, `width`, `height`, `display` (`none`, `inline`),
//!   `visibility`, `border-radius`, `background-image: url()`, `background-size`,
//!   `background-repeat`, `background-position` and the `background` shorthand. Units: `px`, `%`, `em`, `rem`, `pt`, `vw`, `vh`.
//!   Layout is block flow with wrapped inline text (no floats, no flexbox).
//! * JavaScript (ES5 plus arrow functions): `var/let/const`, functions, `if/for/while`,
//!   objects, arrays, `Math.*`, `parseInt/parseFloat/String/Number`,
//!   `document.getElementById/querySelector/body`, `element.textContent/innerText/className/id`,
//!   `element.style.*`, `element.setAttribute`.
//!
//! The page talks to the vehicle through `window.omsi`:
//! * the host calls `window.omsi.update({ num: {name: value}, str: {name: "text"} })` with the
//!   script variables that changed (all of them on the first call);
//! * the page calls `window.omsi.setVar(name, value)` to write a script variable back;
//! * the page calls `window.omsi.trigger(name)` to press a trigger of the vehicle's scripts
//!   (what a button in the cab does).
//! * `window.omsi.vehicle` is a normalised snapshot of the vehicle with fixed names for every
//!   bus: `engine.running/rpm`, `battery.on`, `doors.list[i].isOpen`, `lights.*`, `brakes.*` and
//!   more (see [`crate::vehicle_api`] for the full list). A signal the bus lacks is `null` or
//!   `false`. It is current whenever `update` runs and can be read from timers too.
//! * `window.omsi.vars.num` / `.str` hold the latest value of every variable of the bus's own
//!   variable list under its lower-case name, and `window.omsi.getVar(name)` reads one in any
//!   letter case (`undefined` when the bus has none). `update`'s argument carries `vehicle`
//!   and `vars` as well as the changed `num` / `str`.
//!
//! A page can be operated: the app passes presses, releases and moves of the pointer on the
//! texture ([`HtmlRenderer::pointer`]). They become `pointerdown`/`mousedown`, `pointerup`/
//! `mouseup`, `click` and `mousemove` on the element under the pointer, which bubble up through
//! its parents. Listeners: the `onclick="..."` attribute (`event` is the event object), the
//! `element.onclick = f` property and `element.addEventListener("click", f)`; the event has
//! `type`, `x`, `y` (texture pixels), `target`, `stopPropagation()` and `preventDefault()`.
//! * `display:inline-block` (and `<button>`, which has a default look) lays boxes out in rows that
//!   wrap; a box without a `width` is as wide as its content. Use it for key pads and lists.
//! * `window.omsi.time` (`hour`, `minute`, `second`, `asString` = `HH:MM:SS`), `window.omsi.date`
//!   (`day`, `month`, `year`, `asString`: `DD.MM.YYYY`, `MM/DD/YYYY` for `en`) show the simulation
//!   clock; `window.omsi.locale` is the interface language (`en`, `de`, ...).
//! * `window.omsi.vehicle.route` holds line, destination sign, the stops with their planned
//!   times, the stop the bus is at and the last stop (see [`crate::vehicle_api`]).
//!   `window.omsi.depot` lists the depot file's `lines[]` (each with its `routes[]`), `routes[]`
//!   and `destinations[]`; the page sets the IBIS with `omsi.setRoute(index)` (line, route and
//!   destination of `depot.routes[index]`), `omsi.setLine(text)` (the first route of that
//!   line) and `omsi.setDestination(index)` (only the destination sign, `depot.destinations`).
//!   `omsi.setNextStop(index)` moves the duty on to stop `index` of its trip (`route.stops[index]`;
//!   the stops before it are skipped; going back to an earlier stop makes the stops from there on due again).
//! * `window.omsi.getDepartures(stop)` returns the departures of the next two hours at the bus stop
//!   with that name (at most 20, soonest first) as `{ line, destination, time }`, `time` a
//!   timestamp on the scale of `window.omsi.timestamp`. The first call for a stop returns `[]`; the
//!   host fills `window.omsi.departures` (see [`crate::vehicle_api::departures`]) and calls `update`.
//! * More JavaScript for such pages: `setTimeout`/`setInterval`/`clear*`, `classList`,
//!   `createElement`/`appendChild`/`removeChild`/`remove`, `innerHTML` with markup, `getAttribute`,
//!   `parentNode`, `Object.keys`, `Array.forEach/map/filter/indexOf/includes/pop/shift/slice`,
//!   `String.split/replace`. Timers run when the page is drawn and use real time.
//!
//! # Layout of this module
//! * `dom`: HTML parser and CSS rule parser. `style`: computed style. `layout`: block flow and
//!   inline text. `canvas`: pixels and painter.
//! * `js`: the JavaScript subset (`lexer`, `parser`, `interp`).
//! * `renderer`: [`EngineRenderer`], the [`HtmlRenderer`] backend that ties them together.

use crate::htmltex::{HtmlRenderer, PointerKind};
use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont, VariableFont};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod api;
mod canvas;
mod dom;
mod images;
mod js;
mod layout;
mod renderer;
mod style;

use api::*;
use canvas::*;
use dom::*;
use images::*;
use js::*;
use layout::*;
use style::*;

pub use renderer::EngineRenderer;

pub(crate) const ROBOTO: &[u8] = include_bytes!("../../../../assets/fonts/Roboto-VariableFont_wdth,wght.ttf");
thread_local! {
    /// The regular and the bold face, parsed once per thread instead of once per frame.
    static FONTS: Option<(FontRef<'static>, FontRef<'static>)> = FontRef::try_from_slice(ROBOTO).ok().map(|reg| {
        let mut bold = reg.clone();
        bold.set_variation(b"wght", 700.0);
        (reg, bold)
    });
}

/// Run `f` with the regular and the bold face; `None` when the font does not parse.
pub(crate) fn with_fonts<R>(f: impl FnOnce(&FontRef<'static>, &FontRef<'static>) -> R) -> Option<R> {
    FONTS.with(|p| p.as_ref().map(|(reg, bold)| f(reg, bold)))
}

/// The bytes of the face for the characters Roboto lacks (Hangul, CJK ...): the file in
/// `OMSI_HTML_FALLBACK_FONT` (and `..._BOLD`), else Malgun Gothic from the Windows fonts or
/// AppleGothic on macOS.
/// Read once per process; `None` when there is none.
fn fallback_bytes() -> Option<&'static (Vec<u8>, Vec<u8>)> {
    static BYTES: std::sync::OnceLock<Option<(Vec<u8>, Vec<u8>)>> = std::sync::OnceLock::new();
    BYTES
        .get_or_init(|| {
            let read = |p: &std::path::Path| std::fs::read(p).ok();
            if let Some(reg) = omsi_cfg::flags::OMSI_HTML_FALLBACK_FONT.os().and_then(|p| read(p.as_ref())) {
                let bold = omsi_cfg::flags::OMSI_HTML_FALLBACK_FONT_BOLD
                    .os()
                    .and_then(|p| read(p.as_ref()))
                    .unwrap_or_else(|| reg.clone());
                return Some((reg, bold));
            }
            if cfg!(target_os = "macos") {
                let reg = read(std::path::Path::new("/System/Library/Fonts/Supplemental/AppleGothic.ttf"))?;
                return Some((reg.clone(), reg));
            }
            let dir = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
            let reg = read(&dir.join("malgun.ttf"))?;
            let bold = read(&dir.join("malgunbd.ttf")).unwrap_or_else(|| reg.clone());
            Some((reg, bold))
        })
        .as_ref()
}

thread_local! {
    static FALLBACK: Option<(FontRef<'static>, FontRef<'static>)> = fallback_bytes().and_then(|(reg, bold)| {
        Some((FontRef::try_from_slice(reg).ok()?, FontRef::try_from_slice(bold).ok()?))
    });
}

/// The face that draws `ch`, its glyph and which face it is (0 = `font`, 1 = the fallback):
/// `font` itself, or the fallback face when `font` has no glyph for it.
pub(crate) fn face_for<R>(font: &FontRef<'static>, bold: bool, ch: char, f: impl FnOnce(&FontRef<'static>, ab_glyph::GlyphId, u8) -> R) -> R {
    let id = font.glyph_id(ch);
    if id.0 != 0 || ch.is_ascii() {
        return f(font, id, 0);
    }
    FALLBACK.with(|fb| match fb {
        Some((reg, b)) => {
            let face = if bold { b } else { reg };
            let fid = face.glyph_id(ch);
            if fid.0 != 0 { f(face, fid, 1) } else { f(font, id, 0) }
        }
        None => f(font, id, 0),
    })
}

pub(crate) const STEP_LIMIT: u32 = 400_000;
pub(crate) const DEPTH_LIMIT: u32 = 48;

#[cfg(test)]
mod tests;