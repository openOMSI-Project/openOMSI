# Contributing to openOMSI

Thanks for helping! A few rules keep the project healthy:

* **No original code or assets.** Never copy anything from an OMSI 2 installation into the
  repository - no textures, models, sounds, maps, scripts. Tests that need
  content read it from a local installation (`OMSI_ROOT`) and skip themselves without one.
* **Compatibility first.** A change must not break a stock map or a mod that worked. Run
  `cargo run --release -p omsi-check -- "/path/to/OMSI 2"` before and after larger changes.
* **One change per pull request**, with a message that says what the player notices.
* `cargo test --workspace` and `cargo build --release` must pass (CI checks all platforms).
* Code style: `rustfmt` defaults, comments explain *why*.

## Issues

* **English only** - titles and text, so every contributor can read and search them. An
  issue in another language is closed automatically with a request to translate it, and
  opens again by itself once it is edited into English. Logs and game text can stay as
  they are.
* One problem or idea per issue, on the latest release. Questions go to the
  [Discord server](https://discord.gg/VG2EKVafYG).

## Where things are

See the layout in the [README](README.md#repository-layout) and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). File formats: [docs/FORMATS.md](docs/FORMATS.md).

## Releases

Maintainers bump `MAJOR.MINOR` in the `VERSION` file; everything else is automatic - see
[docs/VERSIONING.md](docs/VERSIONING.md).
