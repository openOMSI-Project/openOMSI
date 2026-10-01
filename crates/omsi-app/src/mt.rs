//! Machine translation of the interface (the `machine_translation` setting): every text the
//! translation tables (`locales/app.yml`) and OMSI's language files do not have is
//! translated on this machine by Meta's NLLB-200 model (600M, int8) run with CTranslate2 -
//! what the `trad` crate does - and kept in `~/.openomsi/cache/mt-<lang>.json`, so a
//! text is translated once and then read from there.
//!
//! Nothing waits for it: `omsi_ui::tr` asks `lookup`, which answers from the cache or puts
//! the text in the queue and says English for now; a thread translates the queue in batches
//! and the label is drawn in the language a moment later. The model (~620 MB) is fetched
//! from Hugging Face the first time and loaded only while there is something to translate
//! (it takes ~700 MB of memory; unloaded after a minute of nothing to do).

use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const MODEL_REPO: &str = "JustFrederik/nllb-200-distilled-600M-ct2-int8";
const MODEL_FILES: [&str; 4] = ["config.json", "shared_vocabulary.txt", "tokenizer.json", "model.bin"];
/// The model repository's revision fetched, and each file's SHA-256 there (checked before a
/// download is kept).
const MODEL_REVISION: &str = "302d78f00e6fdb50a1064059df7c392b735e9d05";
const MODEL_SHA256: [&str; 4] = [
    "0c2f6fa2057c7264d052fb4a62ba3476eeae70487acddfa8e779a53a00cbf44c",
    "a132a83330f45514c2476eb81d1d69b3c41762264d16ce0a7ea982e5d6c728e5",
    "e316b82de11d0f951f370943b3c438311629547285129b0b81dadabd01bca665",
    "ed1beaf75134de7505315a5223162f56acff397eff6b50638a500d3936fe707b",
];

#[derive(Default)]
struct Mt {
    /// Translations by interface language (`ru`, `de`, `fr`).
    cache: HashMap<String, HashMap<String, String>>,
    asked: HashSet<(String, String)>,
    queue: VecDeque<(String, String)>,
    loaded: HashSet<String>,
}

fn state() -> &'static (Mutex<Mt>, Condvar) {
    static S: OnceLock<(Mutex<Mt>, Condvar)> = OnceLock::new();
    S.get_or_init(|| (Mutex::new(Mt::default()), Condvar::new()))
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static WORKER: std::sync::Once = std::sync::Once::new();
/// What the translation is doing, for the launcher's settings page.
static STATUS: Mutex<String> = Mutex::new(String::new());

pub fn status() -> String {
    STATUS.lock().clone()
}

fn set_status(s: impl Into<String>) {
    *STATUS.lock() = s.into();
}

fn data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".openomsi"))
}

fn model_dir() -> Option<PathBuf> {
    Some(data_dir()?.join("models").join("nllb-200-distilled-600M-int8"))
}

fn cache_file(lang: &str) -> Option<PathBuf> {
    Some(data_dir()?.join("cache").join(format!("mt-{lang}.json")))
}

/// NLLB's name of an interface language.
fn nllb(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "ru" => "rus_Cyrl",
        "de" => "deu_Latn",
        "fr" => "fra_Latn",
        "uk" => "ukr_Cyrl",
        "pl" => "pol_Latn",
        "cs" => "ces_Latn",
        "es" => "spa_Latn",
        "it" => "ita_Latn",
        "be" => "bel_Cyrl",
        "kk" => "kaz_Cyrl",
        "hu" => "hun_Latn",
        "pt" | "pt-pt" => "por_Latn",
        "nl" => "nld_Latn",
        "tr" => "tur_Latn",
        "zh-tw" | "zh-hk" => "zho_Hant",
        "ko" => "kor_Hang",
        "th" => "tha_Thai",
        "vi" => "vie_Latn",
        "id" => "ind_Latn",
        "ms" => "zsm_Latn",
        "tl" => "tgl_Latn",
        "ja" => "jpn_Jpan",
        "zh" | "zh-cn" => "zho_Hans",
        "hi" => "hin_Deva",
        _ => return None,
    })
}

/// Turn machine translation on or off (the setting).
pub fn enable(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    if !on {
        omsi_ui::i18n::set_fallback(None);
        return;
    }
    if !cfg!(target_os = "macos") {
        set_status("Not available on this system yet");
        return;
    }
    omsi_ui::i18n::set_fallback(Some(lookup));
    WORKER.call_once(|| {
        let _ = std::thread::Builder::new().name("translation".into()).spawn(worker);
    });
}

/// Is a text one to translate? Words, not a number, a code, a file or a bus's name.
fn worth(text: &str) -> bool {
    let t = text.trim();
    if t.len() < 2 || t.len() > 400 {
        return false;
    }
    // (a file, a folder: never)
    if t.contains('/') || t.contains('\\') || (!t.contains(' ') && t.contains('.') && !t.ends_with('.')) {
        return false;
    }
    // (a session code or an address in it: the line is translated around it, not with it)
    if t.split_whitespace().any(|w| w.matches('-').count() >= 3 || w.starts_with("http")) {
        return false;
    }
    // at least one word of three small letters ("MAN SD200", "12:05", "IBIS" are left)
    let mut run = 0;
    for c in t.chars() {
        if c.is_ascii_lowercase() {
            run += 1;
            if run >= 3 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Names that are data, not interface: buses, maps, liveries, weather, lines, stops. They are
/// shown as their files write them ("BMC Procity 10.6 EU Diesel" came out "BMC Автомотивная
/// Процессия").
fn names() -> &'static parking_lot::RwLock<HashSet<String>> {
    static N: OnceLock<parking_lot::RwLock<HashSet<String>>> = OnceLock::new();
    N.get_or_init(Default::default)
}

/// Keep these texts as they are (see `names`).
pub fn protect<'a>(texts: impl IntoIterator<Item = &'a str>) {
    let mut n = names().write();
    for t in texts {
        n.insert(t.trim().to_string());
    }
}

/// `omsi_ui::tr`'s fallback: the translation when there is one, else asked for.
fn lookup(lang: &str, text: &str) -> Option<String> {
    if !ENABLED.load(Ordering::Relaxed) || nllb(lang).is_none() || !worth(text) || names().read().contains(text.trim()) {
        return None;
    }
    let (m, cv) = state();
    let mut mt = m.lock();
    if !mt.loaded.contains(lang) {
        mt.loaded.insert(lang.to_string());
        if let Some(saved) = cache_file(lang).and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok()) {
            log::info!("translation: {} {lang} texts from the cache", saved.len());
            mt.cache.entry(lang.to_string()).or_default().extend(saved);
        }
    }
    if let Some(t) = mt.cache.get(lang).and_then(|c| c.get(text)) {
        return Some(t.clone());
    }
    let key = (lang.to_string(), text.to_string());
    if mt.asked.len() < 20_000 && mt.asked.insert(key.clone()) {
        mt.queue.push_back(key);
        cv.notify_one();
    }
    None
}

/// NLLB's output made fit for a label.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn clean(s: &str) -> String {
    s.replace("<unk>", "").trim().to_string()
}

fn worker() {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let mut translator: Option<ct2rs::Translator<ct2rs::tokenizers::auto::Tokenizer>> = None;
        let mut idle_since = Instant::now();
        loop {
            let batch: Vec<(String, String)> = {
                let (m, cv) = state();
                let mut mt = m.lock();
                if mt.queue.is_empty() {
                    cv.wait_for(&mut mt, Duration::from_secs(5));
                }
                let Some(first) = mt.queue.front().map(|x| x.0.clone()) else {
                    drop(mt);
                    // nothing to do for a minute: the model's memory goes back
                    if translator.is_some() && idle_since.elapsed() > Duration::from_secs(60) {
                        translator = None;
                        log::info!("translation: model unloaded (nothing to translate)");
                    }
                    continue;
                };
                let mut out = Vec::new();
                let mut k = 0;
                while k < mt.queue.len() && out.len() < 24 {
                    if mt.queue[k].0 == first {
                        out.push(mt.queue.remove(k).unwrap());
                    } else {
                        k += 1;
                    }
                }
                out
            };
            if !ENABLED.load(Ordering::Relaxed) {
                continue;
            }
            idle_since = Instant::now();
            if translator.is_none() {
                let Some(dir) = fetch_model() else {
                    // (nothing to translate with: the texts stay English, asked again next start)
                    omsi_ui::i18n::set_fallback(None);
                    return;
                };
                let t0 = Instant::now();
                let cfg = ct2rs::Config { device: ct2rs::Device::CPU, compute_type: ct2rs::ComputeType::INT8, num_threads_per_replica: 2, ..Default::default() };
                match ct2rs::Translator::new(&dir, &cfg) {
                    Ok(t) => {
                        log::info!("translation: model loaded in {:.1} s", t0.elapsed().as_secs_f32());
                        set_status("Ready");
                        translator = Some(t);
                    }
                    Err(e) => {
                        log::warn!("translation: the model could not be loaded: {e}");
                        set_status(format!("The translation model could not be loaded: {e}"));
                        omsi_ui::i18n::set_fallback(None);
                        return;
                    }
                }
            }
            let lang = batch[0].0.clone();
            let Some(target) = nllb(&lang) else { continue };
            let src: Vec<String> = batch.iter().map(|(_, t)| format!("eng_Latn {t}")).collect();
            let prefix: Vec<Vec<&str>> = batch.iter().map(|_| vec![target]).collect();
            let opts = ct2rs::TranslationOptions { beam_size: 2, max_decoding_length: 256, ..Default::default() };
            let t0 = Instant::now();
            let res = translator.as_ref().unwrap().translate_batch_with_target_prefix(&src, &prefix, &opts, None);
            match res {
                Ok(r) => {
                    let (m, _) = state();
                    let mut mt = m.lock();
                    let c = mt.cache.entry(lang.clone()).or_default();
                    for ((_, text), (out, _)) in batch.iter().zip(r) {
                        let t = clean(&out);
                        if !t.is_empty() {
                            c.insert(text.clone(), t);
                        }
                    }
                    let snapshot = c.clone();
                    drop(mt);
                    log::debug!("translation: {} {lang} texts in {:.2} s", batch.len(), t0.elapsed().as_secs_f32());
                    if let Some(p) = cache_file(&lang) {
                        let _ = std::fs::create_dir_all(p.parent().unwrap());
                        if let Ok(b) = serde_json::to_vec_pretty(&snapshot) {
                            let tmp = p.with_extension("part");
                            if std::fs::write(&tmp, b).is_ok() {
                                let _ = std::fs::rename(&tmp, &p);
                            }
                        }
                    }
                }
                Err(e) => log::warn!("translation: {e}"),
            }
        }
    }
}

/// The model's folder, fetched from Hugging Face the first time.
#[allow(dead_code)]
fn fetch_model() -> Option<PathBuf> {
    let dir = model_dir()?;
    if MODEL_FILES.iter().all(|f| dir.join(f).is_file()) {
        return Some(dir);
    }
    std::fs::create_dir_all(&dir).ok()?;
    for (f, want) in MODEL_FILES.into_iter().zip(MODEL_SHA256) {
        let path = dir.join(f);
        if path.is_file() {
            continue;
        }
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REVISION}/{f}");
        log::info!("translation: fetching {url}");
        let resp = match ureq::get(&url).timeout(Duration::from_secs(1800)).call() {
            Ok(r) => r,
            Err(e) => {
                log::warn!("translation: the model could not be fetched: {e}");
                set_status("The translation model could not be downloaded");
                return None;
            }
        };
        let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let tmp = path.with_extension("part");
        let mut out = std::fs::File::create(&tmp).ok()?;
        let mut rd = resp.into_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut hash = <sha2::Sha256 as sha2::Digest>::new();
        let mut done: u64 = 0;
        let mut last = Instant::now();
        loop {
            let n = match std::io::Read::read(&mut rd, &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    log::warn!("translation: download broke off: {e}");
                    set_status("The translation model could not be downloaded");
                    return None;
                }
            };
            sha2::Digest::update(&mut hash, &buf[..n]);
            if std::io::Write::write_all(&mut out, &buf[..n]).is_err() {
                set_status("No room on the disk for the translation model");
                return None;
            }
            done += n as u64;
            if last.elapsed() > Duration::from_millis(500) && total > 1_000_000 {
                last = Instant::now();
                set_status(format!("Downloading the translation model: {} %", done * 100 / total));
            }
        }
        drop(out);
        let got: String = sha2::Digest::finalize(hash).iter().map(|b| format!("{b:02x}")).collect();
        if got != want {
            log::warn!("translation: {f} does not match its published SHA-256; removed");
            let _ = std::fs::remove_file(&tmp);
            set_status("The translation model could not be downloaded");
            return None;
        }
        std::fs::rename(&tmp, &path).ok()?;
    }
    set_status("Ready");
    Some(dir)
}

#[cfg(test)]
mod tests {
    #[test]
    fn what_is_translated() {
        assert!(super::worth("Back to my bus"));
        assert!(super::worth("Time speed x2"));
        assert!(!super::worth("MAN SD200"));
        assert!(!super::worth("12:05"));
        assert!(!super::worth("Vehicles/MAN_SD200/MAN_SD77.bus"));
        assert!(!super::worth("IBIS"));
    }
}
