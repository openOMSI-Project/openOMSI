# Versioning and releases

## The version number: `MAJOR.MINOR.COMMIT`

| Part | Where it comes from | Example |
| --- | --- | --- |
| `MAJOR` | the first number in the [`VERSION`](../VERSION) file, changed by hand | `1` |
| `MINOR` | the second number in `VERSION`, changed by hand | `1.2` |
| `COMMIT` | counted automatically: commits on `main` since `VERSION` last changed | `1.2.17` |

`COMMIT` restarts from `0` whenever `VERSION` changes: after `1.1.23`, a commit that sets
`VERSION` to `1.2` is released as `1.2.0`, and the next one as `1.2.1`.

To start a new version, edit one line and push:

```sh
echo 1.2 > VERSION
git commit -am "Version 1.2" && git push
```

`scripts/version.sh` prints the version of the checked-out commit; the game shows it in the
launcher's side bar, in its log and in `openomsi --version` (baked in by
`crates/omsi-app/build.rs`, which uses the same rule).

## Releases

Every push to `main` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml):

1. works out the version with `scripts/version.sh`;
2. builds Windows x64 and ARM64 (MSVC), macOS for Apple silicon and Intel, Linux x64 and
   ARM64 and Android in parallel, and packs the dedicated server from the Linux and Windows
   builds;
3. creates the release `v<version>` (tag on that commit) with the archives
   `openOMSI-<version>-windows-x64.zip`, `-windows-arm64.zip`, `-macos-arm64.zip`,
   `-macos-x64.zip`, `-linux-x64.zip`, `-linux-arm64.zip`, `-android-arm64.apk`,
   `-server-linux-x64.zip`, `-server-linux-arm64.zip`, `-server-windows-x64.zip`,
   `-server-windows-arm64.zip`, and release notes generated from the commits.

Pull requests run the same builds without publishing anything. Build output never goes
into the repository (`target/` and `dist/` are ignored).

The version badge at the top of the README always shows the newest release.

## The website

[`.github/workflows/pages.yml`](../.github/workflows/pages.yml) publishes `site/` together with
the Markdown files of `docs/` to GitHub Pages
(https://openOMSI-Project.github.io/openOMSI/) whenever they change on `main`. The site renders
the Markdown in the browser, so a documentation change is one edit in `docs/`.
