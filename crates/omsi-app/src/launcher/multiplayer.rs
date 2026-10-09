//! The Multiplayer page: two ways to drive with others.
//!
//! * **Connect by Code** - one player hosts (their next duty opens a session; the code to
//!   give out is shown here and on Sessions), the others paste the code. The game finds the
//!   host at home, across the internet through the routers, and through the host's free
//!   Cloudflare tunnel where routers cannot be passed (`omsi_net::bridge`, `ws`, `tunnel`;
//!   cloudflared is fetched by the game when it is not installed). The page says none of
//!   that: the player gives out a code, that is all.
//! * **Servers** - dedicated servers (`omsi --server`, like Minecraft's): **Favorites** are
//!   addresses the player added; **Public** lists servers that posted to the lobby
//!   (`public = auto` / `1` in `server.cfg`). Joining one turns the Drive page into the
//!   server's: the map, time, date and weather are the server's, the bus and the duty are
//!   the player's, and "Leave Server" goes back.

use super::state::{JoinProto, ServerEntry};
use super::theme::*;
use super::ui::{id_of, ButtonKind};
use super::Launcher;
use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Rect, Weight};
use serde_json::Value;

#[derive(Default)]
pub struct MultiplayerView {
    pub tab: usize,
    /// Under Servers: 0 Favorites, 1 Public.
    pub servers_tab: usize,
    pub add_address: String,
    pub add_name: String,
    pub selected: Option<usize>,
    /// 0 Auto, 1 UDP, 2 WebSocket (see `JoinProto`).
    pub proto: usize,
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Multiplayer", "Drive with friends by a code, or on servers that are always on.");
    let tabs = Rect::new(body.x, body.y, 360.0, 36.0);
    let mut tab = l.mp.tab;
    if l.ui.segmented("mp-tab", tabs, &mut tab, &["Connect by Code", "Servers"]) {
        l.mp.tab = tab;
    }
    let rest = Rect::new(body.x, tabs.bottom() + 18.0, body.w, body.bottom() - tabs.bottom() - 18.0);
    if l.mp.tab == 0 {
        by_code(l, rest);
    } else {
        servers(l, rest);
    }
}

fn by_code(l: &mut Launcher, r: Rect) {
    let col = (r.w - 24.0) / 2.0;
    // host
    let host = Rect::new(r.x, r.y, col, 300.0);
    l.ui.panel(host);
    let mut y = host.y + 18.0;
    l.ui.heading(Rect::new(host.x + 18.0, y, host.w - 36.0, 28.0), "Host a game", Some("wifi_tethering"));
    y += 36.0;
    let hosting = l.state.choice.lan_mode == "host";
    let h = l.ui.paragraph("Your next duty opens a session. Give the code to your friends: it works at home and over the internet.", Vec2::new(host.x + 18.0, y), host.w - 36.0, 12.5, Weight::Regular, TEXT_DIM);
    y += h + 12.0;
    let mut on = hosting;
    if l.ui.toggle("mp-host", Rect::new(host.x + 18.0, y, host.w - 36.0, 32.0), &mut on, "Host my next duty") {
        l.state.choice.lan_mode = if on { "host".into() } else { "off".into() };
        l.state.joined_server = None;
        l.state.touched();
        if on {
            // (the way in for friends behind strict routers: fetched now if it is missing, so
            // that the game does not wait for it)
            std::thread::spawn(omsi_net::tunnel::ensure_cloudflared);
        }
    }
    y += 44.0;
    // the running session's code, when there is one
    let session = l.state.instances.iter().filter(|i| i.running).filter_map(|i| i.lan_status.clone()).find(|s| s.get("role").and_then(Value::as_str) == Some("host"));
    match session {
        Some(s) => {
            let code = s.get("code").and_then(Value::as_str).unwrap_or("").to_string();
            let tunnel = s.get("tunnel").and_then(Value::as_str).map(str::to_string);
            l.ui.label(Rect::new(host.x + 18.0, y, 120.0, 20.0), "Session code");
            y += 22.0;
            let cr = Rect::new(host.x + 18.0, y, host.w - 36.0 - 44.0, ROW);
            l.ui.solid(cr);
            l.ui.p().rounded(cr, RADIUS, FIELD);
            l.ui.text_in(&code, Rect::new(cr.x + 10.0, cr.y, cr.w - 20.0, cr.h), 13.0, Weight::Medium, TEXT, Align::Left);
            if l.ui.icon_button("mp-copy", Vec2::new(cr.right() + 22.0, cr.center().y), 16.0, "content_copy", "Copy the code") {
                l.ui.clipboard_out = Some(code.clone());
                l.state.set_status("Session code copied", false);
            }
            y += ROW + 8.0;
            let line = match tunnel {
                Some(_) => "Friends can join from anywhere".to_string(),
                None => "Getting ready for friends on the internet…".to_string(),
            };
            l.ui.text_in(&line, Rect::new(host.x + 18.0, y, host.w - 36.0, 18.0), 11.5, Weight::Regular, TEXT_FAINT, Align::Left);
        }
        None => {
            let t = if hosting { "Start a duty on the Drive page: the code appears here." } else { "Turn on hosting, then start a duty on the Drive page." };
            l.ui.text_in(t, Rect::new(host.x + 18.0, y, host.w - 36.0, 20.0), 12.5, Weight::Regular, TEXT_SOFT, Align::Left);
        }
    }
    if l.ui.button("mp-go-drive", Rect::new(host.x + 18.0, host.bottom() - 18.0 - ROW, 200.0, ROW), "Go to Drive", Some("directions_bus"), ButtonKind::Normal) {
        l.go(super::Page::Drive);
    }
    // join
    let join = Rect::new(r.x + col + 24.0, r.y, col, 300.0);
    l.ui.panel(join);
    let mut y = join.y + 18.0;
    l.ui.heading(Rect::new(join.x + 18.0, y, join.w - 36.0, 28.0), "Connect by Code", Some("link"));
    y += 36.0;
    let h = l.ui.paragraph("Paste the code your friend's game shows. The map, time and weather are the host's; you choose your bus.", Vec2::new(join.x + 18.0, y), join.w - 36.0, 12.5, Weight::Regular, TEXT_DIM);
    y += h + 12.0;
    let mut a = if l.state.choice.lan_mode == "join" && l.state.joined_server.is_none() { l.state.choice.lan_addr.clone() } else { String::new() };
    if l.ui.text_input("mp-code", Rect::new(join.x + 18.0, y, join.w - 36.0, ROW), &mut a, "OMSI-XXXX-XXXX-…", Some("link")) {
        l.state.choice.lan_addr = a.trim().to_string();
        l.state.choice.lan_mode = if a.trim().is_empty() { "off".into() } else { "join".into() };
        l.state.joined_server = None;
        l.state.touched();
    }
    y += ROW + 8.0;
    if l.state.choice.lan_mode == "join" && l.state.joined_server.is_none() {
        l.state.check_join();
        let (ok, text) = l.state.join.clone();
        l.ui.icon(if ok { "check_circle" } else { "error" }, Vec2::new(join.x + 26.0, y + 9.0), 15.0, if ok { OK } else { DANGER });
        l.ui.paragraph(&text, Vec2::new(join.x + 40.0, y - 2.0), join.w - 58.0, 12.0, Weight::Regular, if ok { TEXT_DIM } else { DANGER });
    }
    let can = l.state.choice.lan_mode == "join" && l.state.joined_server.is_none() && l.state.join.0 && !l.state.choice.lan_addr.is_empty();
    if l.ui.button("mp-join", Rect::new(join.x + 18.0, join.bottom() - 18.0 - ROW, 200.0, ROW), "Choose a bus and join", Some("exit_to_app"), if can { ButtonKind::Primary } else { ButtonKind::Normal }) {
        if can {
            l.go(super::Page::Drive);
        } else {
            l.state.set_status("Paste a session code first", true);
        }
    }
}

fn servers(l: &mut Launcher, r: Rect) {
    let sub = Rect::new(r.x, r.y, 280.0, 36.0);
    let mut tab = l.mp.servers_tab;
    if l.ui.segmented("mp-srv-tab", sub, &mut tab, &["Favorites", "Public"]) {
        l.mp.servers_tab = tab;
        l.mp.selected = None;
    }
    let rest = Rect::new(r.x, sub.bottom() + 14.0, r.w, r.bottom() - sub.bottom() - 14.0);
    if l.mp.servers_tab == 0 {
        favorites(l, rest);
    } else {
        public_list(l, rest);
    }
}

fn favorites(l: &mut Launcher, r: Rect) {
    let bar = Rect::new(r.x, r.y, r.w, ROW);
    let name_w = 200.0;
    let btn_w = 150.0;
    let addr_w = r.w - name_w - btn_w - 24.0;
    let mut a = l.mp.add_address.clone();
    if l.ui.text_input("mp-add-addr", Rect::new(bar.x, bar.y, addr_w, ROW), &mut a, "Server address", Some("dns")) {
        l.mp.add_address = a.trim().to_string();
    }
    let mut n = l.mp.add_name.clone();
    if l.ui.text_input("mp-add-name", Rect::new(bar.x + addr_w + 12.0, bar.y, name_w, ROW), &mut n, "Name (optional)", None) {
        l.mp.add_name = n;
    }
    if l.ui.button("mp-add", Rect::new(bar.right() - btn_w, bar.y, btn_w, ROW), "Add server", Some("add"), ButtonKind::Primary) {
        let addr = l.mp.add_address.trim().to_string();
        if addr.is_empty() {
            l.state.set_status("Type the server's address (an IP, a name or a link)", true);
        } else if l.state.servers.iter().any(|s| s.address.eq_ignore_ascii_case(&addr)) {
            l.state.set_status("That server is in the list already", true);
        } else {
            l.state.servers.push(ServerEntry { name: l.mp.add_name.trim().to_string(), address: addr.clone() });
            l.state.save_servers();
            l.state.ask_server(&addr, 0.0);
            l.mp.add_address.clear();
            l.mp.add_name.clear();
        }
    }
    let list = Rect::new(r.x, bar.bottom() + 16.0, r.w, r.bottom() - bar.bottom() - 16.0 - ROW - 12.0);
    let entries = l.state.servers.clone();
    for e in &entries {
        l.state.ask_server(&e.address, 15.0);
    }
    if entries.is_empty() {
        l.ui.panel(Rect::new(list.x, list.y, list.w, 110.0));
        l.ui.icon("dns", Vec2::new(list.x + 44.0, list.y + 55.0), 30.0, TEXT_FAINT);
        l.ui.paragraph("No servers yet. Add a server by the address its owner gives you.", Vec2::new(list.x + 80.0, list.y + 32.0), list.w - 110.0, 13.0, Weight::Regular, TEXT_DIM);
    }
    let row_h = 84.0;
    let mut join: Option<String> = None;
    let mut remove: Option<usize> = None;
    for (k, e) in entries.iter().enumerate() {
        let rr = Rect::new(list.x, list.y + k as f32 * (row_h + 8.0), list.w, row_h);
        if rr.bottom() > list.bottom() {
            break;
        }
        let sel = l.mp.selected == Some(k);
        if l.ui.row(&format!("srv-{}", e.address), rr, sel) {
            l.mp.selected = Some(k);
        }
        draw_server_row(l, rr, &e.address, &e.name, k, true, &mut join, &mut remove);
    }
    if let Some(k) = remove {
        let gone = l.state.servers.remove(k);
        l.icons.remove(&gone.address);
        l.state.save_servers();
        l.mp.selected = None;
    }
    finish_join(l, join);
    if l.ui.button("mp-refresh", Rect::new(r.x, r.bottom() - ROW, 150.0, ROW), "Refresh", Some("refresh"), ButtonKind::Normal) {
        for e in &entries {
            l.state.ask_server(&e.address, 0.0);
        }
    }
    join_via(l, r);
}

fn public_list(l: &mut Launcher, r: Rect) {
    l.state.fetch_public(60.0);
    let list = Rect::new(r.x, r.y, r.w, r.h - ROW - 12.0);
    let entries = l.state.public_servers.clone();
    for e in &entries {
        l.state.ask_server(&e.url, 15.0);
    }
    // only show posts that answered /status (or are still being asked); drop dead/spam
    let visible: Vec<&omsi_net::lobby::Entry> = entries
        .iter()
        .filter(|e| match l.state.server_info.get(&e.url) {
            Some((_, Err(_))) => false,
            _ => true,
        })
        .collect();
    if visible.is_empty() {
        l.ui.panel(Rect::new(list.x, list.y, list.w, 110.0));
        if l.state.public_loading {
            l.ui.icon("public", Vec2::new(list.x + 44.0, list.y + 55.0), 30.0, TEXT_FAINT);
            l.ui.paragraph("Looking for public servers…", Vec2::new(list.x + 80.0, list.y + 40.0), list.w - 110.0, 13.0, Weight::Regular, TEXT_DIM);
        } else if let Some(err) = l.state.public_error.clone() {
            l.ui.icon("error", Vec2::new(list.x + 44.0, list.y + 55.0), 30.0, DANGER);
            l.ui.paragraph(&format!("Could not load the public list: {err}"), Vec2::new(list.x + 80.0, list.y + 32.0), list.w - 110.0, 13.0, Weight::Regular, DANGER);
        } else {
            l.ui.icon("public", Vec2::new(list.x + 44.0, list.y + 55.0), 30.0, TEXT_FAINT);
            l.ui.paragraph("No public servers online right now. Dedicated servers with public = auto appear here.", Vec2::new(list.x + 80.0, list.y + 32.0), list.w - 110.0, 13.0, Weight::Regular, TEXT_DIM);
        }
    }
    let row_h = 84.0;
    let mut join: Option<String> = None;
    let mut remove: Option<usize> = None;
    let mut fav: Option<(String, String)> = None;
    for (k, e) in visible.iter().enumerate() {
        let rr = Rect::new(list.x, list.y + k as f32 * (row_h + 8.0), list.w, row_h);
        if rr.bottom() > list.bottom() {
            break;
        }
        let sel = l.mp.selected == Some(k);
        if l.ui.row(&format!("psrv-{}", e.url), rr, sel) {
            l.mp.selected = Some(k);
        }
        draw_server_row(l, rr, &e.url, &e.name, k, false, &mut join, &mut remove);
        if l.ui.icon_button(&format!("psrv-fav-{k}"), Vec2::new(rr.right() - 30.0, rr.y + 42.0), 16.0, "star", "Add to Favorites") {
            fav = Some((e.url.clone(), e.name.clone()));
        }
    }
    if let Some((addr, name)) = fav {
        if l.state.servers.iter().any(|s| s.address.eq_ignore_ascii_case(&addr)) {
            l.state.set_status("That server is in Favorites already", false);
        } else {
            l.state.servers.push(ServerEntry { name, address: addr });
            l.state.save_servers();
            l.state.set_status("Added to Favorites", false);
        }
    }
    finish_join(l, join);
    if l.ui.button("mp-pub-refresh", Rect::new(r.x, r.bottom() - ROW, 150.0, ROW), "Refresh", Some("refresh"), ButtonKind::Normal) {
        l.state.fetch_public(0.0);
        for e in &entries {
            l.state.ask_server(&e.url, 0.0);
        }
    }
    join_via(l, r);
    let _ = remove;
}

fn draw_server_row(l: &mut Launcher, rr: Rect, address: &str, name: &str, k: usize, can_remove: bool, join: &mut Option<String>, remove: &mut Option<usize>) {
    let info = l.state.server_info.get(address).map(|x| x.1.clone());
    let ir = Rect::new(rr.x + 12.0, rr.y + 10.0, 64.0, 64.0);
    match l.icons.get(address) {
        Some(tex) => l.ui.image(ir, *tex, 8.0),
        None => {
            l.ui.p().rounded(ir, 8.0, FIELD);
            let letter = name.chars().chain(info.as_ref().and_then(|i| i.as_ref().ok()).map(|i| i.name.clone()).unwrap_or_default().chars()).next().unwrap_or('S').to_uppercase().to_string();
            l.ui.text_in(&letter, ir, 26.0, Weight::Bold, TEXT_DIM, Align::Center);
        }
    }
    if let Some(Ok(i)) = info.as_ref() {
        if !i.icon.is_empty() && !l.icons.contains_key(address) && !l.icons_pending.iter().any(|p| p.0 == address) {
            if let Ok(img) = image::load_from_memory(&i.icon) {
                l.icons_pending.push((address.to_string(), img.to_rgba8()));
            }
        }
    }
    let x = ir.right() + 14.0;
    let title = if !name.is_empty() { name.to_string() } else { info.as_ref().and_then(|i| i.as_ref().ok()).map(|i| i.name.clone()).unwrap_or_else(|| address.to_string()) };
    l.ui.text_in(&title, Rect::new(x, rr.y + 10.0, rr.w - 320.0, 22.0), 15.0, Weight::Medium, TEXT, Align::Left);
    match info.as_ref() {
        Some(Ok(i)) => {
            l.ui.text_in(&i.motd, Rect::new(x, rr.y + 32.0, rr.w - 320.0, 18.0), 12.5, Weight::Regular, TEXT_SOFT, Align::Left);
            let map = std::path::Path::new(&i.map.replace('\\', "/")).parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| i.map.clone());
            l.ui.text_in(&format!("{map} · {} · {}", i.time, if i.weather.is_empty() { "the map's weather" } else { i.weather.as_str() }), Rect::new(x, rr.y + 52.0, rr.w - 320.0, 18.0), 11.5, Weight::Regular, TEXT_FAINT, Align::Left);
            l.ui.text_in(&format!("{}/{}", i.players, i.max_players), Rect::new(rr.right() - 300.0, rr.y + 10.0, 80.0, 22.0), 14.0, Weight::Medium, OK, Align::Right);
            l.ui.icon("signal_cellular_alt", Vec2::new(rr.right() - 206.0, rr.y + 21.0), 16.0, OK);
        }
        Some(Err(err)) => {
            l.ui.text_in(&format!("Can't reach the server: {err}"), Rect::new(x, rr.y + 32.0, rr.w - 320.0, 18.0), 12.0, Weight::Regular, DANGER, Align::Left);
            l.ui.text_in(address, Rect::new(x, rr.y + 52.0, rr.w - 320.0, 18.0), 11.5, Weight::Regular, TEXT_FAINT, Align::Left);
        }
        None => {
            l.ui.text_in("Asking the server…", Rect::new(x, rr.y + 32.0, rr.w - 320.0, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        }
    }
    if l.ui.button(&format!("srv-join-{k}-{}", if can_remove { "f" } else { "p" }), Rect::new(rr.right() - 180.0, rr.y + 24.0, 120.0, 36.0), "Join", Some("exit_to_app"), ButtonKind::Primary) {
        *join = Some(address.to_string());
    }
    if can_remove && l.ui.icon_button(&format!("srv-del-{k}"), Vec2::new(rr.right() - 30.0, rr.y + 42.0), 16.0, "delete", "Remove from the list") {
        *remove = Some(k);
    }
}

fn finish_join(l: &mut Launcher, join: Option<String>) {
    if let Some(a) = join {
        l.state.ask_server(&a, 5.0);
        let proto = [JoinProto::Auto, JoinProto::Udp, JoinProto::WebSocket][l.mp.proto.min(2)];
        l.state.join_server(&a, proto);
        if l.state.joined_server.as_deref() == Some(a.as_str()) {
            l.go(super::Page::Drive);
        }
    }
}

fn join_via(l: &mut Launcher, r: Rect) {
    l.ui.text_in("Join via", Rect::new(r.x + 170.0, r.bottom() - ROW, 70.0, ROW), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
    let mut proto = l.mp.proto;
    if l.ui.segmented("mp-proto", Rect::new(r.x + 244.0, r.bottom() - ROW, 330.0, ROW), &mut proto, &["Auto", "UDP", "WebSocket"]) {
        l.mp.proto = proto;
    }
    let _ = id_of;
}
