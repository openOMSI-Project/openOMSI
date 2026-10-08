//! The three gamepad families the game knows by name - Xbox, PlayStation 4 (DualShock 4) and
//! PlayStation 5 (DualSense) - what their buttons are called, and the buttons a gamepad
//! nobody set up in `gamectrler.cfg` drives with.
//!
//! The sticks and triggers need no table (the left stick steers, the right trigger is the
//! throttle, the left one the brake, the right stick looks round: `controllers.rs`); the
//! buttons do, because OMSI's file numbers them per driver and a gamepad has none in it.

use gilrs::Button;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PadKind {
    Xbox,
    Ps4,
    Ps5,
}

const SONY: u16 = 0x054c;

impl PadKind {
    /// The family a connected device belongs to, by its USB ids, else by its name. None: not
    /// one of the three (a wheel, a joystick, a Switch pad).
    pub(crate) fn detect(name: &str, id: Option<(u16, u16)>) -> Option<PadKind> {
        let n = name.to_ascii_lowercase();
        if let Some((vendor, product)) = id {
            if vendor == SONY {
                return match product {
                    0x0ce6 | 0x0df2 => Some(PadKind::Ps5),
                    0x05c4 | 0x09cc | 0x0ba0 | 0x0cda => Some(PadKind::Ps4),
                    _ if n.contains("dualsense") || n.contains("ps5") => Some(PadKind::Ps5),
                    // (a Sony device nobody lists by name: most are DualShocks)
                    _ if n.contains("dualshock") || n.contains("ps4") || n.contains("wireless controller") => Some(PadKind::Ps4),
                    _ => None,
                };
            }
        }
        if n.contains("dualsense") || n.contains("ps5 controller") {
            Some(PadKind::Ps5)
        } else if n.contains("dualshock") || n.contains("ps4 controller") {
            Some(PadKind::Ps4)
        } else if n.contains("xbox") || n.contains("xinput") || n.starts_with("controller (") {
            Some(PadKind::Xbox)
        } else {
            None
        }
    }

    /// The setting `pad_type`: `auto` (None) or one of the families.
    pub(crate) fn from_setting(v: &str) -> Option<PadKind> {
        match v.trim().to_ascii_lowercase().as_str() {
            "xbox" => Some(PadKind::Xbox),
            "ps4" => Some(PadKind::Ps4),
            "ps5" => Some(PadKind::Ps5),
            _ => None,
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            PadKind::Xbox => "Xbox",
            PadKind::Ps4 => "PlayStation 4",
            PadKind::Ps5 => "PlayStation 5",
        }
    }

    /// What the button is called on this pad.
    pub(crate) fn label(self, b: Button) -> &'static str {
        let sony = self != PadKind::Xbox;
        match b {
            Button::South => if sony { "Cross" } else { "A" },
            Button::East => if sony { "Circle" } else { "B" },
            Button::West => if sony { "Square" } else { "X" },
            Button::North => if sony { "Triangle" } else { "Y" },
            Button::LeftTrigger => if sony { "L1" } else { "LB" },
            Button::RightTrigger => if sony { "R1" } else { "RB" },
            Button::LeftTrigger2 => if sony { "L2" } else { "LT" },
            Button::RightTrigger2 => if sony { "R2" } else { "RT" },
            Button::Select => match self {
                PadKind::Xbox => "View",
                PadKind::Ps4 => "Share",
                PadKind::Ps5 => "Create",
            },
            Button::Start => if sony { "Options" } else { "Menu" },
            Button::LeftThumb => if sony { "L3" } else { "Left stick click" },
            Button::RightThumb => if sony { "R3" } else { "Right stick click" },
            Button::DPadUp => "D-pad up",
            Button::DPadDown => "D-pad down",
            Button::DPadLeft => "D-pad left",
            Button::DPadRight => "D-pad right",
            _ => "Button",
        }
    }
}

/// The buttons that have a default action, in the order the settings list them.
pub(crate) const PRESET_BUTTONS: [Button; 13] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::DPadLeft,
    Button::DPadRight,
    Button::DPadUp,
    Button::DPadDown,
    Button::Start,
    Button::Select,
    Button::RightThumb,
];

/// The key action a button drives when its pad has no buttons set up (OMSI's names, as
/// `gamectrler.cfg` and the Controllers page use them).
pub(crate) fn default_action(b: Button) -> Option<&'static str> {
    Some(match b {
        Button::South => "doors_all",
        Button::East => "view_toggle_interior",
        Button::West => "view_toggle_viewpoint",
        Button::North => "view_reset_direction",
        Button::LeftTrigger => "gear_down",
        Button::RightTrigger => "gear_up",
        Button::DPadLeft => "blinker_left_toggle",
        Button::DPadRight => "blinker_right_toggle",
        Button::DPadUp => "view_interiorcam_plus",
        Button::DPadDown => "view_interiorcam_minus",
        Button::Start => "sim_pause",
        Button::Select => "screenshot",
        Button::RightThumb => "view_reset_all_directions",
        _ => return None,
    })
}

/// What the action does, in the words of the settings list.
pub(crate) fn action_text(action: &str) -> &'static str {
    match action {
        "doors_all" => "Open / close all doors",
        "view_toggle_interior" => "Cabin / outside view",
        "view_toggle_viewpoint" => "Next view",
        "view_reset_direction" => "Reset the view direction",
        "gear_down" => "Gear down (manual gearbox)",
        "gear_up" => "Gear up (manual gearbox)",
        "blinker_left_toggle" => "Left indicator",
        "blinker_right_toggle" => "Right indicator",
        "view_interiorcam_plus" => "Next interior camera",
        "view_interiorcam_minus" => "Previous interior camera",
        "sim_pause" => "Pause",
        "screenshot" => "Screenshot",
        "view_reset_all_directions" => "Reset all view directions",
        _ => "",
    }
}

/// A button of a Sony pad read through DirectInput (Windows lists a DualShock 4 or a
/// DualSense as a generic game controller, which gilrs - XInput only there - never sees):
/// the buttons as the HID descriptor numbers them from 0, and the first hat after
/// `controllers::HAT_BUTTONS` as up, right, down, left. Triggers are axes and not listed.
pub(crate) fn sony_direct_input_button(n: usize, hat_base: usize) -> Option<Button> {
    if n >= hat_base {
        return match n - hat_base {
            0 => Some(Button::DPadUp),
            1 => Some(Button::DPadRight),
            2 => Some(Button::DPadDown),
            3 => Some(Button::DPadLeft),
            _ => None,
        };
    }
    match n {
        0 => Some(Button::West),
        1 => Some(Button::South),
        2 => Some(Button::East),
        3 => Some(Button::North),
        4 => Some(Button::LeftTrigger),
        5 => Some(Button::RightTrigger),
        8 => Some(Button::Select),
        9 => Some(Button::Start),
        10 => Some(Button::LeftThumb),
        11 => Some(Button::RightThumb),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_are_found_by_ids_and_names() {
        assert_eq!(PadKind::detect("Wireless Controller", Some((0x054c, 0x0ce6))), Some(PadKind::Ps5));
        assert_eq!(PadKind::detect("Wireless Controller", Some((0x054c, 0x09cc))), Some(PadKind::Ps4));
        assert_eq!(PadKind::detect("PS5 Controller", None), Some(PadKind::Ps5));
        assert_eq!(PadKind::detect("DualShock 4 Wireless Controller", None), Some(PadKind::Ps4));
        assert_eq!(PadKind::detect("Xbox Wireless Controller", Some((0x045e, 0x0b12))), Some(PadKind::Xbox));
        assert_eq!(PadKind::detect("Controller (XBOX 360 For Windows)", None), Some(PadKind::Xbox));
        assert_eq!(PadKind::detect("Logitech G29 Driving Force Racing Wheel", Some((0x046d, 0xc24f))), None);
        // (a "Wireless Controller" of another maker is nobody's gamepad family)
        assert_eq!(PadKind::detect("Wireless Controller", Some((0x057e, 0x2009))), None);
    }

    #[test]
    fn settings_and_names() {
        assert_eq!(PadKind::from_setting("auto"), None);
        assert_eq!(PadKind::from_setting("PS5"), Some(PadKind::Ps5));
        assert_eq!(PadKind::Xbox.label(Button::South), "A");
        assert_eq!(PadKind::Ps4.label(Button::South), "Cross");
        assert_eq!(PadKind::Ps4.label(Button::Select), "Share");
        assert_eq!(PadKind::Ps5.label(Button::Select), "Create");
    }

    #[test]
    fn every_preset_button_has_an_action_and_a_text() {
        for b in PRESET_BUTTONS {
            let a = default_action(b).expect("action");
            assert!(!action_text(a).is_empty(), "{a}");
        }
    }

    #[test]
    fn sony_buttons_through_direct_input() {
        assert_eq!(sony_direct_input_button(1, 128), Some(Button::South));
        assert_eq!(sony_direct_input_button(6, 128), None); // L2 is an axis
        assert_eq!(sony_direct_input_button(130, 128), Some(Button::DPadDown));
    }
}
