# Building from source

Releases for every commit are on the [Releases](https://github.com/openOMSI-Project/openOMSI/releases)
page; build from source only to work on openOMSI itself.

All scripts live in `scripts/`, run from any folder (paths with spaces are fine) and put the
result into `dist/<platform>/`. That folder is also the game's **content folder** (mods go
beside the binary), so the scripts replace only the binaries and never delete anything else
there.

## Requirements

* The latest [Rust stable](https://rustup.rs) (`rustup update stable`): the builds use it,
  and the dependencies raise the oldest Rust they accept as they are updated.
* **macOS**: Xcode Command Line Tools (`xcode-select --install`). Metal is used for drawing.
* **Windows**: Rust *x86_64 MSVC* and Visual Studio Build Tools with *Desktop development
  with C++* and the Windows SDK. CMake is needed to build the OpenXR dependency;
  it must be on `PATH`. Vulkan or DirectX 12 is used for drawing.
* **Linux** (Debian/Ubuntu names):
  `sudo apt install build-essential pkg-config libasound2-dev libudev-dev libgtk-3-dev libxkbcommon-dev libwayland-dev libssl-dev`.
  Vulkan drivers (Mesa, NVIDIA) are needed to play.
* **Android**: the `aarch64-linux-android` Rust target, JDK 17, and an Android SDK with
  platform 34 or newer, build-tools and the NDK. `scripts/build-android.sh` looks for them
  through `android/env.sh` (`ANDROID_HOME`, `ANDROID_NDK_HOME`).

## Build

| Platform | Command | Result |
| --- | --- | --- |
| macOS | `scripts/build-macos.sh` | `dist/macos/openOMSI.app` |
| Windows | `scripts\build-windows.cmd` | `dist\windows\openomsi.exe`, `openomsi-launcher.exe` |
| Windows, from a Mac | `scripts/build-windows-cross.sh` (needs `brew install mingw-w64`) | `dist/windows/` |
| Linux | `scripts/build-linux.sh` | `dist/linux/openomsi`, `openomsi-launcher`, `.desktop` file |
| Android | `scripts/build-android.sh` | `dist/android/openOMSI-<version>.apk` |
| Dedicated server | `scripts/build-server.sh [folder]` | `dist/server/` with `start.sh` |
| 32-bit plugin host | `scripts/build-plugin-host.sh` | `dist/omsi-plugin-host32.exe` (see [PLUGINS.md](PLUGINS.md)) |

Plain cargo works too: `cargo build --release -p omsi-app` builds `target/release/openomsi`.

## The programs

* `openomsi` (`crates/omsi-app`) - the game. Started with no arguments it opens the launcher
  window; with arguments it starts a session directly (see [USER_GUIDE.md](USER_GUIDE.md));
  with `--server server.cfg` it is the dedicated server.
* `openomsi-launcher` (`crates/omsi-launcher-core`) - the launcher's commands for a terminal
  (`openomsi-launcher --cli maps`, `--cli install '{"path":"mod.zip"}'` …).
* `omsi-check` (`tools/omsi-check`) - loads every content file of an installation and reports
  what failed: `cargo run --release -p omsi-check -- "/path/to/OMSI 2"`.

## Icons

The application icon is the logo's mark - the ring with the bus front, on the navy square:
`assets/icons/app/openomsi.svg` (Windows/Linux), `openomsi-macos.svg` (macOS, with the
standard margin) and `openomsi-small.svg` (the 16 to 32 pixel pictures, with a thicker ring
and a simpler bus); the wordmarks are in `assets/logos`. `openomsi.ico` is embedded into the Windows executables at build time
(`build.rs`, `winresource`), `openomsi.icns` goes into the macOS bundle, and
`openomsi-256.png` is the window icon on Windows and Linux. To regenerate them after changing
the SVGs (needs `cargo install resvg`):

```sh
scripts/make-icons.sh
```

## Tests

```sh
cargo test --workspace
```

Some tests read an OMSI 2 installation (`OMSI_ROOT=/path/to/OMSI 2`) and skip themselves
when there is none.
