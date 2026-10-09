//! Ending the game from outside: the launcher's Stop button sends SIGTERM, a terminal
//! Ctrl+C sends SIGINT, a closed terminal SIGHUP. Left to their default these end the
//! process on the spot, so no session summary and no personnel file were written and the
//! LAN peers only noticed the player was gone when he timed out. Here the signal only
//! marks the request; a watcher thread wakes the event loop, which then ends the session
//! the way Escape and closing the window do (summary, personnel file, LAN goodbye). A
//! second signal ends the process at once, for a game that does not react.

use std::sync::atomic::{AtomicI32, Ordering};

/// The signal that asked the game to end (0 = none yet).
static REQUESTED: AtomicI32 = AtomicI32::new(0);

/// Why the game was asked to end, for the log.
pub fn signal_name(sig: i32) -> &'static str {
    match sig {
        SIGTERM => "SIGTERM",
        SIGINT => "SIGINT",
        SIGHUP => "SIGHUP",
        RESTART => "restart",
        _ => "a signal",
    }
}

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
/// Exit code for a dedicated server that should be started again (`start.sh` / `start.cmd`).
pub const RESTART: i32 = 75;

/// Ask the game or dedicated server to end (the next step sees [`requested`]). Used by the
/// local admin page (`stop` / `restart`) where no OS signal arrives.
pub fn request(sig: i32) {
    let _ = REQUESTED.compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst);
}

/// Stop the dedicated server (same as SIGTERM).
pub fn request_stop() {
    request(SIGTERM);
}

/// Stop and ask `start.sh` / `start.cmd` to launch again (exit code [`RESTART`]).
pub fn request_restart() {
    request(RESTART);
}

#[cfg(unix)]
mod sys {
    pub type Handler = usize;
    pub const SIG_DFL: Handler = 0;
    pub const SIG_IGN: Handler = 1;
    pub const SIG_ERR: Handler = usize::MAX;
    extern "C" {
        pub fn signal(sig: i32, handler: Handler) -> Handler;
        pub fn raise(sig: i32) -> i32;
    }
}

/// The handler: only atomics and async-signal-safe calls (`signal`, `raise`) in here.
#[cfg(unix)]
extern "C" fn on_signal(sig: i32) {
    if REQUESTED.swap(sig, Ordering::SeqCst) != 0 {
        // asked a second time: the default action (the process ends now)
        // SAFETY: both calls are async-signal-safe
        unsafe {
            sys::signal(sig, sys::SIG_DFL);
            sys::raise(sig);
        }
    }
}

/// Take SIGTERM, SIGINT and SIGHUP over for the window's event loop: `wake` is called once,
/// from a watcher thread, when one of them arrived (a signal the parent set to be ignored,
/// as `nohup` does, stays ignored).
pub fn install(wake: impl FnOnce(i32) + Send + 'static) {
    #[cfg(unix)]
    {
        for sig in [SIGTERM, SIGINT, SIGHUP] {
            // SAFETY: `on_signal` has the C signature and does only async-signal-safe work
            unsafe {
                let old = sys::signal(sig, on_signal as extern "C" fn(i32) as sys::Handler);
                if old == sys::SIG_ERR {
                    log::warn!("cannot handle {}: the game ends at once when it arrives", signal_name(sig));
                } else if old == sys::SIG_IGN {
                    sys::signal(sig, sys::SIG_IGN);
                }
            }
        }
        let spawned = std::thread::Builder::new().name("quit signal".into()).spawn(move || loop {
            let sig = REQUESTED.load(Ordering::SeqCst);
            if sig != 0 {
                wake(sig);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        });
        if let Err(e) = spawned {
            log::warn!("no quit signal watcher ({e}): SIGTERM ends the game without saving");
            // SAFETY: back to the default actions
            unsafe {
                for sig in [SIGTERM, SIGINT, SIGHUP] {
                    let old = sys::signal(sig, sys::SIG_DFL);
                    if old == sys::SIG_IGN {
                        sys::signal(sig, sys::SIG_IGN);
                    }
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Windows: the launcher asks with WM_CLOSE (taskkill without /F), which arrives as
        // a close request of the window
        let _ = wake;
    }
}

/// The signal that asked the game to end, if one did.
pub fn requested() -> Option<i32> {
    Some(REQUESTED.load(Ordering::SeqCst)).filter(|s| *s != 0)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_signal_wakes_the_loop_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        install(move |sig| tx.send(sig).unwrap());
        assert!(requested().is_none());
        // SAFETY: raising a handled signal in our own process
        unsafe { sys::raise(SIGTERM) };
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), SIGTERM);
        assert_eq!(requested(), Some(SIGTERM));
        // woken once: the watcher is done (and has dropped its sender)
        assert_eq!(rx.recv_timeout(std::time::Duration::from_millis(300)), Err(std::sync::mpsc::RecvTimeoutError::Disconnected));
        // (a second signal would end the test process: not raised here)
    }
}
