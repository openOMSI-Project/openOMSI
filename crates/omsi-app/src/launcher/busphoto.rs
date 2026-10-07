//! The photos on the bus picker's tiles (see `buspick`): each bus drawn once by the game's own
//! renderer - a showroom of its own, the "studio", with the bus always seen from the same
//! front corner in the same noon light - read back and kept as a PNG under
//! `~/.openomsi/cache/busphotos/`, named after the bus file, the paint and the file's
//! modification time, so that a bus that changed is photographed again and one that did not is
//! never drawn twice.
//!
//! The tiles ask for their photos while they are drawn; what is on the disk is read on a thread
//! of its own and shown as it comes, what is not is photographed one bus at a time, the tiles on
//! screen first, and only while the launcher is idle: reading a bus and putting it on the GPU
//! stops a frame or two, which nobody notices while nobody moves the mouse. A tile shows the
//! bus's initials until its photo is there.

use super::showroom::{Look, Showroom};
use super::Launcher;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

/// A photo's size in pixels: 16:10, a tile's photo at about twice the interface's scale.
pub const PHOTO_W: u32 = 560;
pub const PHOTO_H: u32 = 350;
/// The way the photos are taken: raised when the corner, the light or the size change, so
/// that the photos taken before are taken again.
const STYLE: u32 = 2;
/// The photos on the GPU at once (under a megabyte each): those drawn longest ago go first,
/// and are read from the disk again when they are wanted.
const ON_GPU: usize = 48;
/// A bus that is not read and placed in this long is given up for the session.
const GIVE_UP_S: f32 = 120.0;
/// The light of every photo: a summer's day at half past twelve, the map's own weather.
const PHOTO_DATE: &str = "2000-06-21";
const PHOTO_TIME: i32 = 12 * 60 + 30;

/// A bus file and a paint (empty: the bus's own).
type Wanted = (String, String);

enum Photo {
    /// Its file is being read.
    Reading,
    /// On the GPU: its texture, its size, and when a tile last drew it.
    Shown { tex: usize, w: u32, h: u32, used: f32 },
    /// On the disk, put off the GPU to make room.
    Stored,
    /// Not photographed yet.
    ToTake,
    /// The bus could not be drawn (its model is protected, or does not load): its initials stay.
    Failed,
}

struct Entry {
    file: PathBuf,
    photo: Photo,
}

/// The photos asked for, and the studio that takes them.
#[derive(Default)]
pub struct Photos {
    known: HashMap<Wanted, Entry>,
    /// Asked for by the tiles drawn in the last frame, in their order (on screen first).
    asked: Vec<Wanted>,
    /// "Update bus pictures": every bus, photographed after what is on screen.
    backlog: VecDeque<Wanted>,
    backlog_total: usize,
    studio: Option<Box<Showroom>>,
    /// The bus being photographed: what was asked, the look it is drawn with, and the seconds
    /// spent on it (on the bus step: the studio waits while the step is not drawn).
    shooting: Option<(Wanted, Look, f32)>,
    /// The thread that reads the photos from the disk: the paths to read, and what it read.
    reader: Option<(Sender<(Wanted, PathBuf)>, Receiver<(Wanted, Option<image::RgbaImage>)>)>,
    on_gpu: usize,
}

/// Where the photos are kept.
pub fn cache_dir() -> PathBuf {
    omsi_launcher_lib::data_dir().join("cache").join("busphotos")
}

/// The photo's file name for a bus file (its path as resolved), a paint and the bus file's
/// modification time: a hash of the three and the photo's style.
pub fn photo_name(path: &str, paint: &str, mtime: u64) -> String {
    let key = format!("{STYLE}|{}|{}|{mtime}|{PHOTO_W}x{PHOTO_H}", path.replace('\\', "/").to_lowercase(), paint.trim().to_lowercase());
    format!("{:016x}.png", super::ui::id_of(&key))
}

fn photo_file(root: &str, bus: &str, paint: &str) -> PathBuf {
    let path = omsi_cfg::resolve_path(Path::new(root), bus);
    let mtime = omsi_launcher_lib::index::mtime_ns(&path);
    cache_dir().join(photo_name(&path.to_string_lossy(), paint, mtime))
}

impl Photos {
    /// The photo of `bus` in `paint` for a tile drawn now: its texture and size once it is
    /// there. Asking is what puts it in line to be read or photographed.
    pub fn get(&mut self, root: &str, bus: &str, paint: &str, now: f32) -> Option<(usize, u32, u32)> {
        let k: Wanted = (bus.to_string(), paint.to_string());
        if !self.known.contains_key(&k) {
            let file = photo_file(root, bus, paint);
            let photo = if file.is_file() { Photo::Stored } else { Photo::ToTake };
            self.known.insert(k.clone(), Entry { file, photo });
        }
        let e = self.known.get_mut(&k)?;
        match &mut e.photo {
            Photo::Shown { tex, w, h, used } => {
                *used = now;
                return Some((*tex, *w, *h));
            }
            Photo::Stored => {
                e.photo = Photo::Reading;
                let file = e.file.clone();
                self.read(k, file);
            }
            Photo::ToTake => {
                if !self.asked.contains(&k) {
                    self.asked.push(k);
                }
            }
            Photo::Reading | Photo::Failed => {}
        }
        None
    }

    /// Forget the photo of `bus` in `paint` and delete its file (a livery saved again under its
    /// name looks different now): the next tile asks for a new one.
    pub fn forget(&mut self, root: &str, bus: &str, paint: &str) {
        let _ = std::fs::remove_file(photo_file(root, bus, paint));
        let k: Wanted = (bus.to_string(), paint.to_string());
        self.known.remove(&k);
    }

    /// Put every one of `buses` in line ("Update bus pictures"): those without a photo are
    /// photographed after what is on screen, while the launcher is idle.
    pub fn queue_all(&mut self, root: &str, buses: &[String]) {
        self.backlog.clear();
        for b in buses {
            let k: Wanted = (b.clone(), String::new());
            let missing = match self.known.get(&k) {
                Some(e) => matches!(e.photo, Photo::ToTake),
                None => !photo_file(root, b, "").is_file(),
            };
            if missing {
                self.backlog.push_back(k);
            }
        }
        self.backlog_total = self.backlog.len();
    }

    /// The bus being photographed now.
    pub fn busy_with(&self) -> Option<&str> {
        self.shooting.as_ref().map(|s| s.0 .0.as_str())
    }

    /// Stop going through every bus.
    pub fn stop_all(&mut self) {
        self.backlog.clear();
        self.backlog_total = 0;
    }

    /// Going through every bus: how many are done of how many.
    pub fn progress(&self) -> Option<(usize, usize)> {
        (self.backlog_total > 0).then(|| (self.backlog_total - self.backlog.len(), self.backlog_total))
    }

    fn read(&mut self, k: Wanted, file: PathBuf) {
        let (tx, _) = self.reader.get_or_insert_with(|| {
            let (tx, rx_paths) = channel::<(Wanted, PathBuf)>();
            let (tx_done, rx) = channel();
            std::thread::spawn(move || {
                for (k, p) in rx_paths {
                    let img = image::open(&p).ok().map(|i| i.to_rgba8());
                    if tx_done.send((k, img)).is_err() {
                        break;
                    }
                }
            });
            (tx, rx)
        });
        let _ = tx.send((k, file));
    }

    /// The next bus to photograph: the first tile on screen without a photo, else the next of
    /// "Update bus pictures".
    fn next(&mut self, root: &str) -> Option<Wanted> {
        let asked = std::mem::take(&mut self.asked);
        if let Some(k) = asked.into_iter().find(|k| self.known.get(k).is_some_and(|e| matches!(e.photo, Photo::ToTake))) {
            return Some(k);
        }
        while let Some(k) = self.backlog.pop_front() {
            let e = self.known.entry(k.clone()).or_insert_with(|| {
                let file = photo_file(root, &k.0, &k.1);
                let photo = if file.is_file() { Photo::Stored } else { Photo::ToTake };
                Entry { file, photo }
            });
            if matches!(e.photo, Photo::ToTake) {
                return Some(k);
            }
        }
        if self.shooting.is_none() {
            self.backlog_total = 0;
        }
        None
    }
}

/// Once a frame on the bus step, before the tiles are drawn: the photos read from the disk go
/// to the GPU, the studio goes on with the bus it is photographing (or, idle, starts on the
/// next), and the photos drawn longest ago leave the GPU when there are too many.
pub fn work(l: &mut Launcher) {
    let now = l.ui.time;
    let dt = l.ui.dt;
    // (idle: no mouse or key for a moment, nothing dragged, the chosen bus not being read)
    let idle = l.last_input.elapsed().as_secs_f32() > 0.6 && l.dragging.is_none() && !l.showroom.busy;
    let root = l.state.config.root.clone();
    let map = l.state.choice.map.clone();
    let Launcher { renderer, gpu, showroom, .. } = l;
    let (Some(renderer), Some(gpu)) = (renderer.as_mut(), gpu.as_mut()) else { return };
    let photos = &mut showroom.photos;
    // what the disk gave
    let mut read = Vec::new();
    if let Some((_, rx)) = photos.reader.as_ref() {
        while let Ok(x) = rx.try_recv() {
            read.push(x);
        }
    }
    for (k, img) in read {
        let Some(e) = photos.known.get_mut(&k) else { continue };
        match img {
            Some(img) => {
                let (w, h) = img.dimensions();
                let tex = gpu.add_image(&renderer.device, &renderer.queue, w, h, img.as_raw());
                e.photo = Photo::Shown { tex, w, h, used: now };
                photos.on_gpu += 1;
            }
            // (a file that does not decode is taken again)
            None => e.photo = Photo::ToTake,
        }
    }
    // the studio
    if let Some((k, look, spent)) = photos.shooting.clone() {
        let spent = spent + dt;
        if let Some(s) = photos.shooting.as_mut() {
            s.2 = spent;
        }
        let studio = photos.studio.get_or_insert_with(|| Box::new(Showroom::studio()));
        studio.update(renderer, dt);
        if studio.shows(&look) {
            let picture = studio.photograph(renderer, PHOTO_W, PHOTO_H);
            studio.forget();
            photos.shooting = None;
            if let Some(e) = photos.known.get_mut(&k) {
                match picture {
                    Some(img) => {
                        let (w, h) = img.dimensions();
                        let tex = gpu.add_image(&renderer.device, &renderer.queue, w, h, img.as_raw());
                        e.photo = Photo::Shown { tex, w, h, used: now };
                        photos.on_gpu += 1;
                        let file = e.file.clone();
                        log::info!("bus picker: photographed {} in {spent:.1} s", k.0);
                        // (written on a thread: a PNG takes a moment to make)
                        std::thread::spawn(move || {
                            let _ = std::fs::create_dir_all(cache_dir());
                            if let Err(e) = img.save_with_format(&file, image::ImageFormat::Png) {
                                log::warn!("bus picker: photo not saved to {}: {e}", file.display());
                            }
                        });
                    }
                    None => e.photo = Photo::Failed,
                }
            }
        } else if studio.gave_up(&look) || spent > GIVE_UP_S {
            log::warn!("bus picker: no photo of {}: {}", k.0, studio.error.clone().unwrap_or_else(|| "it took too long".into()));
            if let Some(e) = photos.known.get_mut(&k) {
                e.photo = Photo::Failed;
            }
            // (a studio still reading the bus is dropped with it: what it reads is not waited for)
            photos.studio = None;
            photos.shooting = None;
        }
    } else if idle && !map.is_empty() {
        if let Some(k) = photos.next(&root) {
            let look = Look { root: PathBuf::from(&root), map, bus: k.0.clone(), paint: k.1.clone(), weather: String::new(), time: PHOTO_TIME, date: PHOTO_DATE.to_string() };
            photos.studio.get_or_insert_with(|| Box::new(Showroom::studio())).want(look.clone());
            photos.shooting = Some((k, look, 0.0));
        }
    }
    photos.asked.clear();
    // too many on the GPU: those not drawn for a while go back to the disk
    if photos.on_gpu > ON_GPU {
        let mut shown: Vec<(f32, Wanted)> = photos.known.iter().filter_map(|(k, e)| match e.photo {
            Photo::Shown { used, .. } if now - used > 0.5 => Some((used, k.clone())),
            _ => None,
        }).collect();
        shown.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, k) in shown.into_iter().take(photos.on_gpu - ON_GPU) {
            if let Some(e) = photos.known.get_mut(&k) {
                if let Photo::Shown { tex, .. } = e.photo {
                    gpu.free(tex);
                    e.photo = Photo::Stored;
                    photos.on_gpu -= 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_photo_is_named_after_the_bus_its_paint_and_its_time() {
        let a = photo_name("C:\\OMSI 2\\Vehicles\\MAN_SD200\\SD200.bus", "", 1);
        assert_eq!(a, photo_name("c:/omsi 2/vehicles/man_sd200/sd200.bus", "", 1), "the same file however it is written");
        assert_ne!(a, photo_name("C:\\OMSI 2\\Vehicles\\MAN_SD200\\SD200.bus", "BVG", 1), "another paint, another photo");
        assert_ne!(a, photo_name("C:\\OMSI 2\\Vehicles\\MAN_SD200\\SD200.bus", "", 2), "a changed bus is photographed again");
        assert!(a.ends_with(".png") && a.len() == 20);
    }

    #[test]
    fn the_tiles_on_screen_are_photographed_before_the_rest() {
        let mut p = Photos::default();
        for b in ["a.bus", "b.bus", "c.bus"] {
            p.known.insert((b.into(), String::new()), Entry { file: PathBuf::from(b), photo: Photo::ToTake });
        }
        p.backlog = [("a.bus".to_string(), String::new()), ("b.bus".to_string(), String::new())].into();
        p.backlog_total = 2;
        p.asked = vec![("c.bus".into(), String::new())];
        assert_eq!(p.next("").map(|k| k.0), Some("c.bus".into()));
        assert!(p.asked.is_empty(), "what was asked is asked again by the next frame's tiles");
        assert_eq!(p.next("").map(|k| k.0), Some("a.bus".into()));
        // (one already photographed is passed over)
        p.known.get_mut(&("b.bus".to_string(), String::new())).unwrap().photo = Photo::Stored;
        assert_eq!(p.next(""), None);
        assert_eq!(p.progress(), None, "done: nothing left to count");
    }
}
