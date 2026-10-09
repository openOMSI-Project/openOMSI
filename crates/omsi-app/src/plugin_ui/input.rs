//! The mouse and the keyboard on the plugins' panels while they have the mouse: clicks,
//! sliders and tabs set where they are pressed, sliders and draggable panels followed while
//! the button is held, and the text typed into a field.

use super::{hit_at, HitKind, Key, PluginPanels};
use glam::Vec2;
use omsi_plugin::ui::UiState;
use omsi_ui::Rect;

/// What the left button holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Drag {
    /// A slider: its plugin, panel and id, and its track on the window (physical pixels).
    Slider { owner: u64, panel: String, element: String, track: [f32; 2] },
    /// A panel moved by its free parts: where the cursor was (physical pixels).
    Panel { owner: u64, panel: String, last: Vec2 },
}

/// A press on the panels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Press {
    pub owner: u64,
    pub panel: String,
    pub element: Option<String>,
    pub kind: HitKind,
    /// Where across the hit it is, 0..1 (a slider's value, the tab).
    pub fraction: f32,
    /// The hit's left and right edges on the window (physical pixels).
    pub span: [f32; 2],
    /// It is on no clickable part of a panel that can be dragged.
    pub drag: bool,
    /// It is on something that takes clicks (the panel itself when it is clickable).
    pub hit: bool,
}

/// A panel's place moved by where the player dragged it, kept on the screen.
pub(crate) fn moved(at: Vec2, by: (f32, f32), w: f32, h: f32, screen: Vec2) -> Vec2 {
    Vec2::new((at.x + by.0).clamp(0.0, (screen.x - w).max(0.0)), (at.y + by.1).clamp(0.0, (screen.y - h).max(0.0)))
}

impl PluginPanels {
    /// What a press at `(x, y)` (physical pixels on the window) is on: the topmost panel's
    /// part there.
    pub fn press_at(&self, ui: &UiState, x: f32, y: f32) -> Option<Press> {
        let k = self.scale.max(0.25);
        for e in ui.panels().iter().rev().filter(|e| e.panel.visible) {
            let Some(card) = self.cards.get(&Key::Panel(e.owner, e.id.clone())) else { continue };
            if card.seen != self.frame {
                continue;
            }
            let local = (Vec2::new(x, y) - card.origin) / k;
            if !Rect::new(0.0, 0.0, card.laid.w, card.laid.h).contains(local) {
                continue;
            }
            let hit = hit_at(&card.laid, local).map(|i| &card.laid.hits[i]);
            // (a clickable panel's own hit, the whole card, does not keep it from being dragged)
            let on_part = hit.is_some_and(|h| h.element.is_some());
            return Some(match hit {
                Some(h) => Press {
                    owner: e.owner,
                    panel: e.id.clone(),
                    element: h.element.clone(),
                    kind: h.kind,
                    fraction: ((local.x - h.r.x) / h.r.w.max(1.0)).clamp(0.0, 1.0),
                    span: [card.origin.x + h.r.x * k, card.origin.x + h.r.right() * k],
                    drag: e.panel.draggable && !on_part,
                    hit: true,
                },
                None => Press { owner: e.owner, panel: e.id.clone(), element: None, kind: HitKind::Click, fraction: 0.0, span: [0.0, 0.0], drag: e.panel.draggable, hit: false },
            });
        }
        None
    }
}

impl crate::App {
    /// Whether the plugins' panels have the mouse (`omsi.ui.focus`).
    pub(crate) fn plugin_focus(&self) -> bool {
        super::focused(&self.integrations.plugins)
    }

    /// Esc while the panels have the mouse: typing in a field ends first, then the mouse goes
    /// back to the bus (and not on to the menu). True when it did one of them.
    pub(crate) fn release_plugin_focus(&mut self) -> bool {
        let Some(p) = self.integrations.plugins.as_ref().filter(|p| p.ui.borrow().focused()) else {
            return false;
        };
        let mut ui = p.ui.borrow_mut();
        if ui.typing().is_some() {
            ui.stop_typing(false);
        } else {
            ui.release_focus();
        }
        true
    }

    /// The left button while the panels have the mouse: a press on a clickable part goes to
    /// its plugin as `ui_click`, a slider or tabs take the place pressed, a draggable panel
    /// follows the cursor from its free parts; the bus gets none of it.
    pub(crate) fn plugin_click(&mut self, pressed: bool) {
        let Some(p) = self.integrations.plugins.as_ref() else {
            return;
        };
        if !pressed {
            self.integrations.plugin_panels.drag = None;
            return;
        }
        let (x, y) = self.input.cursor;
        let press = self.integrations.plugin_panels.press_at(&p.ui.borrow(), x, y);
        let mut ui = p.ui.borrow_mut();
        // (a press anywhere but on the field typed into ends the typing)
        let on_field = |pr: &Option<super::input::Press>, ui: &UiState| matches!((pr, ui.typing()), (Some(pr), Some(t)) if pr.owner == t.owner && pr.panel == t.panel && pr.element.as_deref() == Some(t.element.as_str()));
        if ui.typing().is_some() && !on_field(&press, &ui) {
            ui.stop_typing(false);
        }
        let Some(pr) = press else { return };
        match (pr.kind, pr.element.as_deref()) {
            (HitKind::Slider, Some(el)) => {
                ui.set_control(pr.owner, &pr.panel, el, pr.fraction);
                self.integrations.plugin_panels.drag = Some(Drag::Slider { owner: pr.owner, panel: pr.panel.clone(), element: el.to_string(), track: pr.span });
            }
            (HitKind::Tabs, Some(el)) => ui.set_control(pr.owner, &pr.panel, el, pr.fraction),
            _ if pr.drag => {
                self.integrations.plugin_panels.drag = Some(Drag::Panel { owner: pr.owner, panel: pr.panel.clone(), last: Vec2::new(x, y) });
            }
            _ if pr.hit => ui.click(pr.owner, &pr.panel, pr.element.as_deref()),
            _ => {}
        }
    }

    /// The cursor moved while a slider or a panel is dragged: it follows. True when it did
    /// (the cursor's other work is left out then).
    pub(crate) fn plugin_drag_move(&mut self, x: f32, y: f32) -> bool {
        let Some(drag) = self.integrations.plugin_panels.drag.clone() else { return false };
        let Some(p) = self.integrations.plugins.as_ref() else { return false };
        let mut ui = p.ui.borrow_mut();
        if !ui.focused() {
            self.integrations.plugin_panels.drag = None;
            return false;
        }
        match drag {
            Drag::Slider { owner, panel, element, track } => {
                let f = ((x - track[0]) / (track[1] - track[0]).max(1.0)).clamp(0.0, 1.0);
                ui.set_control(owner, &panel, &element, f);
            }
            Drag::Panel { owner, panel, last } => {
                let k = self.integrations.plugin_panels.scale.max(0.25);
                ui.drag(owner, &panel, (x - last.x) / k, (y - last.y) / k);
                self.integrations.plugin_panels.drag = Some(Drag::Panel { owner, panel, last: Vec2::new(x, y) });
            }
        }
        true
    }

    /// A key while the player types into a plugin's text field: the text goes to the field,
    /// Backspace rubs out, Enter hands it to the plugin and Esc leaves it. True when the
    /// key was the field's (the bus does not get it).
    pub(crate) fn plugin_typing_key(&mut self, code: winit::keyboard::KeyCode, text: Option<&str>) -> bool {
        use winit::keyboard::KeyCode;
        let Some(p) = self.integrations.plugins.as_ref() else { return false };
        let mut ui = p.ui.borrow_mut();
        if ui.typing().is_none() {
            return false;
        }
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter => ui.stop_typing(true),
            KeyCode::Escape => ui.stop_typing(false),
            KeyCode::Backspace => ui.type_text(None),
            _ => {
                if let Some(t) = text.filter(|t| t.chars().any(|c| !c.is_control())) {
                    ui.type_text(Some(t));
                }
            }
        }
        true
    }
}
