//! The games the launcher started. Any number may run at once: each gets its own log
//! (`game.log`, `game-2.log`, ...), an id (passed to the game in `OMSI_INSTANCE`) and a
//! registry file `~/.openomsi/instances/<id>.json`, so that every launcher window (and
//! `--cli instances`) sees them. The game itself writes `~/.openomsi/lan/<id>.json`
//! while a LAN session runs; its code and players are shown with the instance.
//!
//! A process id is reused once its process has ended, so an entry also keeps when its
//! process started: a live process with that id is the game only if it started then too.
//! Nothing is shown as running, and nothing is stopped, on the process id alone.

use crate::install::{now_secs, pid_alive};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A finished game stays in the list this long (seconds), so its log can still be read.
const KEEP_ENDED: u64 = 30 * 60;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Instance {
    /// The launcher's name for the game (its registry and LAN status files).
    pub id: String,
    pub pid: u32,
    /// When the process started, as the system keeps it (`process_start`).
    #[serde(default)]
    pub process_started: Option<u64>,
    /// 1 for `game.log`, n for `game-n.log`.
    pub slot: u32,
    pub log: String,
    pub started: u64,
    pub map: String,
    pub bus: String,
    pub entry: Option<i32>,
    pub line: Option<String>,
    pub tour: Option<String>,
    pub profile: String,
    /// off, host, or join:<target>
    pub lan: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub ended: Option<u64>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// When Stop was pressed (the game is writing its session and leaving until it ends).
    #[serde(default)]
    pub stopping: Option<u64>,
    /// Stop had to kill it (it did not end by itself within the grace time).
    #[serde(default)]
    pub killed: bool,
    /// The game's LAN status file (role, code, players, warnings), while it runs.
    #[serde(default)]
    pub lan_status: Option<Value>,
    /// The last line of its log.
    #[serde(default)]
    pub last_line: String,
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Children of this launcher by instance id, so that they are reaped (a finished child that
/// is never waited for stays a zombie and looks alive).
static CHILDREN: Mutex<Vec<(String, std::process::Child)>> = Mutex::new(Vec::new());

fn dir() -> PathBuf {
    crate::data_dir().join("instances")
}

fn lan_dir() -> PathBuf {
    crate::data_dir().join("lan")
}

fn write(inst: &Instance) {
    let d = dir();
    let _ = std::fs::create_dir_all(&d);
    let p = d.join(format!("{}.json", inst.id));
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(inst).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Collect the exit codes of our children that finished.
fn reap() {
    let mut kids = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
    kids.retain_mut(|(id, c)| match c.try_wait() {
        Ok(Some(status)) => {
            if let Some(mut inst) = read_all().into_iter().find(|i| i.id == *id && i.ended.is_none()) {
                inst.running = false;
                inst.ended = Some(now_secs());
                inst.exit_code = status.code();
                write(&inst);
            }
            false
        }
        Ok(None) => true,
        Err(_) => false,
    });
}

fn read_all() -> Vec<Instance> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return Vec::new() };
    rd.flatten().filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false)).filter_map(|e| std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Instance>(&b).ok())).collect()
}

/// Is the game `id` a child of this launcher that has not been reaped? (Its process id
/// cannot have been reused then.)
fn is_our_child(id: &str) -> bool {
    CHILDREN.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(i, _)| i == id)
}

/// When the process `pid` started, as a number that differs between two processes that had
/// the same id (microseconds since 1970 on macOS, clock ticks since boot on Linux, the
/// creation FILETIME on Windows); None when it cannot be read (no such process, another
/// user's, or a system this does not know) and for a process that has ended but was not
/// collected by its parent yet (a zombie: it is not running, whatever its id says).
#[cfg(target_os = "macos")]
pub fn process_start(pid: u32) -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, &mut info as *mut _ as *mut libc::c_void, size) };
    (n == size && info.pbi_pid == pid && info.pbi_status != libc::SZOMB).then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

#[cfg(target_os = "linux")]
pub fn process_start(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // field 22 (field 3 is the state); the command name before them (field 2, in
    // parentheses) may hold spaces
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    if fields.next()? == "Z" {
        return None;
    }
    fields.nth(18)?.parse().ok()
}

#[cfg(windows)]
pub fn process_start(pid: u32) -> Option<u64> {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetProcessTimes(h: isize, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    const QUERY_LIMITED: u32 = 0x1000;
    unsafe {
        let h = OpenProcess(QUERY_LIMITED, 0, pid);
        if h == 0 {
            return None;
        }
        let (mut created, mut exited, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
        let ok = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(h);
        (ok && created != 0).then_some(created)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn process_start(_pid: u32) -> Option<u64> {
    None
}

/// Is the process `inst.pid` still the one that was started for the entry (alive, and
/// started when the entry says)?
fn same_process(inst: &Instance) -> bool {
    pid_alive(inst.pid) && inst.process_started.is_some() && process_start(inst.pid) == inst.process_started
}

/// Is the game of this entry running? An entry without a start time (or on a system where
/// it cannot be read) counts only while it is this launcher's own child.
fn is_that_game(inst: &Instance) -> bool {
    inst.ended.is_none() && (is_our_child(&inst.id) || same_process(inst))
}

fn last_lines(path: &Path, n: usize, max_bytes: u64) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(max_bytes);
    let _ = f.seek(SeekFrom::Start(from));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().skip(if from > 0 { 1 } else { 0 }).map(|l| l.to_string()).collect();
    let k = lines.len().saturating_sub(n);
    lines.drain(..k);
    lines
}

/// Every game started by a launcher, running ones first, newest first.
pub fn list() -> Vec<Instance> {
    reap();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir()) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "json").unwrap_or(true) {
            continue;
        }
        let Some(mut inst) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Instance>(&b).ok()) else {
            let _ = std::fs::remove_file(&p);
            continue;
        };
        // a process id is reused eventually: only the process that started then is the game
        let alive = is_that_game(&inst);
        if inst.running && !alive {
            inst.running = false;
            inst.ended = Some(now_secs());
            write(&inst);
        }
        inst.running = alive;
        if !alive && now_secs().saturating_sub(inst.ended.unwrap_or(0)) > KEEP_ENDED {
            let _ = std::fs::remove_file(&p);
            continue;
        }
        if inst.id.is_empty() {
            inst.id = inst.pid.to_string();
        }
        let lan = lan_dir().join(format!("{}.json", inst.id));
        if alive {
            inst.lan_status = std::fs::read(&lan).ok().and_then(|b| serde_json::from_slice(&b).ok());
        } else {
            let _ = std::fs::remove_file(&lan);
        }
        inst.last_line = last_lines(Path::new(&inst.log), 1, 4096).pop().unwrap_or_default();
        out.push(inst);
    }
    // LAN status files of games that are gone (the file names the game's process)
    if let Ok(rd) = std::fs::read_dir(lan_dir()) {
        for e in rd.flatten() {
            let pid = std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.get("pid").and_then(|p| p.as_u64()));
            if pid.map(|p| !pid_alive(p as u32)).unwrap_or(false) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    out.sort_by(|a, b| b.running.cmp(&a.running).then(b.started.cmp(&a.started)));
    out
}

/// LAN sessions hosted by games on this machine (for "join a session here").
pub fn local_hosts() -> Vec<Value> {
    list().into_iter().filter(|i| i.running).filter_map(|i| i.lan_status).filter(|s| s.get("role").and_then(|r| r.as_str()) == Some("host")).collect()
}

/// The log file for a new game: `game.log` when no running game writes it, else the first
/// free `game-n.log`.
fn free_slot(running: &[Instance]) -> (u32, PathBuf) {
    let data = crate::data_dir();
    for n in 1u32.. {
        if running.iter().any(|i| i.slot == n) {
            continue;
        }
        let name = if n == 1 { "game.log".to_string() } else { format!("game-{n}.log") };
        return (n, data.join(name));
    }
    unreachable!()
}

pub struct Started {
    pub pid: u32,
    pub log: PathBuf,
    pub command: String,
    pub others: usize,
}

/// Where the launcher's window is (the middle of it, physical pixels on the desktop): the
/// game opens on that screen rather than the main one (#1959).
static SCREEN_AT: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// Tell the games started from now on which screen the launcher stands on.
pub fn set_screen_at(at: Option<(i32, i32)>) {
    *SCREEN_AT.lock().unwrap_or_else(|e| e.into_inner()) = at;
}

/// Start the game with `args`; games already running keep running.
pub fn start(game: &Path, args: &[String], d: &crate::Duty, profile: &str) -> Result<Started> {
    let running: Vec<Instance> = list().into_iter().filter(|i| i.running).collect();
    let (slot, log) = free_slot(&running);
    let file = std::fs::File::create(&log).with_context(|| format!("creating {}", log.display()))?;
    let err = file.try_clone()?;
    let id = format!("{}-{}-{}", now_secs(), std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let mut cmd = std::process::Command::new(game);
    cmd.args(args).env("OMSI_INSTANCE", &id).stdout(file).stderr(err);
    if let Some((x, y)) = *SCREEN_AT.lock().unwrap_or_else(|e| e.into_inner()) {
        cmd.env("OMSI_SCREEN_AT", format!("{x},{y}"));
    }
    let child = cmd.spawn().with_context(|| format!("starting {}", game.display()))?;
    let pid = child.id();
    let process_started = process_start(pid);
    CHILDREN.lock().unwrap_or_else(|e| e.into_inner()).push((id.clone(), child));
    let inst = Instance {
        id,
        pid,
        process_started,
        slot,
        log: log.to_string_lossy().to_string(),
        started: now_secs(),
        map: d.map.clone(),
        bus: d.bus.clone(),
        entry: d.entry,
        line: d.line.clone(),
        tour: d.tour.clone(),
        profile: profile.to_string(),
        lan: d.lan.clone().unwrap_or_else(|| "off".into()),
        args: args.to_vec(),
        running: true,
        ..Default::default()
    };
    write(&inst);
    let command = format!("{} {}", game.display(), args.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" "));
    Ok(Started { pid, log, command, others: running.len() })
}

/// How long Stop waits for a game to end by itself (it writes the session summary and the
/// personnel file and says goodbye to its LAN players) before it is killed.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(8);

/// End a game the launcher started: ask it to quit, wait for it, kill it only when it
/// does not end in time. Returns true when it ended by itself.
pub fn stop(pid: u32) -> Result<bool> {
    stop_within(pid, STOP_GRACE)
}

fn stop_within(pid: u32, grace: std::time::Duration) -> Result<bool> {
    reap();
    let Some(mut inst) = read_all().into_iter().find(|i| i.pid == pid && is_that_game(i)) else {
        return Err(anyhow!("no running game with process id {pid} was started by the launcher"));
    };
    // every launcher window shows it as stopping meanwhile
    inst.stopping = Some(now_secs());
    write(&inst);
    let by_itself = end_process(&inst, grace)?;
    reap();
    if let Some(mut inst) = read_all().into_iter().find(|i| i.id == inst.id) {
        inst.running = false;
        inst.ended.get_or_insert(now_secs());
        inst.killed = !by_itself;
        let _ = std::fs::remove_file(lan_dir().join(format!("{}.json", inst.id)));
        write(&inst);
    }
    Ok(by_itself)
}

/// End the entry's game: ask it to quit (SIGTERM; the game then ends its session as Escape
/// does) and wait up to `grace` for it, then kill it. Our own child is watched by its
/// handle; another launcher's game is signalled only after checking, right before each
/// signal, that the process is still that game. Returns true when it ended by itself.
fn end_process(inst: &Instance, grace: std::time::Duration) -> Result<bool> {
    let ours = is_our_child(&inst.id);
    if !ours && !same_process(inst) {
        return Err(anyhow!("process {} is no longer the game that was started (it has ended; the id now belongs to another program) - nothing was stopped", inst.pid));
    }
    let ended = || if ours { reap(); !is_our_child(&inst.id) } else { !same_process(inst) };
    let asked = if ours {
        // while the list is held nobody collects the child, so its id is still its own
        let mut kids = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
        let running = kids.iter_mut().find(|(id, _)| *id == inst.id).map(|(_, c)| matches!(c.try_wait(), Ok(None))).unwrap_or(false);
        if !running {
            // it has ended already (collected by the next look at the list)
            return Ok(true);
        }
        request_quit(inst.pid)
    } else {
        request_quit(inst.pid)
    };
    if let Err(e) = asked {
        // (it may have ended just now)
        if !ended() {
            return Err(e);
        }
        return Ok(true);
    }
    let t0 = std::time::Instant::now();
    while t0.elapsed() < grace {
        if ended() {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if ended() {
        return Ok(true);
    }
    if ours {
        let mut kids = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, c)) = kids.iter_mut().find(|(id, _)| *id == inst.id) {
            let _ = c.kill();
            let _ = c.wait();
        }
    } else if same_process(inst) {
        force_kill(inst.pid)?;
    }
    Ok(false)
}

#[cfg(unix)]
fn request_quit(pid: u32) -> Result<()> {
    signal(pid, libc::SIGTERM)
}

#[cfg(unix)]
fn force_kill(pid: u32) -> Result<()> {
    signal(pid, libc::SIGKILL)
}

#[cfg(unix)]
fn signal(pid: u32, sig: libc::c_int) -> Result<()> {
    if unsafe { libc::kill(pid as libc::pid_t, sig) } != 0 {
        return Err(anyhow!("could not stop process {pid}: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Windows: taskkill without /F closes the game's window (the game ends its session as
/// when the window is closed); with /F it ends the process.
#[cfg(not(unix))]
fn request_quit(pid: u32) -> Result<()> {
    taskkill(pid, false)
}

#[cfg(not(unix))]
fn force_kill(pid: u32) -> Result<()> {
    taskkill(pid, true)
}

#[cfg(not(unix))]
fn taskkill(pid: u32, force: bool) -> Result<()> {
    let pid = pid.to_string();
    let mut args = vec!["/PID", pid.as_str(), "/T"];
    if force {
        args.push("/F");
    }
    let status = std::process::Command::new("taskkill").args(&args).status()?;
    if !status.success() {
        return Err(anyhow!("could not stop process {pid}"));
    }
    Ok(())
}

/// The end of a game's log.
pub fn log_tail(pid: u32, lines: usize) -> Result<Vec<String>> {
    let inst = list().into_iter().find(|i| i.pid == pid).ok_or_else(|| anyhow!("no game with process id {pid}"))?;
    Ok(last_lines(Path::new(&inst.log), lines.clamp(1, 2000), 256 * 1024))
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    use super::*;

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_reused_process_id_is_not_the_game() {
        let me = std::process::id();
        let started = process_start(me).expect("our own start time");
        assert_eq!(process_start(me), Some(started), "the start time does not change");
        let mut inst = Instance { id: "self".into(), pid: me, process_started: Some(started), ..Default::default() };
        assert!(is_that_game(&inst));
        // the same process id, but a process that started at another time
        inst.process_started = Some(started + 1);
        assert!(!is_that_game(&inst));
        // an entry that does not say when its process started is not taken for a live game
        inst.process_started = None;
        assert!(!is_that_game(&inst));
        inst.process_started = Some(started);
        inst.ended = Some(now_secs());
        assert!(!is_that_game(&inst));
        assert!(process_start(u32::MAX / 2).is_none(), "no such process");
    }

    #[cfg(unix)]
    #[test]
    fn stop_never_signals_another_program() {
        // a program that is not a child the launcher knows, with the id of an old entry
        let mut other = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = other.id();
        let real = process_start(pid);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        // Linux start times have clock-tick resolution: different PIDs can share a
        // start time. The stale entry below must mismatch this PID's own start time.
        assert!(real.is_some());
        let old = Instance { id: "old".into(), pid, process_started: real.map(|t| t.wrapping_sub(5_000_000)), ..Default::default() };
        assert!(!is_that_game(&old));
        assert!(end_process(&old, std::time::Duration::from_millis(300)).is_err());
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(other.try_wait().unwrap().is_none(), "the other program still runs");
        // the entry of that very process is stopped
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let game = Instance { id: "game".into(), pid, process_started: real, ..Default::default() };
            assert!(is_that_game(&game));
            // not our child: it stays a zombie until this test collects it, and a zombie has
            // ended - the stop sees that at once instead of waiting out its grace time
            let t0 = std::time::Instant::now();
            assert!(end_process(&game, std::time::Duration::from_secs(5)).unwrap(), "ended by itself (SIGTERM)");
            assert!(t0.elapsed() < std::time::Duration::from_secs(2), "{:?}", t0.elapsed());
            assert!(process_start(pid).is_none(), "a zombie has no start time");
            let status = other.wait().unwrap();
            assert!(!status.success(), "terminated: {status:?}");
            assert!(!is_that_game(&game), "an ended process is not the game");
        }
        let _ = other.kill();
        let _ = other.wait();
    }

    /// A child of this launcher, registered as a game.
    #[cfg(unix)]
    fn child_game(id: &str, script: &str) -> Instance {
        let child = std::process::Command::new("sh").arg("-c").arg(script).spawn().unwrap();
        let pid = child.id();
        CHILDREN.lock().unwrap().push((id.to_string(), child));
        Instance { id: id.into(), pid, process_started: process_start(pid), ..Default::default() }
    }

    #[cfg(unix)]
    #[test]
    fn stop_lets_the_game_finish_and_kills_only_a_stuck_one() {
        let dir = std::env::temp_dir().join(format!("omsi-stop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // a game that writes its summary when asked to quit
        let summary = dir.join("session.json");
        let game = child_game("graceful-test", &format!("trap 'echo done > \"{}\"; exit 0' TERM; while :; do sleep 0.05; done", summary.display()));
        std::thread::sleep(std::time::Duration::from_millis(200));
        let t0 = std::time::Instant::now();
        assert!(end_process(&game, std::time::Duration::from_secs(5)).unwrap(), "ended by itself");
        assert!(t0.elapsed() < std::time::Duration::from_secs(3), "no waiting out the grace time: {:?}", t0.elapsed());
        assert_eq!(std::fs::read_to_string(&summary).unwrap().trim(), "done");
        assert!(!is_our_child("graceful-test"), "reaped");
        // a game that does not react is killed once the grace time is over
        let stuck = child_game("stuck-test", "trap '' TERM; exec sleep 30");
        std::thread::sleep(std::time::Duration::from_millis(200));
        let t0 = std::time::Instant::now();
        assert!(!end_process(&stuck, std::time::Duration::from_millis(600)).unwrap(), "killed");
        assert!(t0.elapsed() >= std::time::Duration::from_millis(600));
        reap();
        assert!(!is_our_child("stuck-test"), "reaped after the kill");
        assert!(!pid_alive(stuck.pid) || process_start(stuck.pid) != stuck.process_started, "the stuck game is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
