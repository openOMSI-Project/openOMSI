//! Linux: devices with buttons but fewer than two axes - a gear shifter, a button box, a
//! handbrake button - read from evdev. gilrs leaves them out ("doesn't have at least 1
//! button and 2 axes"), so they could not be set up at all.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

const EV_KEY: u16 = 0x01;
const EVENT_SIZE: usize = 24;

struct Device {
    name: String,
    node: String,
    file: File,
    buttons: usize,
}

pub(crate) struct ButtonDevices {
    devices: Vec<Device>,
    scanned: Instant,
}

impl ButtonDevices {
    pub fn new() -> ButtonDevices {
        let mut d = ButtonDevices { devices: Vec::new(), scanned: Instant::now() };
        d.scan();
        d
    }

    /// Open the devices of this kind not open yet (and so one plugged in later).
    fn scan(&mut self) {
        self.scanned = Instant::now();
        let Ok(dir) = std::fs::read_dir("/sys/class/input") else { return };
        let mut nodes: Vec<String> = dir.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.starts_with("event")).collect();
        nodes.sort();
        for node in nodes {
            if self.devices.iter().any(|d| d.node == node) {
                continue;
            }
            let sys = format!("/sys/class/input/{node}/device");
            let Ok(name) = std::fs::read_to_string(format!("{sys}/name")) else { continue };
            let name = name.trim().to_string();
            let caps = |kind: &str| std::fs::read_to_string(format!("{sys}/capabilities/{kind}")).unwrap_or_default();
            let key_bits = caps("key");
            let keys = crate::controllers::key_bitmap_buttons(&key_bits);
            // (joystick and gamepad buttons only, and a device with two axes or more is gilrs's.
            // A gaming mouse reports buttons in the joystick range too, a keyboard's media keys
            // come as a device of their own: whatever moves a pointer or has keyboard keys is
            // not ours, nor what udev does not take for a joystick)
            if bit_count(&caps("abs")) >= 2
                || bit_count(&caps("rel")) > 0
                || has_keyboard_keys(&key_bits)
                || !keys.iter().any(|c| crate::controllers::code_button(*c).is_some())
                || udev_says_no_joystick(&node)
            {
                continue;
            }
            match OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(format!("/dev/input/{node}")) {
                Ok(file) => {
                    let buttons = crate::controllers::declared_button_count(&name);
                    log::info!("game controller: {name} (/dev/input/{node}), {buttons} buttons, no axes: read from evdev");
                    self.devices.push(Device { name, node, file, buttons });
                }
                Err(e) => log::warn!("game controller: {name} (/dev/input/{node}) cannot be read: {e}"),
            }
        }
    }

    /// The buttons pressed (true) and let go since the last call, numbered as for gilrs's devices.
    pub fn poll(&mut self, out: &mut Vec<(String, usize, bool)>) {
        if self.scanned.elapsed() > Duration::from_secs(2) {
            self.scan();
        }
        let mut gone = Vec::new();
        for (i, d) in self.devices.iter_mut().enumerate() {
            let mut buf = [0u8; EVENT_SIZE * 64];
            loop {
                match d.file.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        for ev in buf[..n - n % EVENT_SIZE].chunks_exact(EVENT_SIZE) {
                            let kind = u16::from_ne_bytes([ev[16], ev[17]]);
                            let code = u16::from_ne_bytes([ev[18], ev[19]]) as u32;
                            let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
                            // (2 is the key's auto-repeat)
                            if kind != EV_KEY || value > 1 {
                                continue;
                            }
                            if let Some(b) = crate::controllers::evdev_button_number(&d.name, code) {
                                out.push((d.name.clone(), b, value == 1));
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        log::info!("game controller: {} (/dev/input/{}) went away: {e}", d.name, d.node);
                        gone.push(i);
                        break;
                    }
                }
            }
        }
        for i in gone.into_iter().rev() {
            self.devices.remove(i);
        }
    }

    /// (name, number of buttons) of every device open.
    pub fn connected(&self) -> impl Iterator<Item = (&str, usize)> {
        self.devices.iter().map(|d| (d.name.as_str(), d.buttons))
    }
}

/// A key below the buttons (0x100, the bitmap's last four words): a keyboard's.
fn has_keyboard_keys(bitmap: &str) -> bool {
    bitmap.split_whitespace().rev().take(4).filter_map(|w| u64::from_str_radix(w, 16).ok()).any(|w| w != 0)
}

/// udev's database classifies the device and it is not a joystick (without the database,
/// in a container, nothing is known).
fn udev_says_no_joystick(node: &str) -> bool {
    let Ok(dev) = std::fs::read_to_string(format!("/sys/class/input/{node}/dev")) else { return false };
    let Ok(data) = std::fs::read_to_string(format!("/run/udev/data/c{}", dev.trim())) else { return false };
    !data.lines().any(|l| l == "E:ID_INPUT_JOYSTICK=1")
}

fn bit_count(bitmap: &str) -> u32 {
    bitmap.split_whitespace().filter_map(|w| u64::from_str_radix(w, 16).ok()).map(u64::count_ones).sum()
}

#[cfg(test)]
mod tests {
    #[test]
    fn axes_are_counted() {
        assert_eq!(super::bit_count("0"), 0);
        assert_eq!(super::bit_count("3007b"), 8);
        assert_eq!(super::bit_count("1 3"), 3);
    }

    #[test]
    fn keyboards_and_mice_are_told_from_a_gear_shifter() {
        // the Aerosoft gear shifter, a gaming mouse, a keyboard's media keys
        assert!(!super::has_keyboard_keys("3f 0 0 0 0 0 0 ffff00000000 0 0 0 0"));
        assert!(!super::has_keyboard_keys("ffffffff0000 0 0 0 0"));
        assert!(super::has_keyboard_keys("3f00733fff 0 0 483ffff17aff32d bfd4444600000000 1 130ff38b17d000 677bfad9415fed 19ed68000004400 10000002"));
    }
}
