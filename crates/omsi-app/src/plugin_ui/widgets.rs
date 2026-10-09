//! The panels' controls and pictures (`omsi.ui` version 2): checkboxes, sliders, text
//! fields, tabs, charts, tables and images - their sizes and what they are drawn from.

use super::{color, ink_on, Hit, HitKind, Item, Layout, ACCENT, BUTTON, DIVIDER, INK, LINE, TRACK};
use glam::Vec2;
use hashbrown::HashMap;
use omsi_plugin::ui::{ChartStyle, Element, Kind};
use omsi_ui::{Color, Gpu, Rect, Weight};
use std::path::{Path, PathBuf};

const CHECK: f32 = 18.0;
const CHECK_GAP: f32 = 8.0;
const CONTROL_PX: f32 = 14.0;
const SLIDER_H: f32 = 24.0;
const SLIDER_W: f32 = 140.0;
const KNOB: f32 = 14.0;
const FIELD_H: f32 = 30.0;
const FIELD_W: f32 = 180.0;
const FIELD_PAD: f32 = 8.0;
const TABS_H: f32 = 30.0;
const CHART_W: f32 = 120.0;
/// Largest picture side a plugin's image is drawn from (bigger ones are scaled down).
const IMAGE_MAX: u32 = 1024;
const GREY: Color = Color::rgba(142, 142, 142, 1.0);

impl Layout<'_> {
    pub(super) fn widget_natural(&self, e: &Element) -> f32 {
        match &e.kind {
            Kind::Image { width, .. } => *width,
            Kind::Checkbox { text, .. } => (CHECK + CHECK_GAP + self.width(text, CONTROL_PX, Weight::Regular)).ceil(),
            Kind::Slider { .. } => SLIDER_W,
            Kind::Input { .. } => FIELD_W,
            Kind::Tabs { tabs, .. } => tabs.iter().map(|t| self.width(t, CONTROL_PX, Weight::Medium) + 24.0).sum::<f32>().ceil(),
            Kind::Chart { .. } => CHART_W,
            Kind::Table { columns, size, .. } => columns.iter().map(|c| self.width(c, *size, Weight::Bold) + 12.0).sum::<f32>().ceil(),
            _ => 0.0,
        }
    }

    pub(super) fn widget_height(&self, e: &Element, w: f32) -> f32 {
        match &e.kind {
            // (a picture wider than its room keeps its shape)
            Kind::Image { width, height, .. } => height * (w / width).min(1.0),
            Kind::Checkbox { .. } => CHECK.max(CONTROL_PX * LINE),
            Kind::Slider { .. } => SLIDER_H,
            Kind::Input { .. } => FIELD_H,
            Kind::Tabs { .. } => TABS_H,
            Kind::Chart { height, .. } => *height,
            Kind::Table { columns, rows, size, .. } => {
                let head = if columns.is_empty() { 0.0 } else { size * LINE + 3.0 };
                head + rows.len() as f32 * size * LINE
            }
            _ => 0.0,
        }
    }

    fn hit(&mut self, r: Rect, e: &Element, kind: HitKind) -> usize {
        self.out.hits.push(Hit { r, element: e.id.clone(), kind });
        self.out.hits.len() - 1
    }

    pub(super) fn widget_emit(&mut self, e: &Element, r: Rect) {
        let accent = e.color.map(color).unwrap_or(ACCENT);
        match &e.kind {
            Kind::Image { path, width, height } => {
                let k = (r.w / width).min(1.0);
                let (w, h) = (width * k, height * k);
                if let Some(path) = path {
                    self.out.items.push(Item::Image { r: Rect::new(r.x, r.y + (r.h - h) * 0.5, w, h), path: path.clone() });
                }
            }
            Kind::Checkbox { text, checked } => {
                let hot = self.hit(r, e, HitKind::Click);
                let b = Rect::new(r.x, r.y + (r.h - CHECK) * 0.5, CHECK, CHECK);
                self.rounded(b, 4.0, if *checked { accent } else { TRACK }, Some(hot));
                if *checked {
                    self.out.items.push(Item::Icon { name: "check".into(), center: Vec2::new(b.x + CHECK * 0.5, b.y + CHECK * 0.5), size: 16.0, color: ink_on(accent) });
                }
                let x = b.right() + CHECK_GAP;
                let t = self.fit(text, CONTROL_PX, Weight::Regular, (r.right() - x).max(0.0));
                let y = self.baseline(r.y, r.h, CONTROL_PX, Weight::Regular);
                self.text(t, CONTROL_PX, Weight::Regular, Vec2::new(x, y), INK);
            }
            Kind::Slider { value, min, max, .. } => {
                // (the knob stays inside the track's ends)
                let track = Rect::new(r.x + KNOB * 0.5, r.y + r.h * 0.5 - 2.0, (r.w - KNOB).max(1.0), 4.0);
                let hot = self.hit(Rect::new(track.x, r.y, track.w, r.h), e, HitKind::Slider);
                let f = if max > min { ((value - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
                self.rounded(track, 2.0, TRACK, None);
                if f > 0.0 {
                    self.rounded(Rect::new(track.x, track.y, track.w * f, track.h), 2.0, accent, None);
                }
                let cx = track.x + track.w * f;
                self.rounded(Rect::new(cx - KNOB * 0.5, r.y + (r.h - KNOB) * 0.5, KNOB, KNOB), KNOB * 0.5, INK, Some(hot));
            }
            Kind::Input { text, placeholder, .. } => {
                let b = Rect::new(r.x, r.y + (r.h - FIELD_H) * 0.5, r.w, FIELD_H);
                let hot = self.hit(b, e, HitKind::Click);
                self.rounded(b, 6.0, BUTTON, Some(hot));
                let typing = self.typing.is_some_and(|t| e.id.as_deref() == Some(t));
                let room = (b.w - 2.0 * FIELD_PAD - 2.0).max(0.0);
                let y = self.baseline(b.y, b.h, CONTROL_PX, Weight::Regular);
                let shown = if text.is_empty() { placeholder } else { text };
                // (a long text shows its end while it is typed)
                let mut t = self.fit(shown, CONTROL_PX, Weight::Regular, room);
                if typing && t != *shown {
                    let chars: Vec<char> = shown.chars().collect();
                    let mut start = 0;
                    while start < chars.len() && self.width(&chars[start..].iter().collect::<String>(), CONTROL_PX, Weight::Regular) > room {
                        start += 1;
                    }
                    t = chars[start..].iter().collect();
                }
                let tw = if text.is_empty() { 0.0 } else { self.width(&t, CONTROL_PX, Weight::Regular) };
                self.text(t, CONTROL_PX, Weight::Regular, Vec2::new(b.x + FIELD_PAD, y), if text.is_empty() { GREY } else { INK });
                if typing {
                    self.out.items.push(Item::Rect { r: Rect::new(b.x + FIELD_PAD + tw + 1.0, b.y + 7.0, 1.5, b.h - 14.0), color: accent });
                    self.out.items.push(Item::Rect { r: Rect::new(b.x + 4.0, b.bottom() - 2.0, b.w - 8.0, 2.0), color: accent });
                }
            }
            Kind::Tabs { tabs, selected } => {
                let b = Rect::new(r.x, r.y + (r.h - TABS_H) * 0.5, r.w, TABS_H);
                self.hit(b, e, HitKind::Tabs);
                self.rounded(b, 8.0, TRACK, None);
                let n = tabs.len().max(1) as f32;
                let w = b.w / n;
                for (i, t) in tabs.iter().enumerate() {
                    let cell = Rect::new(b.x + w * i as f32, b.y, w, b.h);
                    let on = i == *selected;
                    if on {
                        self.rounded(Rect::new(cell.x + 2.0, cell.y + 2.0, cell.w - 4.0, cell.h - 4.0), 6.0, accent, None);
                    }
                    let t = self.fit(t, CONTROL_PX, Weight::Medium, (cell.w - 12.0).max(0.0));
                    let tw = self.width(&t, CONTROL_PX, Weight::Medium);
                    let y = self.baseline(cell.y, cell.h, CONTROL_PX, Weight::Medium);
                    self.text(t, CONTROL_PX, Weight::Medium, Vec2::new(cell.x + (cell.w - tw) * 0.5, y), if on { ink_on(accent) } else { INK });
                }
            }
            Kind::Chart { values, min, max, style, fill, .. } => self.chart(r, values, *min, *max, *style, *fill, accent),
            Kind::Table { columns, rows, widths, size } => self.table(r, columns, rows, widths, *size, e.color.map(color)),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn chart(&mut self, r: Rect, values: &[f32], min: Option<f32>, max: Option<f32>, style: ChartStyle, fill: bool, c: Color) {
        self.out.items.push(Item::Rect { r, color: DIVIDER.alpha(0.6) });
        if values.is_empty() {
            return;
        }
        // (bars stand on 0 unless a value is below it; a line spans its values)
        let least = values.iter().copied().fold(f32::INFINITY, f32::min);
        let lo = min.unwrap_or(if style == ChartStyle::Bars { least.min(0.0) } else { least });
        let mut hi = max.unwrap_or_else(|| values.iter().copied().fold(f32::NEG_INFINITY, f32::max));
        if hi <= lo {
            hi = lo + 1.0;
        }
        let y_of = |v: f32| r.bottom() - ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * (r.h - 2.0) - 1.0;
        match style {
            ChartStyle::Bars => {
                let w = r.w / values.len() as f32;
                for (i, v) in values.iter().enumerate() {
                    let top = y_of(*v);
                    let bar = Rect::new(r.x + w * i as f32 + (w * 0.15).min(1.0), top, (w * 0.7).max(1.0).max(w - 2.0), r.bottom() - top);
                    self.out.items.push(Item::Rect { r: bar, color: c });
                }
            }
            ChartStyle::Line => {
                let n = values.len();
                let x_of = |i: usize| if n == 1 { r.x + r.w * 0.5 } else { r.x + r.w * i as f32 / (n - 1) as f32 };
                for i in 1..n {
                    let (a, b) = (Vec2::new(x_of(i - 1), y_of(values[i - 1])), Vec2::new(x_of(i), y_of(values[i])));
                    if fill {
                        self.out.items.push(Item::Poly { pts: vec![a, b, Vec2::new(b.x, r.bottom()), Vec2::new(a.x, r.bottom())], color: c.alpha(0.25) });
                    }
                    self.out.items.push(Item::Line { a, b, w: 2.0, color: c });
                }
            }
        }
    }

    fn table(&mut self, r: Rect, columns: &[String], rows: &[Vec<String>], widths: &[f32], size: f32, c: Option<Color>) {
        let n = columns.len().max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
        let shares: Vec<f32> = (0..n).map(|i| widths.get(i).copied().filter(|w| *w > 0.0).unwrap_or(1.0)).collect();
        let total: f32 = shares.iter().sum();
        let xs: Vec<(f32, f32)> = shares.iter().scan(r.x, |x, s| {
            let w = r.w * s / total;
            let cell = (*x, w);
            *x += w;
            Some(cell)
        }).collect();
        let lh = size * LINE;
        let mut y = r.y;
        let line = |l: &mut Self, cells: &[String], y: f32, weight: Weight, ink: Color| {
            for (k, cell) in cells.iter().enumerate().take(n) {
                let (x, w) = xs[k];
                let t = l.fit(cell, size, weight, (w - 6.0).max(0.0));
                let base = l.baseline(y, lh, size, weight);
                l.text(t, size, weight, Vec2::new(x, base), ink);
            }
        };
        if !columns.is_empty() {
            line(self, columns, y, Weight::Bold, GREY);
            y += lh + 1.0;
            self.out.items.push(Item::Rect { r: Rect::new(r.x, y, r.w, 1.0), color: DIVIDER });
            y += 2.0;
        }
        for row in rows {
            line(self, row, y, Weight::Regular, c.unwrap_or(INK));
            y += lh;
        }
    }
}

/// The plugins' pictures, read once and kept on the graphics card (by file).
#[derive(Default)]
pub(crate) struct Images {
    loaded: HashMap<PathBuf, Option<usize>>,
}

impl Images {
    /// The picture's texture, read from its file the first time (None: it cannot be read).
    pub(crate) fn get(&mut self, gpu: &mut Gpu, device: &wgpu::Device, queue: &wgpu::Queue, path: &Path) -> Option<usize> {
        if let Some(t) = self.loaded.get(path) {
            return *t;
        }
        let tex = match image::open(path) {
            Ok(img) => {
                let img = if img.width().max(img.height()) > IMAGE_MAX { img.thumbnail(IMAGE_MAX, IMAGE_MAX) } else { img };
                let rgba = img.to_rgba8();
                Some(gpu.add_image(device, queue, rgba.width(), rgba.height(), rgba.as_raw()))
            }
            Err(e) => {
                log::warn!("plugin picture {}: {e}", path.display());
                None
            }
        };
        self.loaded.insert(path.to_path_buf(), tex);
        tex
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use omsi_plugin::ui::panel_from_source;

    #[test]
    fn controls_take_their_room_and_their_hits() {
        let fonts = Fonts::new();
        let p = panel_from_source(
            r##"{ width = 320, children = {
                { type = "checkbox", id = "c", text = "Rain", checked = true },
                { type = "slider", id = "s", value = 5, min = 0, max = 10 },
                { type = "input", id = "i", placeholder = "Name" },
                { type = "tabs", id = "t", tabs = { "A", "B" }, selected = 2 },
                { type = "chart", values = { 1, 3, 2 }, height = 40, fill = true },
                { type = "chart", values = { 1, 3, 2 }, style = "bars" },
                { type = "table", columns = { "Stop", "Time" }, rows = { { "Zoo", "12:01" }, { "Markt", "12:05" } } },
                { type = "image", src = "none.png", width = 64, height = 32 },
            } }"##,
        )
        .unwrap();
        let l = layout_typing(&p, &fonts, 1.0, Some("i"));
        let kinds: Vec<(Option<&str>, HitKind)> = l.hits.iter().map(|h| (h.element.as_deref(), h.kind)).collect();
        assert_eq!(kinds, [(Some("c"), HitKind::Click), (Some("s"), HitKind::Slider), (Some("i"), HitKind::Click), (Some("t"), HitKind::Tabs)]);
        // the chart's line in two segments with their fill, the bars, the table's header line
        assert_eq!(l.items.iter().filter(|i| matches!(i, Item::Line { .. })).count(), 2);
        assert_eq!(l.items.iter().filter(|i| matches!(i, Item::Poly { .. })).count(), 2);
        assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "Markt")));
        // (an image that is not there takes its room and draws nothing)
        assert!(!l.items.iter().any(|i| matches!(i, Item::Image { .. })));
        assert!(l.h > 300.0, "{}", l.h);
    }

    /// A picture of the controls as the game draws them (needs a graphics adapter):
    /// `cargo test -p omsi-app --lib widgets::tests::preview_controls -- --ignored`, written
    /// to `OMSI_UI_PREVIEW` or `target/ui-controls.png`.
    #[test]
    #[ignore]
    fn preview_controls() {
        let (w, h, k) = (800u32, 620u32, 1.0f32);
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut gpu = Gpu::new(&device, format, 1, 1024);
        let fonts = Fonts::new();
        let mut atlas = Atlas::new(1024);
        atlas.begin_frame();
        // (a plugin's picture: a round badge, written for the test)
        let icon = std::env::temp_dir().join(format!("omsi-ui-badge-{}.png", std::process::id()));
        image::RgbaImage::from_fn(64, 64, |x, y| {
            let d = ((x as f32 - 31.5).powi(2) + (y as f32 - 31.5).powi(2)).sqrt();
            if d > 31.0 { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([244, 127 + (y * 2) as u8, 48, 255]) }
        })
        .save(&icon)
        .unwrap();
        let src = format!(
            r##"{{ width = 380, accent = "#F47F30", children = {{
                {{ type = "row", children = {{ {{ type = "image", src = "{}", width = 40, height = 40 }}, {{ type = "text", text = "Dispatcher", size = 18, weight = "bold", grow = true }} }} }},
                {{ type = "tabs", id = "t", tabs = {{ "Duty", "Stats", "Settings" }}, selected = 2 }},
                {{ type = "checkbox", id = "c", text = "Announce the stops", checked = true }},
                {{ type = "checkbox", id = "c2", text = "Warn before a red light" }},
                {{ type = "row", children = {{ {{ type = "text", text = "Volume" }}, {{ type = "slider", id = "s", value = 7, min = 0, max = 10, grow = true }} }} }},
                {{ type = "input", id = "i", text = "Rathaus Spandau" }},
                {{ type = "input", id = "j", placeholder = "Your name" }},
                {{ type = "chart", values = {{ 40, 42, 38, 45, 52, 49, 55, 61, 58, 50 }}, height = 50, fill = true }},
                {{ type = "chart", values = {{ 3, 5, 2, 7, 4, 6 }}, height = 30, style = "bars", color = "#2E7D32" }},
                {{ type = "table", columns = {{ "Stop", "Due", "Delay" }}, widths = {{ 2, 1, 1 }}, rows = {{ {{ "Zoo", "12:01", "+0:30" }}, {{ "Markt", "12:05", "+1:10" }}, {{ "Rathaus", "12:14", "-" }} }} }},
            }} }}"##,
            icon.to_string_lossy().replace('\\', "/")
        );
        let mut p = panel_from_source(&src).unwrap();
        // (the picture by its path: a panel of the tests has no plugin folder)
        if let Kind::Row { children, .. } = &mut p.children[0].kind {
            children[0].kind = Kind::Image { path: Some(icon.clone()), width: 40.0, height: 40.0 };
        }
        let laid = layout_typing(&p, &fonts, 1.0, Some("i"));
        let mut images = super::Images::default();
        draw_card(&mut gpu, &device, &queue, &mut atlas, &fonts, &view, (w, h), k, &laid, Vec2::new(30.0, 20.0), Some(1), true, &mut images);
        let out = omsi_cfg::flags::OMSI_UI_PREVIEW.live_var().unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/ui-controls.png").to_string());
        super::super::preview::save(&device, &queue, &target, w, h, &out);
    }
}
