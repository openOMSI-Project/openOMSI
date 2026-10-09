#!/bin/sh
# Build openOMSI for Windows x64 on macOS or Linux (cross-compiled with MinGW) into
# dist/windows. On Windows itself use scripts\build-windows.cmd.
# Needs: brew install mingw-w64 (or apt install mingw-w64); rustup target add x86_64-pc-windows-gnu
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
command -v cargo >/dev/null 2>&1 || { echo "Install Rust from https://rustup.rs, then run this script again." >&2; exit 1; }
command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || { echo "Install MinGW: brew install mingw-w64" >&2; exit 1; }
rustup target list --installed | grep -q '^x86_64-pc-windows-gnu$' || rustup target add x86_64-pc-windows-gnu
export OPENOMSI_VERSION="${OPENOMSI_VERSION:-$(sh scripts/version.sh 2>/dev/null || echo 0.0.0)}"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++
export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
# (MinGW keeps debug info inside the .exe, which made it ~200 MB; this build goes without.)
export CARGO_PROFILE_RELEASE_DEBUG=0
cargo build --locked --release --target x86_64-pc-windows-gnu -p omsi-app -p omsi-launcher-core
mkdir -p dist/windows   # (the folder is also the content folder: mods stay)
# (the release package also carries Google's ANGLE - libEGL.dll, libGLESv2.dll and
# LICENSE-ANGLE.txt from openOMSI-org/angle-openomsi, see .github/workflows/release.yml -
# for the "ANGLE (DirectX 11)" graphics API; startup::backend_order skips it without them)
cp target/x86_64-pc-windows-gnu/release/openomsi.exe target/x86_64-pc-windows-gnu/release/openomsi-launcher.exe assets/steam_redist/steam_api64.dll dist/windows/
printf '\nopenOMSI %s built in dist/windows\n' "$OPENOMSI_VERSION"
