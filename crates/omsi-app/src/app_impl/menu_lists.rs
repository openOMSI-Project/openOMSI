//! The game menu's lists (`game_lists`): the chooser, the drop-downs, the settings tabs,
//! the tours' pane and the text fields typed into.

use super::*;

impl App {
    /// Show one of the menu's lists in the chooser (see `game_lists`).
    pub(crate) fn open_list(&mut self, kind: crate::game_lists::ListKind) {
        crate::game_lists::forget_page_titles();
        // (a search belongs to its list; `chooser_pick` has taken the kind when it goes on)
        if self.menus.list_kind.as_ref().is_some_and(|k| *k != kind) {
            self.forget_search();
        }
        self.menus.dropdown = None;
        self.menus.admin_list = Some(crate::game_lists::items(self, &kind));
        self.menus.list_kind = Some(kind);
        // (on its first line, not on a heading, nor on the search line above it: Enter
        // picks what it picked before the lists could be searched)
        let search = self.menus.admin_list.as_ref().and_then(|l| l.first()).is_some_and(|l| l.1 == "search");
        self.menus.chooser = Some(if self.is_heading(0) || search { self.chooser_next(0, 1) } else { 0 });
    }

    /// Line `k` of the list shown heads the lines under it (`game_lists::HEADING`).
    fn is_heading(&self, k: usize) -> bool {
        self.menus.admin_list.as_ref().and_then(|l| l.get(k)).is_some_and(|l| l.1 == crate::game_lists::HEADING)
    }

    /// The line `step` lines on from `sel` (round the list; `n - 1` is one back), over the
    /// headings.
    fn chooser_next(&self, sel: usize, step: usize) -> usize {
        let n = self.menus.admin_list.as_ref().unwrap_or(&self.menus.vehicle_list).len().max(1);
        let mut k = sel;
        for _ in 0..n {
            k = (k + step) % n;
            if !self.is_heading(k) {
                break;
            }
        }
        k
    }

    /// Left or Right on line `k` of a list, or a click on the arrows round its value: its
    /// setting one step down (`-`) or up (`+`), see `game_lists::ADJUST`; other lines stay.
    pub(crate) fn chooser_adjust(&mut self, k: usize, dir: &str) {
        let Some(action) = self.menus.admin_list.as_ref().and_then(|l| l.get(k)).and_then(|l| l.1.strip_suffix(crate::game_lists::ADJUST)).map(|a| format!("{a} {dir}")) else { return };
        // (run as a pick of the line, with the step in place of the mark)
        if let Some(l) = self.menus.admin_list.as_mut().and_then(|l| l.get_mut(k)) {
            l.1 = action;
        }
        self.chooser_pick(k);
    }

    fn icao_edit_key(&mut self,code:KeyCode){
        match code{
            KeyCode::Escape=>{self.menus.menu_edit=None;self.menus.menu_edit_icao=false;if let Some(w)=self.window.as_ref(){w.set_ime_allowed(false);}},
            KeyCode::Backspace|KeyCode::Delete=>{if let Some(d)=self.menus.menu_edit.as_mut(){d.pop();}},
            KeyCode::Enter|KeyCode::NumpadEnter=>{self.apply_icao_edit();return;},
            _=>{}
        }
        self.refresh_list();
    }
    pub(crate) fn icao_edit_text(&mut self,text:&str){
        if !self.menus.menu_edit_icao{return}
        if let Some(d)=self.menus.menu_edit.as_mut(){
            for c in text.chars().filter(|c|c.is_ascii_alphabetic()){
                if d.len()>=4{break} d.push(c.to_ascii_uppercase());
            }
        }
        self.refresh_list();
    }
    pub(crate) fn start_icao_edit(&mut self){
        self.menus.menu_edit=Some(String::new()); self.menus.menu_edit_icao=true;
        if let Some(w)=self.window.as_ref(){w.set_ime_allowed(true);}
    }
    pub(crate) fn apply_icao_edit(&mut self){
        let code=self.menus.menu_edit.take().unwrap_or_default().trim().to_ascii_uppercase();
        self.menus.menu_edit_icao=false;
        if let Some(w)=self.window.as_ref(){w.set_ime_allowed(false);}
        if code.len()==4&&code.chars().all(|c|c.is_ascii_alphabetic()){
            self.settings.metar_station=code.clone();
            crate::game_lists::remember_setting("metar_station",&code);
            self.session.metar_rx=None; self.session.metar_once=false; self.session.metar_next=0.0;
            self.service_msg=Some((format!("METAR source: {code}"),3.0));
        }else if !code.is_empty(){self.service_msg=Some(("ICAO must be exactly 4 letters".into(),3.0));}
        self.refresh_list();
    }

    fn time_edit_key(&mut self, code: KeyCode) {
        let digit = match code {
            KeyCode::Digit0 | KeyCode::Numpad0 => Some('0'),
            KeyCode::Digit1 | KeyCode::Numpad1 => Some('1'),
            KeyCode::Digit2 | KeyCode::Numpad2 => Some('2'),
            KeyCode::Digit3 | KeyCode::Numpad3 => Some('3'),
            KeyCode::Digit4 | KeyCode::Numpad4 => Some('4'),
            KeyCode::Digit5 | KeyCode::Numpad5 => Some('5'),
            KeyCode::Digit6 | KeyCode::Numpad6 => Some('6'),
            KeyCode::Digit7 | KeyCode::Numpad7 => Some('7'),
            KeyCode::Digit8 | KeyCode::Numpad8 => Some('8'),
            KeyCode::Digit9 | KeyCode::Numpad9 => Some('9'),
            _ => None,
        };
        match code {
            KeyCode::Escape => self.menus.menu_edit = None,
            KeyCode::Backspace | KeyCode::Delete => {
                if let Some(d) = self.menus.menu_edit.as_mut() {
                    d.pop();
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                self.apply_time_edit();
                return;
            }
            _ => {
                if let (Some(c), Some(d)) = (digit, self.menus.menu_edit.as_mut()) {
                    if d.len() < 6 {
                        d.push(c);
                    }
                }
            }
        }
        self.refresh_list();
    }

    /// The search of the list shown is over (another list is shown).
    fn forget_search(&mut self) {
        self.menus.menu_search.clear();
        if self.menus.menu_edit_search {
            self.menus.menu_edit = None;
            self.menus.menu_edit_search = false;
        }
    }

    /// A key while the search of a list is typed: Enter keeps it, Escape drops what was
    /// typed (the search before stays); the text comes through `route_edit_text`.
    fn search_edit_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Escape => {
                self.menus.menu_edit = None;
                self.menus.menu_edit_search = false;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                self.menus.menu_search = self.menus.menu_edit.take().unwrap_or_default();
                self.menus.menu_edit_search = false;
            }
            KeyCode::Backspace | KeyCode::Delete => {
                if let Some(text) = self.menus.menu_edit.as_mut() {
                    text.pop();
                }
            }
            _ => {
                if let Some(c) = route_char(code) {
                    self.route_edit_text(&c.to_string());
                    return;
                }
            }
        }
        self.refresh_list();
        self.menus.chooser = Some(0);
    }

    /// A key while a route number is typed in the destination list (#836). Printable
    /// text comes through `route_edit_text` so keyboard layouts and symbols are preserved;
    /// physical key codes remain a fallback for platforms that do not provide text.
    fn route_edit_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Escape => self.menus.menu_edit = None,
            KeyCode::Backspace | KeyCode::Delete => {
                if let Some(t) = self.menus.menu_edit.as_mut() {
                    t.pop();
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(t) = self.menus.menu_edit.take() {
                    crate::game_lists::set_route_by_hand(self, &t);
                    self.close_game_menu();
                }
                return;
            }
            _ => {
                if let (Some(c), Some(t)) = (route_char(code), self.menus.menu_edit.as_mut()) {
                    if t.chars().count() < ROUTE_NUMBER_MAX {
                        t.push(c);
                    }
                }
            }
        }
        self.refresh_list();
    }

    /// Text entered in OMSI's free route-number field. It is intentionally not restricted
    /// to letters and digits: add-on displays use values such as `-10` and other symbols.
    pub(crate) fn route_edit_text(&mut self, text: &str) {
        if self.menus.menu_edit_search {
            if let Some(query) = self.menus.menu_edit.as_mut() {
                for c in text.chars().filter(|c| !c.is_control()) {
                    if query.chars().count() >= SEARCH_MAX {
                        break;
                    }
                    query.push(c);
                }
            }
            self.refresh_list();
            self.menus.chooser = Some(0);
            return;
        }
        if !matches!(self.menus.list_kind, Some(crate::game_lists::ListKind::RouteNumbers)) || self.menus.menu_edit.is_none() {
            return;
        }
        if let Some(t) = self.menus.menu_edit.as_mut() {
            for c in text.chars().filter(|c| !c.is_control()) {
                if t.chars().count() >= ROUTE_NUMBER_MAX {
                    break;
                }
                t.push(c);
            }
        }
        self.refresh_list();
    }

    /// Set the clock to the time typed (digits: hh, hhmm or hhmmss; what is missing is 0).
    pub(crate) fn apply_time_edit(&mut self) {
        let Some(d) = self.menus.menu_edit.take() else { return };
        if !d.is_empty() {
            let mut c = d.clone();
            while c.len() < 6 {
                c.push('0');
            }
            let n = |a: usize| c[a..a + 2].parse::<i64>().unwrap_or(0);
            let (h, m, sec) = (n(0), n(2), n(4));
            if h > 23 || m > 59 || sec > 59 {
                self.service_msg = Some((format!("{:02}:{:02}:{:02} is no time of day", h, m, sec), 3.0));
            } else if self.net.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client) {
                self.service_msg = Some(("In a LAN session the host sets the clock".into(), 3.0));
            } else if self.real_time_locked() {
                self.service_msg = Some(("The time cannot be changed while the real-time sync is on".into(), 3.0));
            } else {
                let t = self.clock.time;
                let day_start = t - t.rem_euclid(86400.0);
                self.shift_clock(day_start + (h * 3600 + m * 60 + sec) as f64 - t);
                self.service_msg = Some((format!("Clock: {:02}:{:02}:{:02}", h, m, sec), 3.0));
            }
        }
        self.refresh_list();
    }

    /// The open list made again from what it shows (a value changed), the chosen line kept.
    pub(crate) fn refresh_list(&mut self) {
        let Some(kind) = self.menus.list_kind.clone() else { return };
        let keep = self.menus.chooser;
        self.open_list(kind);
        if let (Some(k), Some(l)) = (keep, self.menus.admin_list.as_ref()) {
            self.menus.chooser = Some(k.min(l.len().saturating_sub(1)));
        }
    }

    /// A settings window (options, vehicle, world) is open.
    fn settings_list(&self) -> bool {
        use crate::game_lists::ListKind;
        self.menus.chooser.is_some() && matches!(self.menus.list_kind, Some(ListKind::Options(_) | ListKind::Vehicle(_) | ListKind::World(_) | ListKind::Controls | ListKind::Keyboard(_) | ListKind::ControllerDevices(_) | ListKind::Controller(..) | ListKind::ControllerAxis(..) | ListKind::ControllerButtonSettings(..)))
    }

    /// The open list is closed: back to the game menu.
    pub(crate) fn close_list(&mut self) {
        self.menus.key_capture = None;
        self.menus.dropdown = None;
        if self.menus.menu_edit_icao { if let Some(w)=self.window.as_ref(){w.set_ime_allowed(false);} }
        self.menus.menu_edit_icao=false;
        self.menus.menu_edit_search = false;
        self.menus.menu_search.clear();
        self.menus.menu_edit = None;
        self.menus.chooser = None;
        self.menus.admin_list = None;
        self.menus.list_kind = None;
        self.menus.menu_top = None;
    }

    /// Show page `i` of the open settings window.
    pub(crate) fn settings_tab(&mut self, i: usize) {
        use crate::game_lists::ListKind;
        let next = match self.menus.list_kind.as_ref() {
            Some(ListKind::Options(_)) => ListKind::Options(i),
            Some(ListKind::Vehicle(_)) => ListKind::Vehicle(i),
            Some(ListKind::World(_)) => ListKind::World(i),
            Some(ListKind::Keyboard(_)) => ListKind::Keyboard(i.min(1)),
            Some(ListKind::ControllerDevices(_)) => ListKind::ControllerDevices(i.min(crate::game_controller_menu::COMMON_TABS.len() - 1)),
            Some(ListKind::Controller(name, _)) => ListKind::Controller(name.clone(), i.min(3)),
            _ => return,
        };
        self.menus.menu_top = None;
        self.menus.menu_edit = None;
        self.menus.key_capture = None;
        self.open_list(next);
    }

    /// The next (or previous) page of the open settings window, round the ends.
    fn settings_tab_step(&mut self, forward: bool) {
        let Some(kind) = self.menus.list_kind.clone() else { return };
        let Some((titles, at)) = crate::game_lists::page_titles(self, &kind) else { return };
        let n = titles.len().max(1);
        self.settings_tab(if forward { (at + 1) % n } else { (at + n - 1) % n });
    }

    /// A click on the sidebar of a settings window: page `i`, or (the last box) the way back.
    pub(crate) fn settings_side_click(&mut self, i: usize) {
        let Some(kind) = self.menus.list_kind.clone() else { return };
        let n = crate::game_lists::page_titles(self, &kind).map(|t| t.0.len()).unwrap_or(0);
        if i < n {
            self.settings_tab(i);
        } else {
            self.menus.key_capture = None;
            if let Some(parent) = crate::game_lists::run(self, &kind, "back") {
                self.menus.menu_top = None;
                self.open_list(parent);
            } else {
                self.close_list();
            }
        }
    }

    /// Change the value of line `k` of the open settings window as `mv` says.
    pub(crate) fn list_adjust(&mut self, k: usize, mv: crate::game_lists::Move) {
        use crate::game_lists::ListKind;
        let Some(kind) = self.menus.list_kind.clone() else { return };
        if !matches!(kind, ListKind::Options(_) | ListKind::World(_) | ListKind::ControllerDevices(_) | ListKind::Controller(..) | ListKind::ControllerAxis(..) | ListKind::ControllerButtonSettings(..)) {
            return;
        }
        let Some(action) = self.menus.admin_list.as_ref().and_then(|l| l.get(k)).map(|x| x.1.clone()) else { return };
        let slider = crate::game_lists::is_slider(action.split(' ').next().unwrap_or(""));
        crate::game_lists::LIST_DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(next) = crate::game_lists::run_move(self, &kind, &action, mv) {
            // (a slider dragged sends the same value many times over: the list stays)
            if slider && !crate::game_lists::LIST_DIRTY.swap(false, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            self.open_list(next);
            let last = self.menus.admin_list.as_ref().map(|l| l.len().saturating_sub(1)).unwrap_or(0);
            self.menus.chooser = Some(k.min(last));
        }
    }

    /// A click on the control of line `k` (a slider's track, a stepper), `fx` of the way
    /// along it from the left.
    /// True when the control is a slider (which the mouse button then holds: it follows the cursor).
    pub(crate) fn list_click(&mut self, k: usize, fx: f32) -> bool {
        use crate::game_lists::Move;
        let Some(action) = self.menus.admin_list.as_ref().and_then(|l| l.get(k)).map(|x| x.1.clone()) else { return false };
        let verb = action.split(' ').next().unwrap_or("");
        let slider = crate::game_lists::is_slider(verb);
        let mv = if slider {
            Move::To(fx)
        } else if fx < 0.5 {
            Move::Dec
        } else {
            Move::Inc
        };
        self.list_adjust(k, mv);
        slider
    }


    /// A key while the vehicle chooser is open.
    pub(super) fn chooser_key(&mut self, code: KeyCode) {
        if self.menus.dropdown.is_some() {
            self.dropdown_key(code);
            return;
        }
        if self.menus.menu_edit.is_some() {
            if self.menus.menu_edit_search {
                self.search_edit_key(code);
            } else if self.menus.menu_edit_icao {
                self.icao_edit_key(code);
            } else if matches!(self.menus.list_kind, Some(crate::game_lists::ListKind::RouteNumbers)) {
                self.route_edit_key(code);
            } else {
                self.time_edit_key(code);
            }
            return;
        }
        let n = self.menus.admin_list.as_ref().unwrap_or(&self.menus.vehicle_list).len().max(1);
        let sel = self.menus.chooser.unwrap_or(0);
        self.menus.menu_top = None;
        match code {
            KeyCode::Escape if crate::game_controller_menu::is_controller_list(self.menus.list_kind.as_ref()) || matches!(self.menus.list_kind, Some(crate::game_lists::ListKind::Events | crate::game_lists::ListKind::Keyboard(_))) => {
                if let Some(kind) = self.menus.list_kind.clone() {
                    if let Some(back) = crate::game_lists::run(self, &kind, "back") {
                        self.open_list(back);
                    }
                }
            }
            KeyCode::Escape => {
                if self.tours_list() {
                    self.open_list(crate::game_lists::ListKind::Lines);
                } else {
                    self.menus.chooser = None;
                    self.menus.admin_list = None;
                    self.menus.list_kind = None;
                }
            }
            KeyCode::ArrowUp | KeyCode::KeyW => self.menus.chooser = Some(self.chooser_next(sel, n - 1)),
            KeyCode::ArrowDown | KeyCode::KeyS => self.menus.chooser = Some(self.chooser_next(sel, 1)),
            KeyCode::ArrowLeft | KeyCode::KeyA if self.settings_list() => self.list_adjust(sel, crate::game_lists::Move::Dec),
            KeyCode::ArrowRight | KeyCode::KeyD if self.settings_list() => self.list_adjust(sel, crate::game_lists::Move::Inc),
            KeyCode::Tab if self.settings_list() => {
                let back = self.input.keys.contains(&KeyCode::ShiftLeft) || self.input.keys.contains(&KeyCode::ShiftRight);
                self.settings_tab_step(!back);
            }
            // (the stop to start from: - and +)
            KeyCode::Minus | KeyCode::Slash | KeyCode::NumpadSubtract if self.tours_list() => self.tour_stop_step(false),
            KeyCode::Equal | KeyCode::BracketRight | KeyCode::NumpadAdd if self.tours_list() => self.tour_stop_step(true),
            // (the trip of the tour: the arrows go to the one leaving before or after, as OMSI's)
            KeyCode::ArrowLeft | KeyCode::KeyA if self.tours_list() => self.trip_step(false),
            KeyCode::ArrowRight | KeyCode::KeyD if self.tours_list() => self.trip_step(true),
            KeyCode::ArrowLeft | KeyCode::KeyA => self.chooser_adjust(sel, "-"),
            KeyCode::ArrowRight | KeyCode::KeyD => self.chooser_adjust(sel, "+"),
            KeyCode::PageUp if self.settings_list() => self.settings_tab_step(false),
            KeyCode::PageDown if self.settings_list() => self.settings_tab_step(true),
            KeyCode::PageUp => self.menus.chooser = Some(sel.saturating_sub(15)).map(|k| if self.is_heading(k) { self.chooser_next(k, 1) } else { k }),
            KeyCode::PageDown => self.menus.chooser = Some((sel + 15).min(n - 1)).map(|k| if self.is_heading(k) { self.chooser_next(k, 1) } else { k }),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.chooser_pick(sel),
            _ => {}
        }
    }

    /// A key while a drop-down is open: the arrows choose, Enter takes, Esc closes it.
    fn dropdown_key(&mut self, code: KeyCode) {
        let Some(d) = self.menus.dropdown.as_mut() else { return };
        let n = d.items.len().max(1);
        match code {
            KeyCode::Escape => {
                self.menus.dropdown = None;
                return;
            }
            KeyCode::ArrowUp | KeyCode::KeyW => d.sel = (d.sel + n - 1) % n,
            KeyCode::ArrowDown | KeyCode::KeyS => d.sel = (d.sel + 1) % n,
            KeyCode::PageUp => d.sel = d.sel.saturating_sub(5),
            KeyCode::PageDown => d.sel = (d.sel + 5).min(n - 1),
            KeyCode::Home => d.sel = 0,
            KeyCode::End => d.sel = n - 1,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                let i = d.sel;
                self.dropdown_pick(i);
                return;
            }
            _ => return,
        }
        self.dd_reveal();
    }

    /// The drop-down's chosen entry is in view.
    fn dd_reveal(&mut self) {
        let rows = self.ui.as_ref().map(|u| u.dd_rows).unwrap_or(8).max(1);
        if let Some(d) = self.menus.dropdown.as_mut() {
            if d.sel < d.top {
                d.top = d.sel;
            } else if d.sel >= d.top + rows {
                d.top = d.sel + 1 - rows;
            }
        }
    }

    /// Entry `i` of the open drop-down is taken: done, and the window's rows shown again
    /// with the new value.
    pub(crate) fn dropdown_pick(&mut self, i: usize) {
        let Some(d) = self.menus.dropdown.take() else { return };
        let Some((_, action)) = d.items.get(i).cloned() else { return };
        crate::game_lists::dropdown_apply(self, &action);
        if let Some(kind) = self.menus.list_kind.clone() {
            self.open_list(kind);
            let last = self.menus.admin_list.as_ref().map(|l| l.len().saturating_sub(1)).unwrap_or(0);
            self.menus.chooser = Some(d.row.min(last));
        }
    }

    /// A line's tours are open, with the stops of the tour chosen beside them.
    fn tours_list(&self) -> bool {
        self.menus.chooser.is_some() && matches!(self.menus.list_kind, Some(crate::game_lists::ListKind::Tours(..)))
    }

    /// The trip of the chosen tour leaving before (or after) the one chosen, as OMSI's
    /// timetable steps through the times of a tour (the tour itself stays).
    pub(crate) fn trip_step(&mut self, forward: bool) {
        let k = self.menus.chooser.unwrap_or(0);
        let (Some((line, tour)), Some((_, _, trip, trips))) = (crate::game_lists::tour_at(self, k), crate::game_lists::tour_choice(self, k)) else { return };
        let to = if forward { (trip + 1).min(trips.saturating_sub(1)) } else { trip.saturating_sub(1) };
        if to != trip {
            self.menus.pane_scroll = None;
            self.menus.list_kind = Some(crate::game_lists::ListKind::Tours(line, Some((tour, 0, to))));
        }
    }

    /// The stop to start the chosen tour from, one on (or back).
    fn tour_stop_step(&mut self, forward: bool) {
        let k = self.menus.chooser.unwrap_or(0);
        let (Some((line, tour)), Some((n, at, trip, _))) = (crate::game_lists::tour_at(self, k), crate::game_lists::tour_choice(self, k)) else { return };
        let to = if forward { (at + 1).min(n - 1) } else { at.saturating_sub(1) };
        self.menus.pane_scroll = None;
        self.menus.list_kind = Some(crate::game_lists::ListKind::Tours(line, Some((tour, to, trip))));
    }

    /// A click in the timetable beside the tours: stop `i` as the start, or (`usize::MAX`)
    /// the button that starts the trip; `usize::MAX - 1` / `- 2` the trip before / after.
    pub(crate) fn tour_pane_click(&mut self, i: usize) {
        let k = self.menus.chooser.unwrap_or(0);
        // (the arrows beside the time: `usize::MAX - 1` the trip before, `- 2` the next)
        if i == usize::MAX - 1 || i == usize::MAX - 2 {
            self.trip_step(i == usize::MAX - 2);
            return;
        }
        let (Some((line, tour)), Some((n, at, trip, _))) = (crate::game_lists::tour_at(self, k), crate::game_lists::tour_choice(self, k)) else { return };
        if i < n {
            self.menus.pane_scroll = None;
            self.menus.list_kind = Some(crate::game_lists::ListKind::Tours(line, Some((tour, i, trip))));
            return;
        }
        crate::game_lists::start_duty_at(self, &line, &tour, trip, at);
        self.menus.chooser = None;
        self.menus.admin_list = None;
        self.menus.list_kind = None;
        self.close_game_menu();
    }

    /// Place the chosen vehicle: in front of the camera in a free or map view, else beside
    /// the vehicle driven (OMSI puts a new vehicle where the map view points).
    pub(crate) fn chooser_pick(&mut self, k: usize) {
        // (a heading is no choice)
        if self.is_heading(k) {
            return;
        }
        // a row of a settings window that drops a list down (the weather preset, the clouds)
        if self.settings_list() {
            let id = self.menus.admin_list.as_ref().and_then(|l| l.get(k)).map(|l| l.1.clone()).unwrap_or_default();
            if let Some(d) = crate::game_lists::dropdown_for(self, k, &id) {
                self.menus.dropdown = Some(d);
                self.dd_reveal();
                return;
            }
        }
        self.menus.chooser = None;
        // a list of the menu's (the administration, the options …): done, and the list
        // shown again - or the next one (a line's tours), or back to the menu
        if let Some(list) = self.menus.admin_list.take() {
            let kind = self.menus.list_kind.take().unwrap_or(crate::game_lists::ListKind::Admin);
            let Some((_, action)) = list.get(k).cloned() else { return };
            match crate::game_lists::run(self, &kind, &action) {
                Some(next) => {
                    let keep = next == kind;
                    if !keep {
                        self.menus.menu_top = None;
                        self.forget_search();
                    }
                    self.open_list(next);
                    if keep {
                        self.menus.chooser = Some(k.min(self.menus.admin_list.as_ref().map(|l| l.len().saturating_sub(1)).unwrap_or(0)));
                    }
                }
                None if action != "back" && matches!(kind, crate::game_lists::ListKind::Tours(..) | crate::game_lists::ListKind::Numbers | crate::game_lists::ListKind::Destinations | crate::game_lists::ListKind::RouteNumbers | crate::game_lists::ListKind::Hofs | crate::game_lists::ListKind::Spots) => self.close_game_menu(),
                None => self.menus.menu_top = None,
            }
            return;
        }
        // a vehicle of the list: its livery and depot file are asked for first
        self.menus.menu_top = None;
        let Some((_, bus)) = self.menus.vehicle_list.get(k).cloned() else { return };
        self.open_list(crate::game_lists::ListKind::PlaceLivery(bus));
    }
}

/// Printable fallback for a route number when the window backend supplies no text event.
/// Normal typing uses the actual text event so Shift/layout-specific symbols are kept.
fn route_char(code: KeyCode) -> Option<char> {
    let symbol = match code {
        KeyCode::Minus | KeyCode::NumpadSubtract => Some('-'),
        KeyCode::Equal | KeyCode::NumpadAdd => Some('+'),
        KeyCode::Slash | KeyCode::NumpadDivide => Some('/'),
        KeyCode::NumpadMultiply => Some('*'),
        KeyCode::Period | KeyCode::NumpadDecimal => Some('.'),
        KeyCode::Comma => Some(','),
        KeyCode::Semicolon => Some(';'),
        KeyCode::Quote => Some('\''),
        KeyCode::BracketLeft => Some('['),
        KeyCode::BracketRight => Some(']'),
        KeyCode::Backslash => Some('\\'),
        KeyCode::Backquote => Some('`'),
        _ => None,
    };
    if symbol.is_some() {
        return symbol;
    }
    let name = format!("{code:?}");
    let c = name.strip_prefix("Digit").or_else(|| name.strip_prefix("Numpad")).or_else(|| name.strip_prefix("Key"))?;
    let mut chars = c.chars();
    let ch = chars.next()?;
    (chars.next().is_none() && ch.is_ascii_alphanumeric()).then(|| ch.to_ascii_uppercase())
}

#[cfg(test)]
mod route_tests {
    #[test]
    fn a_route_number_takes_digits_letters_and_symbols() {
        use winit::keyboard::KeyCode;
        assert_eq!(super::route_char(KeyCode::Digit5), Some('5'));
        assert_eq!(super::route_char(KeyCode::Numpad0), Some('0'));
        assert_eq!(super::route_char(KeyCode::KeyE), Some('E'));
        assert_eq!(super::route_char(KeyCode::Minus), Some('-'));
        assert_eq!(super::route_char(KeyCode::NumpadSubtract), Some('-'));
        assert_eq!(super::route_char(KeyCode::NumpadAdd), Some('+'));
        assert_eq!(super::route_char(KeyCode::Slash), Some('/'));
        assert_eq!(super::route_char(KeyCode::Space), None);
    }
}

/// How long a route number typed by hand may be. Eight characters were too few: Hong Kong
/// buses take commands through it (`paper_sign_1_name=ABC.png`, `adddept_sign=1`, #1518).
const ROUTE_NUMBER_MAX: usize = 64;

/// How long the search of a list may be.
const SEARCH_MAX: usize = 80;

/// The first entry a drop-down (or the tours' timetable) of `n` entries showing `rows`
/// shows with its thumb (`len` high) at the top `thumb_top` in `track`.
pub(crate) fn dropdown_top_at(thumb_top: f32, track: [f32; 4], len: f32, n: usize, rows: usize) -> usize {
    let max = n.saturating_sub(rows);
    let travel = (track[3] - track[1] - len).max(1.0);
    let f = ((thumb_top - track[1]) / travel).clamp(0.0, 1.0);
    ((f * max as f32).round() as usize).min(max)
}

#[cfg(test)]
mod dropdown_tests {
    /// A drop-down's thumb dragged down its track scrolls the list to its end (#794).
    #[test]
    fn a_dropdowns_thumb_dragged_scrolls_it() {
        // 40 entries, 8 shown, a 300 px track with a 60 px thumb
        let track = [0.0, 100.0, 4.0, 400.0];
        assert_eq!(super::dropdown_top_at(100.0, track, 60.0, 40, 8), 0);
        assert_eq!(super::dropdown_top_at(340.0, track, 60.0, 40, 8), 32);
        assert_eq!(super::dropdown_top_at(220.0, track, 60.0, 40, 8), 16);
        // past the ends it stays at them
        assert_eq!(super::dropdown_top_at(-50.0, track, 60.0, 40, 8), 0);
        assert_eq!(super::dropdown_top_at(900.0, track, 60.0, 40, 8), 32);
        // a list that fits never scrolls
        assert_eq!(super::dropdown_top_at(300.0, track, 60.0, 5, 8), 0);
    }
}
