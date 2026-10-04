//! Verification harness: point it at an OMSI 2 installation and it loads everything.

use anyhow::Result;
use clap::Parser;
use omsi_cfg::{resolve_path, CfgFile};
use omsi_script::{compile, CompileInput, NullHost, State, Vm};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use walkdir::WalkDir;

#[derive(Parser)]
struct Args {
    /// Path to the OMSI 2 installation (the folder containing OMSI).
    root: PathBuf,
    /// Only run the named checks (o3d, scripts, models, scenery, vehicles, maps, misc,
    /// textures, fleet). `fleet` is not part of the default set.
    #[arg(long)]
    only: Vec<String>,
    /// Print every error instead of a summary.
    #[arg(long, short)]
    verbose: bool,
    /// A content folder laid out like OMSI 2 (the game's mod folder), searched before the
    /// installation like the game does. `--only fleet` then checks the vehicles in it.
    #[arg(long)]
    content: Vec<PathBuf>,
    /// `fleet`: only vehicles whose path contains this text.
    #[arg(long)]
    vehicles: Option<String>,
    /// `fleet`: also start every vehicle (Shift+U) and run its scripts for 20 s.
    #[arg(long)]
    run: bool,
    /// Write a fingerprint of what every content file (installation and `--content`) parses
    /// to, one line per file, and do nothing else: two dumps from two builds differ exactly
    /// in the files a loader change affects.
    #[arg(long)]
    dump: Option<PathBuf>,
    /// With `--dump-out`: print the whole parsed structure of the files listed in this file
    /// (paths as `--dump` writes them).
    #[arg(long)]
    dump_detail: Option<PathBuf>,
    #[arg(long)]
    dump_out: Option<PathBuf>,
    /// Resolve a file name as a content file writes it (`--resolve <folder> <name>`, the
    /// folder relative to the installation) and print where it is found - as a path and as
    /// a texture of that folder.
    #[arg(long, num_args = 2)]
    resolve: Vec<String>,
}

mod dump;
mod fleet;

fn files_with_ext(root: &Path, exts: &[&str]) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            exts.contains(&ext.as_str())
        })
        .map(|e| e.into_path())
        .collect()
}

fn check_o3d(root: &Path, verbose: bool) {
    let files = files_with_ext(root, &["o3d", "x"]);
    let ok = AtomicUsize::new(0);
    let errors: Vec<String> = files
        .par_iter()
        .filter_map(|p| match omsi_o3d::load_mesh(p) {
            Ok(_) => {
                ok.fetch_add(1, Ordering::Relaxed);
                None
            }
            Err(e) => Some(format!("{}: {e}", p.strip_prefix(root).unwrap_or(p).display())),
        })
        .collect();
    println!("[meshes] {} ok, {} failed of {}", ok.load(Ordering::Relaxed), errors.len(), files.len());
    if verbose {
        for e in &errors {
            println!("  {e}");
        }
    } else {
        for e in errors.iter().take(10) {
            println!("  {e}");
        }
    }
}

/// Builtin variables the road-vehicle host provides (program/varlist_roadvehicle.txt + generated).
fn roadvehicle_builtins(root: &Path) -> (Vec<String>, Vec<String>) {
    let mut vars = Vec::new();
    if let Ok(f) = CfgFile::read(root.join("program/varlist_roadvehicle.txt")) {
        vars.extend(f.lines.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()));
    }
    let mut svars = Vec::new();
    if let Ok(f) = CfgFile::read(root.join("program/stringvarlist_roadvehicle.txt")) {
        svars.extend(f.lines.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()));
    }
    (vars, svars)
}

fn scenobj_builtins(root: &Path) -> Vec<String> {
    let mut vars = Vec::new();
    if let Ok(f) = CfgFile::read(root.join("program/varlist_scenobj.txt")) {
        vars.extend(f.lines.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()));
    }
    vars
}

/// Extract the script set of a .bus/.ovh/.sco file.
fn script_set(file: &CfgFile) -> CompileInput {
    let mut inp = CompileInput::default();
    let base = file.dir();
    let mut r = file.reader();
    while let Some(k) = r.next_keyword() {
        let target = match k.as_str() {
            "varnamelist" => &mut inp.varlists,
            "stringvarnamelist" => &mut inp.stringvarlists,
            "script" => &mut inp.scripts,
            "constfile" => &mut inp.constfiles,
            _ => continue,
        };
        let n = r.usize();
        for _ in 0..n {
            let rel = r.str();
            if !rel.trim().is_empty() {
                target.push(resolve_path(base, rel));
            }
        }
    }
    inp
}

fn check_scripts(root: &Path, verbose: bool) {
    // the engine's variable lists live in the installation's `program` folder: a mod
    // folder has none, and every script then failed on OMSI's own variables
    let program_root = if root.join("program").is_dir() {
        root.to_path_buf()
    } else {
        std::env::var_os("HOME")
            .and_then(|h| std::fs::read_to_string(Path::new(&h).join(".openomsi-root")).ok())
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| p.join("program").is_dir())
            .unwrap_or_else(|| root.to_path_buf())
    };
    let (rv_vars, rv_svars) = roadvehicle_builtins(&program_root);
    let so_vars = scenobj_builtins(&program_root);
    let files = files_with_ext(root, &["bus", "ovh", "sco"]);
    let objects = AtomicUsize::new(0);
    let clean = AtomicUsize::new(0);
    let scripts_seen = AtomicUsize::new(0);
    let errors: Vec<String> = files
        .par_iter()
        .flat_map(|p| {
            let f = match CfgFile::read(p) {
                Ok(f) => f,
                Err(e) => return vec![e.to_string()],
            };
            let mut inp = script_set(&f);
            if inp.scripts.is_empty() {
                return Vec::new();
            }
            objects.fetch_add(1, Ordering::Relaxed);
            scripts_seen.fetch_add(inp.scripts.len(), Ordering::Relaxed);
            let is_veh = !p.extension().map(|e| e.eq_ignore_ascii_case("sco")).unwrap_or(false);
            if is_veh {
                inp.builtin_vars = rv_vars.clone();
                inp.builtin_str_vars = rv_svars.clone();
                for a in 0..8 {
                    for side in ["L", "R"] {
                        for pre in ["Wheel_Rotation_", "Wheel_RotationSpeed_", "Axle_Steering_", "Axle_Suspension_", "Axle_Springfactor_", "Axle_Brakeforce_", "Axle_SurfaceID_"] {
                            inp.builtin_vars.push(format!("{pre}{a}_{side}"));
                        }
                    }
                    inp.builtin_vars.push(format!("PAX_Entry{a}_Open"));
                    inp.builtin_vars.push(format!("PAX_Entry{a}_Req"));
                    inp.builtin_vars.push(format!("PAX_Exit{a}_Open"));
                    inp.builtin_vars.push(format!("PAX_Exit{a}_Req"));
                }
                for i in 0..6 {
                    inp.builtin_vars.push(format!("Debug_{i}"));
                }
            } else {
                inp.builtin_vars = so_vars.clone();
            }
            let prog = compile(&inp);
            // smoke-run init + one frame with a null host
            let mut vm = Vm::new();
            let mut st = State::new(&prog);
            vm.run_init(&prog, &mut st, &mut NullHost);
            vm.run_frame(&prog, &mut st, &mut NullHost);
            vm.run_frame_ai(&prog, &mut st, &mut NullHost);
            if prog.errors.is_empty() {
                clean.fetch_add(1, Ordering::Relaxed);
            }
            let rel = p.strip_prefix(root).unwrap_or(p).display().to_string();
            prog.errors.iter().map(|e| format!("{rel}: {}", e.to_string().replace(&root.display().to_string(), "."))).collect()
        })
        .collect();
    println!(
        "[scripts] {} objects with scripts ({} script files), {} compiled without errors, {} errors",
        objects.load(Ordering::Relaxed),
        scripts_seen.load(Ordering::Relaxed),
        clean.load(Ordering::Relaxed),
        errors.len()
    );
    let limit = if verbose { usize::MAX } else { 25 };
    for e in errors.iter().take(limit) {
        println!("  {e}");
    }
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    let root = args.root.canonicalize()?;
    // the same content roots as the game: mod folders first, then the installation
    // (a `.zip` is read in place, as the game mounts archives)
    let mut content: Vec<PathBuf> = Vec::new();
    for c in args.content.iter().filter_map(|c| c.canonicalize().ok()) {
        if c.is_file() && c.extension().map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false) {
            match omsi_cfg::vfs::add_content_zip(&c) {
                Ok(m) => content.push(m),
                Err(e) => eprintln!("{}: {e}", c.display()),
            }
        } else {
            omsi_cfg::add_content_root(c.clone());
            content.push(c);
        }
    }
    omsi_cfg::add_content_root(root.clone());
    if let [dir, name] = args.resolve.as_slice() {
        // the folder under the first root that has it (mods and archives first)
        let base = content.iter().chain(std::iter::once(&root)).map(|r| resolve_path(r, dir)).find(|p| omsi_cfg::vfs::is_dir(p)).unwrap_or_else(|| root.join(dir));
        let p = resolve_path(&base, name);
        println!("path    {:?} in {} -> {} (exists: {})", name, base.display(), p.display(), omsi_cfg::vfs::is_file(&p));
        let tex = omsi_texture::find_texture(name, &[base.as_path(), base.join("texture").as_path()]);
        println!("texture {:?} -> {:?}", name, tex);
        return Ok(());
    }
    if args.dump.is_some() || args.dump_detail.is_some() {
        let (rv, rvs) = roadvehicle_builtins(&root);
        let _ = dump::BUILTINS.set((rv, rvs, scenobj_builtins(&root)));
        let mut roots = content.clone();
        roots.push(root.clone());
        if let Some(out) = &args.dump {
            dump::dump(&roots, out)?;
        }
        if let (Some(list), Some(out)) = (&args.dump_detail, &args.dump_out) {
            dump::dump_detail(&roots, list, out)?;
        }
        return Ok(());
    }
    if args.only.iter().any(|o| o == "fleet") {
        fleet::check_fleet(&root, &content, args.vehicles.as_deref(), args.run, args.verbose);
    }
    let want = |n: &str| args.only.is_empty() || args.only.iter().any(|o| o == n);
    if want("o3d") {
        check_o3d(&root, args.verbose);
    }
    if want("scripts") {
        check_scripts(&root, args.verbose);
    }
    if want("models") {
        check_models(&root, args.verbose);
    }
    if want("scenery") {
        check_scenery(&root, args.verbose);
    }
    if want("vehicles") {
        check_vehicles(&root, args.verbose);
    }
    if want("maps") {
        check_maps(&root, args.verbose);
    }
    if want("misc") {
        check_misc(&root, args.verbose);
    }
    if want("textures") {
        check_textures(&root, args.verbose);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Content loaders
// ---------------------------------------------------------------------------------------------

fn report(name: &str, total: usize, errors: &[String], verbose: bool) {
    println!("[{name}] {} ok, {} failed of {total}", total - errors.len(), errors.len());
    let limit = if verbose { usize::MAX } else { 12 };
    for e in errors.iter().take(limit) {
        println!("  {e}");
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

fn check_models(root: &Path, verbose: bool) {
    // model.cfg files are those referenced by [model] in .bus/.ovh/.sco/.hum; approximate by
    // every .cfg under a "model" directory plus vehicle Model dirs.
    let files: Vec<PathBuf> = files_with_ext(root, &["cfg"])
        .into_iter()
        .filter(|p| {
            let s = p.to_string_lossy().to_ascii_lowercase();
            (s.contains("/model/") || s.contains("\\model\\")) && !s.contains("passengercabin") && !s.contains("paths.cfg")
        })
        .collect();
    let errors: Vec<String> = files
        .par_iter()
        .filter_map(|p| match omsi_model::Model::load(p) {
            Ok(m) => {
                let unknown: Vec<String> = m.unknown_keywords.iter().filter(|(k, _)| !matches!(k.as_str(), "passpos" | "drivpos" | "entry" | "exit" | "pathpnt" | "pathlink" | "stepsoundpack" | "texchanges" | "newtexchangemaster" | "entries" | "stamper" | "ticket_sale" | "ticket_sale_money_point" | "ticket_sale_change_point" | "linktonextveh" | "linktoprevveh" | "next_roomheight" | "next_stepsound" | "pathlink_oneway" | "ticket_sale_money_point_2" | "ticket_sale_change_point_2")).map(|(k, l)| format!("[{k}]@{l}")).collect();
                if unknown.is_empty() && !m.meshes.is_empty() {
                    None
                } else if m.meshes.is_empty() && unknown.is_empty() {
                    None
                } else {
                    Some(format!("{}: unknown keywords {}", rel(root, p), unknown.join(" ")))
                }
            }
            Err(e) => Some(format!("{}: {e}", rel(root, p))),
        })
        .collect();
    report("model.cfg", files.len(), &errors, verbose);
}

fn check_scenery(root: &Path, verbose: bool) {
    let files = files_with_ext(root, &["sco"]);
    let ignored = std::sync::atomic::AtomicUsize::new(0);
    // a `[stringvarnamelist]` file that is not there: the object's fields (the values the
    // map fills its string variables with) have no names - as in Omsi.exe, which then
    // shows them nameless too
    let nameless = std::sync::atomic::AtomicUsize::new(0);
    let errors: Vec<String> = files
        .par_iter()
        .filter_map(|p| match omsi_scenery::SceneryObject::load(p) {
            Ok(o) => {
                if o.scripts.stringvarlists.iter().any(|f| !f.exists()) {
                    nameless.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                // `[collisionmesh]` is a misspelling OMSI ignores as well (it knows only
                // `[collision_mesh]`): the original's own behaviour, noted, not a failure
                let ignored_by_omsi = |k: &str| k == "collisionmesh";
                let mut unk: Vec<String> = o.unknown_keywords.iter().filter(|(k, _)| !ignored_by_omsi(k)).map(|(k, l)| format!("[{k}]@{l}")).collect();
                unk.extend(o.model.unknown_keywords.iter().map(|(k, l)| format!("[{k}]@{l}")));
                if o.unknown_keywords.iter().any(|(k, _)| ignored_by_omsi(k)) {
                    ignored.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                if unk.is_empty() {
                    None
                } else {
                    Some(format!("{}: unknown keywords {}", rel(root, p), unk.join(" ")))
                }
            }
            Err(e) => Some(format!("{}: {e}", rel(root, p))),
        })
        .collect();
    report("sco", files.len(), &errors, verbose);
    let n = ignored.load(std::sync::atomic::Ordering::Relaxed);
    if n > 0 {
        println!("  note: {n} objects spell [collisionmesh], which Omsi.exe ignores too (no collision shape)");
    }
    let n = nameless.load(std::sync::atomic::Ordering::Relaxed);
    if n > 0 {
        println!("  note: {n} objects name a [stringvarnamelist] file that is not there: their fields have no name");
    }
    let files = files_with_ext(root, &["sli"]);
    let errors: Vec<String> = files
        .par_iter()
        .filter_map(|p| match omsi_scenery::Spline::load(p) {
            Ok(s) => {
                if s.unknown_keywords.is_empty() && !s.profiles.is_empty() {
                    None
                } else if s.unknown_keywords.is_empty() {
                    None
                } else {
                    Some(format!("{}: unknown keywords {:?}", rel(root, p), s.unknown_keywords))
                }
            }
            Err(e) => Some(format!("{}: {e}", rel(root, p))),
        })
        .collect();
    report("sli", files.len(), &errors, verbose);
}

fn check_vehicles(root: &Path, verbose: bool) {
    let files = files_with_ext(root, &["bus", "ovh"]);
    let errors: Vec<String> = files
        .par_iter()
        .filter_map(|p| match omsi_vehicle::Vehicle::load(p) {
            Ok(v) => {
                if v.unknown_keywords.is_empty() {
                    None
                } else {
                    Some(format!("{}: unknown keywords {:?}", rel(root, p), v.unknown_keywords))
                }
            }
            Err(e) => Some(format!("{}: {e}", rel(root, p))),
        })
        .collect();
    report("bus/ovh", files.len(), &errors, verbose);
    let files = files_with_ext(root, &["hof"]);
    let errors: Vec<String> = files
        .par_iter()
        .filter_map(|p| match omsi_vehicle::Hof::load(p) {
            Ok(h) => {
                if h.termini.is_empty() {
                    Some(format!("{}: no termini", rel(root, p)))
                } else {
                    None
                }
            }
            Err(e) => Some(format!("{}: {e}", rel(root, p))),
        })
        .collect();
    report("hof", files.len(), &errors, verbose);
    let files: Vec<PathBuf> = files_with_ext(root, &["cfg"]).into_iter().filter(|p| p.to_string_lossy().to_ascii_lowercase().contains("sound")).collect();
    let errors: Vec<String> = files
        .par_iter()
        .filter_map(|p| match omsi_vehicle::SoundCfg::load(p) {
            Ok(s) => {
                if s.unknown_keywords.is_empty() {
                    None
                } else {
                    Some(format!("{}: unknown keywords {:?}", rel(root, p), s.unknown_keywords))
                }
            }
            Err(e) => Some(format!("{}: {e}", rel(root, p))),
        })
        .collect();
    report("sound.cfg", files.len(), &errors, verbose);
    let files = files_with_ext(root, &["zug"]);
    let errors: Vec<String> = files.par_iter().filter_map(|p| omsi_vehicle::vehicle::Train::load(p).err().map(|e| format!("{}: {e}", rel(root, p)))).collect();
    report("zug", files.len(), &errors, verbose);
}

fn check_maps(root: &Path, verbose: bool) {
    let globals: Vec<PathBuf> = files_with_ext(root, &["cfg"]).into_iter().filter(|p| p.file_name().map(|f| f.eq_ignore_ascii_case("global.cfg")).unwrap_or(false)).collect();
    let mut errors = Vec::new();
    let mut tiles_total = 0;
    for g in &globals {
        match omsi_map::GlobalCfg::load(g) {
            Ok(gc) => {
                if !gc.unknown_keywords.is_empty() {
                    errors.push(format!("{}: unknown keywords {:?}", rel(root, g), gc.unknown_keywords));
                }
                let dir = gc.dir().to_path_buf();
                let tile_errors: Vec<String> = gc
                    .tiles
                    .par_iter()
                    .filter_map(|t| {
                        let p = omsi_cfg::resolve_path(&dir, &t.file);
                        if !p.exists() {
                            // The original silently treats a listed-but-missing tile as empty.
                            return None;
                        }
                        match omsi_map::Tile::load(&p) {
                            Ok(tile) => {
                                let mut errs = Vec::new();
                                if !tile.unknown_keywords.is_empty() {
                                    errs.push(format!("{}: unknown keywords {:?}", rel(root, &p), tile.unknown_keywords));
                                }
                                let terrain = p.with_extension("map.terrain");
                                if terrain.exists() {
                                    if let Err(e) = omsi_map::Terrain::load(&terrain) {
                                        errs.push(format!("{}: {e}", rel(root, &terrain)));
                                    }
                                }
                                if errs.is_empty() {
                                    None
                                } else {
                                    Some(errs.join("\n  "))
                                }
                            }
                            Err(e) => Some(format!("{}: {e}", rel(root, &p))),
                        }
                    })
                    .collect();
                tiles_total += gc.tiles.len();
                errors.extend(tile_errors);
                // calendar, ai lists, timetable
                let cal = dir.join("Holidays.txt");
                if cal.exists() {
                    if let Err(e) = omsi_map::Calendar::load(&cal) {
                        errors.push(e.to_string());
                    }
                }
                let ai = dir.join("ailists.cfg");
                if ai.exists() {
                    match omsi_map::AiLists::load(&ai) {
                        Ok(a) => {
                            if a.groups.is_empty() {
                                errors.push(format!("{}: no AI groups", rel(root, &ai)));
                            }
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                let tt = omsi_timetable::TimetableData::load(&dir);
                errors.extend(tt.errors.iter().cloned());
                println!(
                    "  map {}: {} tiles, {} bus stops, {} station links, {} trips, {} tracks, {} lines",
                    gc.name,
                    gc.tiles.len(),
                    tt.bus_stops.len(),
                    tt.stn_links.len(),
                    tt.trips.len(),
                    tt.tracks.len(),
                    tt.lines.len()
                );
            }
            Err(e) => errors.push(format!("{}: {e}", rel(root, g))),
        }
    }
    report("maps", globals.len() + tiles_total, &errors, verbose);
    // Chrono tiles
    let chrono: Vec<PathBuf> = files_with_ext(root, &["map"]).into_iter().filter(|p| p.to_string_lossy().contains("Chrono")).collect();
    let errors: Vec<String> = chrono
        .par_iter()
        .filter_map(|p| match omsi_map::Tile::load(p) {
            Ok(t) => {
                if t.unknown_keywords.is_empty() {
                    None
                } else {
                    Some(format!("{}: unknown keywords {:?}", rel(root, p), t.unknown_keywords))
                }
            }
            Err(e) => Some(format!("{}: {e}", rel(root, p))),
        })
        .collect();
    report("chrono tiles", chrono.len(), &errors, verbose);
}

fn check_misc(root: &Path, verbose: bool) {
    macro_rules! simple {
        ($name:expr, $exts:expr, $load:expr) => {{
            let files = files_with_ext(root, $exts);
            let errors: Vec<String> = files.par_iter().filter_map(|p| ($load)(p).map(|e: String| format!("{}: {e}", rel(root, p)))).collect();
            report($name, files.len(), &errors, verbose);
        }};
    }
    simple!("owt", &["owt"], |p: &PathBuf| omsi_content::Weather::load(p).err().map(|e| e.to_string()));
    simple!("oft", &["oft"], |p: &PathBuf| match omsi_content::Font::load_all(p) {
        Ok(f) if !f.is_empty() && f.iter().all(|f| !f.chars.is_empty()) => None,
        Ok(_) => Some("no fonts/chars".to_string()),
        Err(e) => Some(e.to_string()),
    });
    simple!("olf", &["olf"], |p: &PathBuf| match omsi_content::Language::load(p) {
        Ok(l) if !l.strings.is_empty() => None,
        Ok(_) => Some("empty".to_string()),
        Err(e) => Some(e.to_string()),
    });
    simple!("otp", &["otp"], |p: &PathBuf| match omsi_content::TicketPack::load(p) {
        Ok(t) if !t.tickets.is_empty() => None,
        Ok(_) => Some("no tickets".to_string()),
        Err(e) => Some(e.to_string()),
    });
    simple!("cti", &["cti"], |p: &PathBuf| omsi_content::tickets::TicketItems::load(p).err().map(|e| e.to_string()));
    simple!("hum", &["hum"], |p: &PathBuf| match omsi_content::Human::load(p) {
        Ok(h) if !h.model.is_empty() && h.links.len() == 22 => None,
        Ok(_) => Some("incomplete".to_string()),
        Err(e) => Some(e.to_string()),
    });
    simple!("odr", &["odr"], |p: &PathBuf| omsi_content::Driver::load(p).err().map(|e| e.to_string()));
    simple!("osn", &["osn"], |p: &PathBuf| omsi_content::Situation::load(p).err().map(|e| e.to_string()));
    simple!("oop", &["oop"], |p: &PathBuf| match omsi_content::Options::load(p) {
        Ok(o) if o.values.len() > 10 => None,
        Ok(o) => Some(format!("only {} options", o.values.len())),
        Err(e) => Some(e.to_string()),
    });
    simple!("ocu", &["ocu"], |p: &PathBuf| omsi_timetable::CarUse::load(p).err().map(|e| e.to_string()));
    let money: Vec<PathBuf> = files_with_ext(&root.join("Money"), &["cfg"]);
    let errors: Vec<String> = money.par_iter().filter_map(|p| match omsi_content::Currency::load(p) {
        Ok(c) if !c.coins.is_empty() => None,
        Ok(_) => Some(format!("{}: no coins", rel(root, p))),
        Err(e) => Some(format!("{}: {e}", rel(root, p))),
    }).collect();
    report("money", money.len(), &errors, verbose);
    let mut errors = Vec::new();
    match omsi_content::Envir::load(&root.join("envir.cfg")) {
        Ok(e) if !e.sky_textures[0].is_empty() => {}
        Ok(_) => errors.push("envir.cfg: no sky textures".into()),
        Err(e) => errors.push(e.to_string()),
    }
    match omsi_content::KeyboardCfg::load(&root.join("Inputs/keyboard.cfg")) {
        Ok(k) if !k.game.is_empty() && !k.vehicles.is_empty() => {}
        Ok(_) => errors.push("keyboard.cfg: empty".into()),
        Err(e) => errors.push(e.to_string()),
    }
    if let Err(e) = omsi_content::input::load_game_controllers(&root.join("Inputs/gamectrler.cfg")) {
        errors.push(e.to_string());
    }
    match omsi_content::Options::load(&root.join("options.cfg")) {
        Ok(o) if o.values.len() > 10 => {}
        Ok(_) => errors.push("options.cfg: too few".into()),
        Err(e) => errors.push(e.to_string()),
    }
    report("root cfg", 4, &errors, verbose);
}

fn check_textures(root: &Path, verbose: bool) {
    let files = files_with_ext(root, &["dds", "bmp", "tga", "jpg", "png"]);
    let errors: Vec<String> = files
        .par_iter()
        .filter_map(|p| match omsi_texture::decode_file(p) {
            Ok(img) if img.width > 0 && img.height > 0 => None,
            Ok(_) => Some(format!("{}: empty image", rel(root, p))),
            Err(e) => Some(e.to_string().replace(&root.display().to_string(), ".")),
        })
        .collect();
    report("textures", files.len(), &errors, verbose);
}
