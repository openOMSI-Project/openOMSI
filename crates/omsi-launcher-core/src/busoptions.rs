//! The bus options the player chose in the launcher's bus step: the `[setvar]` variables of a
//! bus's liveries (its mirrors, rims, seats, the gearbox it reports), each set to a value of
//! the player's own instead of the livery's. They are kept per bus file in
//! `~/.openomsi/bus-options.json` - a bus keeps its own whatever is driven in between - and
//! go to the game as `--setvar name=value,…` after `--paint` (see `duty_args`).
//!
//! What is left "as the livery" is not in the file: only the choices made.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The choices, per bus file (see `bus_key`): variable (as the livery spells it) → value.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(transparent)]
pub struct BusOptions {
    buses: BTreeMap<String, BTreeMap<String, f32>>,
}

/// A bus file as the choices are kept by: lower case, forward slashes (the launcher and an
/// older list may spell the same file either way).
pub fn bus_key(bus: &str) -> String {
    bus.trim().replace('\\', "/").to_ascii_lowercase()
}

impl BusOptions {
    /// Where they are kept.
    pub fn path() -> PathBuf {
        crate::data_dir().join("bus-options.json")
    }

    /// The choices kept (none when there is no file or it cannot be read).
    pub fn load() -> BusOptions {
        Self::read(&Self::path())
    }

    pub fn read(path: &Path) -> BusOptions {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.write(&Self::path())
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// The choices for `bus`: variable → value (empty: all as the livery).
    pub fn of(&self, bus: &str) -> BTreeMap<String, f32> {
        self.buses.get(&bus_key(bus)).cloned().unwrap_or_default()
    }

    /// The value chosen for `var` of `bus` (the variable's spelling does not matter).
    pub fn get(&self, bus: &str, var: &str) -> Option<f32> {
        self.buses.get(&bus_key(bus))?.iter().find(|(k, _)| k.eq_ignore_ascii_case(var)).map(|(_, v)| *v)
    }

    /// Choose `value` for `var` of `bus`; None leaves it as the livery has it again.
    pub fn set(&mut self, bus: &str, var: &str, value: Option<f32>) {
        let key = bus_key(bus);
        let vars = self.buses.entry(key.clone()).or_default();
        vars.retain(|k, _| !k.eq_ignore_ascii_case(var));
        if let Some(v) = value.filter(|v| v.is_finite()) {
            vars.insert(var.to_string(), v);
        }
        if vars.is_empty() {
            self.buses.remove(&key);
        }
    }

    /// Everything of `bus` as the livery has it again.
    pub fn reset(&mut self, bus: &str) {
        self.buses.remove(&bus_key(bus));
    }
}

/// A value as `--setvar` takes it: whole numbers without a decimal point.
pub fn value_text(v: f32) -> String {
    if v.fract() == 0.0 && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// The `--setvar` argument for `vars` (`name=value,…`), None for none. A name that would not
/// come through it whole (a comma, an equals sign, a space) is left out.
pub fn setvar_arg(vars: &[(String, f32)]) -> Option<String> {
    let parts: Vec<String> = vars
        .iter()
        .filter(|(n, v)| !n.is_empty() && v.is_finite() && !n.contains([',', '=']) && !n.contains(char::is_whitespace))
        .map(|(n, v)| format!("{n}={}", value_text(*v)))
        .collect();
    (!parts.is_empty()).then(|| parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_are_kept_per_bus_file_whatever_its_spelling() {
        let mut o = BusOptions::default();
        o.set("Vehicles\\ABCoach_O560\\O560_E6.bus", "vis_mirrors", Some(0.0));
        o.set("vehicles/abcoach_o560/o560_e6.bus", "vis_wheels", Some(1.0));
        o.set("Vehicles/MAN_SD200/SD200.bus", "vis_grill_invisible", Some(1.0));
        assert_eq!(o.get("Vehicles/ABCoach_O560/O560_E6.bus", "VIS_MIRRORS"), Some(0.0));
        assert_eq!(o.of("Vehicles/ABCoach_O560/O560_E6.bus").len(), 2);
        // a variable chosen again is chosen once, in the spelling of the last choice
        o.set("Vehicles/ABCoach_O560/O560_E6.bus", "VIS_Mirrors", Some(1.0));
        assert_eq!(o.of("Vehicles/ABCoach_O560/O560_E6.bus").into_iter().collect::<Vec<_>>(), vec![("VIS_Mirrors".to_string(), 1.0), ("vis_wheels".to_string(), 1.0)]);
        // back to the livery's, one by one and all at once
        o.set("Vehicles/ABCoach_O560/O560_E6.bus", "vis_mirrors", None);
        assert_eq!(o.get("Vehicles/ABCoach_O560/O560_E6.bus", "vis_mirrors"), None);
        o.reset("Vehicles/ABCoach_O560/O560_E6.bus");
        assert!(o.of("Vehicles/ABCoach_O560/O560_E6.bus").is_empty());
        assert_eq!(o.get("Vehicles/MAN_SD200/SD200.bus", "vis_grill_invisible"), Some(1.0), "another bus keeps its own");
        // (nothing left of a bus: no empty entry for it)
        o.set("Vehicles/MAN_SD200/SD200.bus", "vis_grill_invisible", None);
        assert_eq!(o, BusOptions::default());
    }

    #[test]
    fn choices_survive_a_launch_in_their_file() {
        let dir = std::env::temp_dir().join(format!("omsi_bus_options_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bus-options.json");
        let mut o = BusOptions::default();
        o.set("Vehicles/MAN_NewLionsCity/MAN_12C_2door_ZF.bus", "vis_CTI_spiegeltyp", Some(2.0));
        o.set("Vehicles/MAN_NewLionsCity/MAN_12C_2door_ZF.bus", "vis_CTI_Radbesen", Some(0.5));
        o.write(&file).unwrap();
        let back = BusOptions::read(&file);
        assert_eq!(back, o);
        assert!(std::fs::read_to_string(&file).unwrap().contains("\"vehicles/man_newlionscity/man_12c_2door_zf.bus\""));
        // a file that is not one: nothing chosen, nothing broken
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(BusOptions::read(&file), BusOptions::default());
        assert_eq!(BusOptions::read(&dir.join("none.json")), BusOptions::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_setvar_argument() {
        assert_eq!(setvar_arg(&[]), None);
        let vars = vec![("vis_mirrors".to_string(), 0.0), ("vis_CTI_matrix".to_string(), 2.0), ("seat_tilt".to_string(), 0.25), ("minus".to_string(), -1.0)];
        assert_eq!(setvar_arg(&vars).as_deref(), Some("vis_mirrors=0,vis_CTI_matrix=2,seat_tilt=0.25,minus=-1"));
        // names the game could not split back off are left out
        let odd = vec![("a,b".to_string(), 1.0), ("c=d".to_string(), 1.0), ("e f".to_string(), 1.0), (String::new(), 1.0), ("nan".to_string(), f32::NAN), ("ok".to_string(), 1.0)];
        assert_eq!(setvar_arg(&odd).as_deref(), Some("ok=1"));
        assert_eq!(value_text(3.0), "3");
        assert_eq!(value_text(-0.5), "-0.5");
    }
}
