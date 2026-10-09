//! Pictures of the plugins' panels as the game draws them, for a look by eye (the tests
//! need a graphics adapter and are left out unless asked for).

use super::*;
use omsi_plugin::ui::panel_from_source;

fn panel(src: &str) -> Panel {
    panel_from_source(src).unwrap()
}

/// A picture of a few panels and notifications over a stand-in for the road, drawn as the
/// game draws them (needs a graphics adapter):
/// `cargo test -p omsi-app --lib plugin_ui::preview::preview -- --ignored`, written to
/// `OMSI_UI_PREVIEW` or `target/ui-preview.png`.
#[test]
#[ignore]
fn preview() {
    let (w, h, k) = (1600u32, 900u32, 1.0f32);
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device");
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
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
    let screen = Vec2::new(w as f32 / k, h as f32 / k);
    let panels = [
        (
            r##"{ anchor = "top_left", x = 16, y = 60, width = 340, accent = "#F47F30", children = {
                { type = "row", children = { { type = "icon", name = "directions_bus", color = "#F47F30" }, { type = "text", text = "Linie 42 · Kurs 3", size = 16, weight = "bold", grow = true }, { type = "badge", text = "+1:20", color = "#C62828" } } },
                { type = "text", text = "Nächster Halt: Grundorf, Krankenhaus Nord (Wendeschleife am Haupteingang)", color = "#C8C8C8" },
                { type = "row", gap = 6, children = { { type = "icon", name = "schedule", size = 16, color = "#8E8E8E" }, { type = "text", text = "ab 08:59", size = 13, color = "#8E8E8E" }, { type = "space", size = 8 }, { type = "icon", name = "group", size = 16, color = "#8E8E8E" }, { type = "text", text = "23 Fahrgäste", size = 13, color = "#8E8E8E" } } },
                { type = "bar", value = 0.62 },
            } }"##,
            None,
        ),
        (
            r##"{ anchor = "bottom_left", x = 16, y = 16, width = 260, children = {
                { type = "row", align = "between", children = { { type = "text", text = "Tagesverdienst", color = "#8E8E8E" }, { type = "text", text = "184,50 €", size = 18, weight = "bold" } } },
                { type = "divider" },
                { type = "row", align = "between", children = { { type = "text", text = "Pünktlichkeit" }, { type = "badge", text = "94 %", color = "#2E7D32" } } },
                { type = "row", align = "between", children = { { type = "text", text = "Fahrstil" }, { type = "badge", text = "B", color = "#F9A825" } } },
            } }"##,
            None,
        ),
        (
            r##"{ anchor = "center", width = 380, padding = 16, gap = 10, clickable = true, children = {
                { type = "text", text = "Schicht beendet", size = 20, weight = "bold", align = "center" },
                { type = "text", text = "Du hast 7 von 7 Fahrten gefahren. Möchtest du die nächste Schicht direkt annehmen?", align = "center", color = "#C8C8C8" },
                { type = "space", size = 4 },
                { type = "row", gap = 8, children = {
                    { type = "button", id = "later", text = "Später", grow = true },
                    { type = "button", id = "take", text = "Annehmen", icon = "check", color = "#E8A030", grow = true },
                } },
                { type = "button", id = "details", text = "Details", icon = "receipt_long" },
            } }"##,
            Some("details"),
        ),
    ];
    let mut first = true;
    let mut images = widgets::Images::default();
    for (src, hot) in panels {
        let p = panel(src);
        let laid = layout(&p, &fonts, 1.0);
        let at = place(&p, laid.w, laid.h, screen);
        let hot = hot.and_then(|id| {
            laid.hits
                .iter()
                .position(|h| h.element.as_deref() == Some(id))
        });
        draw_card(
            &mut gpu,
            &device,
            &queue,
            &mut atlas,
            &fonts,
            &view,
            (w, h),
            k,
            &laid,
            at,
            hot,
            first,
            &mut images,
        );
        first = false;
    }
    let toasts = [
        Toast {
            owner: 1,
            serial: 1,
            text: "Fahrt 3 pünktlich beendet: +12,40 €".into(),
            title: Some("Karriere".into()),
            icon: Some("payments".into()),
            color: Some(Rgba([46, 125, 50, 255])),
            seconds: 5.0,
            age: 1.0,
        },
        Toast {
            owner: 1,
            serial: 2,
            text: "Rote Ampel überfahren".into(),
            title: None,
            icon: Some("warning".into()),
            color: Some(Rgba([198, 40, 40, 255])),
            seconds: 5.0,
            age: 1.0,
        },
        Toast {
            owner: 1,
            serial: 3,
            text: "Schichtbeginn in 10 Minuten am Betriebshof".into(),
            title: None,
            icon: None,
            color: None,
            seconds: 5.0,
            age: 1.0,
        },
    ];
    let mut y = TOAST_TOP;
    for t in &toasts {
        let laid = layout(&toast_panel(t), &fonts, 1.0);
        draw_card(
            &mut gpu,
            &device,
            &queue,
            &mut atlas,
            &fonts,
            &view,
            (w, h),
            k,
            &laid,
            Vec2::new(screen.x - 16.0 - laid.w, y),
            None,
            false,
            &mut images,
        );
        y += laid.h + TOAST_GAP;
    }
    let out = omsi_cfg::flags::OMSI_UI_PREVIEW.live_var()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-preview.png")
        });
    save(&device, &queue, &target, w, h, &out.to_string_lossy());
}

/// Read a preview back, lay it over a stand-in for the road (sky, buildings, the street)
/// and write it as a PNG.
pub(crate) fn save(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture, w: u32, h: u32, out: &str) {
    // read back and laid over a stand-in for the road: sky, buildings, the street
    let stride = (w * 4).div_ceil(256) * 256;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (stride * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).ok();
    let data = buf.slice(..).get_mapped_range();
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = y as f32 / h as f32;
            let bg: [f32; 3] = if t < 0.45 {
                [120.0 + 80.0 * t, 165.0 + 60.0 * t, 225.0]
            } else if t < 0.7 && (x / 140) % 3 != 0 {
                [150.0 + (x % 140) as f32 * 0.3, 140.0, 128.0]
            } else if t < 0.7 {
                [205.0, 200.0, 190.0]
            } else {
                [70.0, 72.0, 76.0]
            };
            let i = (y * stride + x * 4) as usize;
            let a = data[i + 3] as f32 / 255.0;
            let px = |c: usize| (data[i + c] as f32 + bg[c] * (1.0 - a)).min(255.0) as u8;
            img.put_pixel(x, y, image::Rgba([px(0), px(1), px(2), 255]));
        }
    }
    img.save(out).unwrap();
    println!("wrote {out}");
}
