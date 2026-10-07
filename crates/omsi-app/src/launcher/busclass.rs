//! The kinds of the installed buses (`service::VehicleClass`: a minibus, a midibus, a coach…)
//! and their makers and models, as a line's choice of buses needs them: the line editor's
//! list of the buses that run a line, and the bus step's marks on them. Telling a minibus from
//! a midibus, or an articulated bus from a solo one, needs each bus file read (its length,
//! its trailer section): that runs once on a thread of its own, and again when the installed
//! buses change; until it is done a bus's kind is guessed from its names.

use omsi_launcher_lib::service::{self, BusFacts, LineVehicles, VehicleClass};
use omsi_launcher_lib::VehicleInfo;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

#[derive(Default)]
pub struct BusClasses {
    /// For how many installed buses the kinds were asked, and the answer on its way.
    asked: Option<usize>,
    rx: Option<Receiver<HashMap<String, VehicleClass>>>,
    /// The kinds by bus file (`key`).
    kinds: Arc<HashMap<String, VehicleClass>>,
}

/// A bus file as the kinds are kept by (lower case, '/').
pub fn key(file: &str) -> String {
    file.trim().replace('\\', "/").to_lowercase()
}

impl BusClasses {
    /// The kinds of `vehicles` asked for (read on a thread once for each set of installed
    /// buses), and taken in when they came.
    pub fn want(&mut self, vehicles: &[VehicleInfo]) {
        if let Some(Ok(k)) = self.rx.as_ref().map(|rx| rx.try_recv()) {
            self.kinds = Arc::new(k);
            self.rx = None;
        }
        if self.asked == Some(vehicles.len()) || vehicles.is_empty() {
            return;
        }
        self.asked = Some(vehicles.len());
        let list: Vec<(String, Vec<String>)> = vehicles.iter().map(|v| (v.file.clone(), vec![v.name.clone(), v.manufacturer.clone(), v.type_name.clone()])).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            use rayon::prelude::*;
            let kinds: HashMap<String, VehicleClass> = list
                .par_iter()
                .map(|(file, texts)| {
                    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                    (key(file), service::class_of_file(file, &refs))
                })
                .collect();
            log::info!("bus kinds: {} buses told apart in {:.1} s", kinds.len(), t0.elapsed().as_secs_f32());
            let _ = tx.send(kinds);
        });
        self.rx = Some(rx);
    }

    /// The kinds are being read.
    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// The kind of an installed bus (guessed from its names while the files are read).
    pub fn class_of(&self, v: &VehicleInfo) -> VehicleClass {
        self.kinds.get(&key(&v.file)).copied().unwrap_or_else(|| service::classify(&[&v.name, &v.manufacturer, &v.type_name, &v.file], false, None, None))
    }

    /// What a line's choice weighs of an installed bus.
    pub fn facts(&self, v: &VehicleInfo) -> BusFacts {
        BusFacts::of_info(v, self.class_of(v))
    }

    /// The installed buses a line's choice allows, by their `key` (none when it chose none:
    /// then every bus may run it, and none is marked).
    pub fn matching(&self, vehicles: &[VehicleInfo], want: &LineVehicles) -> HashSet<String> {
        if want.open() {
            return HashSet::new();
        }
        vehicles.iter().filter(|v| want.allows(&self.facts(v))).map(|v| key(&v.file)).collect()
    }
}
