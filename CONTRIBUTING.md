# Contributing

## Layout

Three crates:

- `crates/soos-core` -- the engine: fend-core wrapped with the document
  model (`prev`/`sum`/`total`/`avg`, converters), currency, natural-language
  rewriting, and syntax highlighting. No UI.
- `crates/soos-app` -- the desktop app (egui/eframe): editor, gutter,
  status bar, tray icon, global hotkey.
- `crates/soos-cli` -- one-shot CLI (`soos-cli "20 inches in cm"`), and the
  `--json`/`--alfred` output modes the launcher integrations use.

## Building

MSRV is 1.82 (set in the workspace manifest's `rust-version`; bumping it
past whatever `stable` currently is needs a reason, not just convenience).

```
cargo build --release
cargo run -p soos-app
cargo run -p soos-cli -- "20 inches in cm"
cargo test --workspace
```

On Linux, install the GUI toolkit's dev headers first (Debian/Ubuntu):

```
sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
```

## Before opening a PR

CI (`.github/workflows/ci.yml`) runs these on Windows, macOS and Linux, and
fails the build on any of them:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run all three locally first.

## The hotkey patch

[marelevn/global-hotkey-patched](https://github.com/marelevn/global-hotkey-patched)
is `global-hotkey` 0.8.0's own source with two one-line additions
(`Code::LaunchApp2`, the OS Calculator key, added to the Windows and X11 key
tables) that upstream doesn't have, pulled in via `[patch.crates-io]` in the
root `Cargo.toml`, pinned to a tag rather than a floating branch. See that
repo's `PATCH.md` for exactly what changed and how to re-apply it if
`global-hotkey` is ever upgraded.

## Releasing

Binaries are built and attached to a **draft** GitHub Release
(`.github/workflows/release.yml`) whenever a `v*` tag is pushed -- a
Windows `.zip`, a macOS `.dmg`, and a Linux `.tar.gz`, all unsigned (the
workflow has commented-out signing steps for when certificates exist).
Publishing the draft is a manual step after checking the artifacts.

There's no installer on any platform. `soos_core::currency::data_dir`
saves everything (the document, settings, rate cache) next to the running
exe -- unzip and run is the whole install, and the whole app is that one
folder. macOS is the one exception: it saves to Application Support
instead, so replacing `Soos.app` (how a `.app` is normally updated)
doesn't take the document down with the old bundle.

## Icons

`assets/icons/` (`soos.ico`, `soos.icns`, `hicolor/{32x32,256x256}.png`) is
generated from `assets/logo.svg`, not hand-edited, but there's no automated
regen tool anymore -- re-derive them with an SVG rasterizer (`resvg`, or
whatever's on hand) if `logo.svg` ever changes; this happens rarely enough
that a one-off script beats maintaining a dedicated tool year-round.
