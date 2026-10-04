# Contributing

## Layout

Three crates:

- `crates/soos-core` -- the engine: fend-core wrapped with the document
  model (`prev`/`sum`/`total`/`avg`, converters), currency, natural-language
  rewriting, syntax highlighting, result formatting, and the files the app
  and CLI share. No UI.
- `crates/soos-app` -- the desktop app (egui/eframe), one module per part of
  the window: `tabs`, `editor`, `status_bar`, the `example` and
  `converters` overlays, plus `recalc` (the worker thread documents are
  recalculated on), `hotkey`, `tray`, `update` and `style`.
- `crates/soos-cli` -- the CLI (`soos-cli '20 inches in cm'`), which prints
  what the app shows for the same line, and the `--json` output the
  PowerToys plugin uses.

## Reading the code

Start at `crates/soos-core/src/lib.rs`: its `//!` comment follows one
recalculation from keystroke to painted result, and is the map of the code.
Each module's own `//!` comment says what it's for. To browse the docs with
working links:

```
cargo doc --workspace --no-deps --document-private-items --open
```

Decisions a user can see -- how `sum` treats errors and units, rounding,
currency symbols, time-zone abbreviations -- are listed in the README's
[How it calculates](README.md#how-it-calculates), not in code comments.

## Writing style

### Comments

- Comment only what the code can't say: a constraint from a dependency, a
  workaround, a number that isn't arbitrary. If the name and type already
  say it, write nothing.
- One summary line, then at most a few lines of why. Longer means it
  belongs in a design note, or the code needs restructuring.
- A comment never settles user-visible behaviour. If you'd call something a
  user sees "intentional", "harmless" or "not supported", put it in the
  README's calculation rules (with a test) or open an issue.
- No plans or "next step" notes -- open an issue.
- Describe the code as it is now. History, and how you found something out,
  go in the commit message.
- Say a thing once, where it applies, and point to it from elsewhere.
- Link other items with rustdoc links (`` [`X`] ``), so CI catches renames.
  Don't cite README section names from code.

### README and other docs

- No marketing adjectives, and no numbers the repository can't back up.
- Every example that shows a result is checked by a test (the README's by
  `readme_examples_match_the_engine`, the app's `?` example by
  `example_lines_match_the_engine`) or says it depends on the date.
- No placeholders.

### UI text

- Sentence case for every label, tooltip and message: "High precision", not
  "High Precision".
- A message says what's wrong and what to do: "Hold Ctrl, Alt or Win with
  that key", not "can't use that".
- A status-bar icon has a tooltip that says what it does and, for a toggle,
  whether it's on.
- Plain typography in anything a user reads: no `--`.

### General

- Short, direct sentences.
- Code identifiers, flags and file paths in backticks.

Whoever commits a change owns every sentence in it, whether they or a tool
wrote it: read the comments and docs in your diff before you commit.

## Building

The MSRV is 1.95, set by egui/eframe and declared once as the workspace
`rust-version`. Every crate inherits it, so Cargo refuses an older
toolchain even for `soos-core` alone. CI checks on exactly that version and
tests on current stable. Raise it only when a dependency requires it.

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

## Making a change

Fork the repo, clone your fork, and branch off `main`. Keep commits focused
-- one logical change per commit -- and open a PR against `main` saying what
changed and why. Small PRs are easier to review than large ones.

## Before opening a PR

CI (`.github/workflows/ci.yml`) runs these on Windows, macOS and Linux:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

and on Linux, the docs with warnings as errors, the Markdown and
placeholder check, and a check on the MSRV:

```
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items
python3 scripts/check-docs.py
cargo +1.95 check --workspace --all-targets --locked
```

Run them locally first.

## The hotkey patch

[marelevn/global-hotkey-patched](https://github.com/marelevn/global-hotkey-patched)
is `global-hotkey` 0.8.0 with `Code::LaunchApp2`, the Calculator key, added
to its Windows and X11 key tables, which upstream lacks. The root
`Cargo.toml` pulls it in through `[patch.crates-io]`, pinned to a commit.
That repository's `PATCH.md` shows the change and how to re-apply it when
`global-hotkey` is upgraded.

## Releasing

Pushing a `v*` tag builds a Windows `.zip`, a macOS `.dmg` (one binary for
Apple silicon and Intel) and a Linux `.tar.gz`, and attaches them to a
**draft** GitHub Release (`.github/workflows/release.yml`). They're
unsigned until there are certificates to sign them with. Publish the draft by hand after checking the files.

There's no installer: `soos_core::storage::data_dir` keeps everything next
to the executable, except on macOS, where replacing `Soos.app` to update
would take the data with it, so it uses Application Support.

## Fonts

JetBrains Mono lacks most currency signs Soos can print (`₹`, `₩`, `₺`,
`฿` and more). `assets/fonts/DejaVuSansMono-Currency.ttf` has just those,
cut from DejaVu Sans Mono 2.37 with fontTools, so the fallback is a few KB
instead of a whole font:

```sh
pyftsubset DejaVuSansMono.ttf --text="₹₩₺₱₪₦₸₡₲₵₭฿₨" \
  --output-file=assets/fonts/DejaVuSansMono-Currency.ttf
```

DejaVu has no `₼`, `₾` or `﷼`, so those show as boxes. A sign added to
soos-core's currencies that JetBrains Mono lacks belongs in that list, and
in the test in `style.rs`.

## Icons

`assets/icons/` (`soos.ico`, `soos.icns`, `hicolor/{32x32,256x256}.png`) is
made from `assets/logo.svg` with an SVG rasterizer such as `resvg`. The logo
draws its letters as text in JetBrains Mono ExtraBold, so that font must be
installed when you rasterize it.

`soos.icns` and `soos.ico` come from a 1024 px PNG of the logo, through
`scripts/macos-icon.py` (needs Pillow). The Mac icon isn't the
logo as drawn: Apple's grid puts an 824 px body with a 185 px corner on a
1024 px canvas, and the edge-to-edge logo looks oversized next to other
Dock icons. For the same reason the app passes no window icon on macOS --
eframe would otherwise set the full-bleed one over the bundle's `.icns`.
The `.ico` keeps the full-bleed logo, at 16, 24, 32, 48, 64 and 256 px.
