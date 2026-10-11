# Contributing

## Building

1. Install Rust 1.95 or newer, the minimum supported Rust version (MSRV).
2. On Linux, install the GUI toolkit's dev headers (Debian/Ubuntu):

   ```
   sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
   ```

3. Build, run and test. On macOS, a dev build uses your real tabs,
   settings and rates (see [Where the data lives](#where-the-data-lives)),
   so `cargo build`, then run the binary with its own `HOME`:

   ```
   cargo build
   HOME=$(mktemp -d) target/debug/soos-app
   HOME=$(mktemp -d) target/debug/soos-cli "20 inches in cm"
   cargo test --workspace --locked
   ```

   `HOME=... cargo run` doesn't work: rustup looks for its toolchains under
   the real `HOME`. On Windows and Linux, `cargo run -p soos-app` and
   `cargo run -p soos-cli -- "20 inches in cm"` are fine.

The MSRV is set by egui/eframe and declared once, as the workspace
`rust-version`. Every crate inherits it, so Cargo refuses an older toolchain
even for `soos-core` alone. CI checks on exactly that version and tests on
current stable. Raise it only when a dependency requires it.

### Where the data lives

There's no installer. `soos_core::storage::data_dir` keeps everything in a
`data` folder next to the executable, except on macOS, where replacing
`Soos.app` to update would take the data with it, so it uses
`~/Library/Application Support/Soos/`.

On Windows and Linux a dev build uses the `data` folder beside it, inside
`target/`; on macOS it uses your real one (see [Building](#building)).

## Making a change

1. Fork the repo, clone your fork, and branch off `dev`.
2. Keep commits focused: one logical change per commit.
3. Run what CI runs (below).
4. Open a PR against `dev` saying what changed and why (see
   [Branches](#branches)).

### What CI runs

The checks in `.github/workflows/ci.yml`. On Windows, macOS and Linux:

```
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
```

CI runs these on Linux only, but they work on any system:

```
# the docs build, with warnings as errors
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --no-deps --document-private-items
# Markdown links and anchors, placeholder text, and the card images
python3 scripts/check-docs.py
# the build on the MSRV (rustup toolchain install 1.95 first)
cargo +1.95 check --workspace --all-targets --locked
```

`scripts/check-docs.py` reads the files Git tracks, so `git add` a new page
before you run it.

## Writing tests

- A test that needs exchange rates uses `RateSource::with_rates`, not a
  rates file; its doc comment says why.
- The guide is tested too: see [README and other docs](#readme-and-other-docs).

## Layout

Three crates:

- `crates/soos-core` -- the calculation engine: fend-core wrapped with the
  document model (`prev`/`sum`/`total`/`avg`, converters), currency,
  natural-language rewriting, syntax highlighting, result formatting, and the
  data files (rates, converters) the app and CLI share. No UI.
- `crates/soos-app` -- the desktop app (egui/eframe), one module per part of
  the window: `tabs`, `editor`, `status_bar`, the `example` and
  `converters` overlays, plus `recalc` (the worker thread that recalculates
  documents), `hotkey`, `tray`, `update`, `style` and `window`.
- `crates/soos-cli` -- the CLI (`soos-cli '20 inches in cm'`), which prints
  what the app shows for the same line, and the `--json` output the
  PowerToys plugin uses.

And around them:

- `integrations/powertoys` -- a C# PowerToys Run plugin that calls
  `soos-cli --json`; see the [plugin guide](docs/powertoys.md).
- `docs/` -- the user guide: Markdown pages, the example cards in
  `docs/cards`, and the images drawn from them in `docs/img`.
- `scripts/` -- `make-cards.py` draws the card images, `check-docs.py` is
  CI's docs check, and `macos-icon.py` makes `soos.icns` and `soos.ico`
  (see [Icons](#icons)). The first two use only the Python standard library;
  `macos-icon.py` needs Pillow and is for maintainers.
- `assets/` -- the logo, icons, fonts, the README screenshot and the macOS
  `.dmg` background.

## Reading the code

Start at `crates/soos-core/src/lib.rs`: its `//!` comment follows one
recalculation from keystroke to painted result, and is the map of the code.
Each module's own `//!` comment says what it's for. To browse the docs with
working links:

```
cargo doc --workspace --no-deps --document-private-items --open
```

Decisions a user can see -- how `sum` treats errors and units, rounding,
currency symbols, time-zone abbreviations -- are shown in the
[guide](docs/README.md), not in code comments.

## Writing style

### Comments

- Comment only what the code can't say: a constraint from a dependency, a
  workaround, a number that isn't arbitrary. If the name and type already
  say it, write nothing.
- One summary line, then at most a few lines of why. Longer, and the code
  probably needs restructuring.
- Don't decide or document user-visible behavior in a comment. If you'd call
  something a user sees "intentional", "harmless" or "not supported", put it
  in the [guide](docs/README.md) (with a test) or open an issue.
- No plans or "next step" notes -- open an issue.
- Describe the code as it is now. History, and how you found something out,
  go in the commit message.
- Say a thing once, where it applies, and point to it from elsewhere.
- Link other items with rustdoc links (`` [`X`] ``), so CI catches renames.
  Don't cite README section names from code.

### README and other docs

- No marketing adjectives, and no numbers the repository can't back up.
- Check every result you show. Three tests do it:
  - `readme_examples_match_the_engine`, in `crates/soos-cli`: the README's
    Quick guide, and each `$ soos-cli '...'` line followed by its answer, in
    the README and the guide.
  - `doc_cards_match_the_engine`, in `crates/soos-cli`: the guide's cards.
  - `example_lines_match_the_engine`, in `crates/soos-app`: the app's `?`
    example.

  Run the first two with `cargo test -p soos-cli`. A `$ soos-cli` answer
  that starts with a weekday (`Friday, ...`) is skipped, since it depends on
  today's date.
- A result in prose, outside a card or a `$ soos-cli '...'` line, isn't
  tested: run it yourself before you write it down.
- An answer that depends on the date says so.
- Some examples in `docs/soos-cli.md` are not covered: the
  `printf ... | soos-cli -` ones and the `--json` ones. Re-run them by hand
  when you change what they show.
- The guide's example cards are `docs/cards/*.txt`: a document with its
  results beside it. Each line is split into expression and result at its
  last run of two or more spaces. After editing one, run
  `python3 scripts/make-cards.py` to redraw `docs/img`; CI fails if they
  differ.
- Keep a page short: the example first, then a line per rule. Put a long
  reference behind a link, not in the README.
- No placeholder text. `PLACEHOLDER` in `scripts/check-docs.py` lists the
  markers CI rejects, in Markdown and in Rust.

### UI text

- Sentence case for every label, tooltip and message: "High precision", not
  "High Precision".
- A message says what's wrong and what to do: "Hold Ctrl, Alt or Win with
  that key", not "can't use that".
- A status-bar icon has a tooltip that says what it does and, for a toggle,
  whether it's on.
- No `--` in the app's own text. Use a full stop or a colon.

### General

- Short, direct sentences.
- Code identifiers, flags and file paths in backticks.
- In Markdown and comments, `--` is the dash; no em dashes.

Whoever commits a change owns every sentence in it, whether they or a tool
wrote it: read the comments and docs in your diff before you commit.

## Maintainers

### Branches

- Work happens on the `dev` branch, and PRs go there. It is merged into
  `main` after testing.
- Each release is squashed into one commit on top of the last release, so
  the history has one commit per version. That commit credits everyone
  whose PR it includes with a `Co-authored-by:` line, so your name stays in
  the history.

### Releasing

1. Bump the version: `version` in the workspace `Cargo.toml` and
   `"Version"` in `integrations/powertoys/plugin.json`. Run
   `cargo check --workspace` to refresh `Cargo.lock`.
2. Squash the release into one commit, with a `Co-authored-by:` line for
   each contributor, and get it onto `main` with a green CI run. Windows
   and macOS are only tested by CI.
3. Tag that commit `v*` and push the tag. `.github/workflows/release.yml`
   runs the tests on Linux first; if one fails, nothing is built and no
   draft is made.
4. The workflow builds a Windows `.zip`, a macOS `.dmg` (one binary for
   Apple silicon and Intel) and a Linux `.tar.gz`, and attaches them to a
   **draft** GitHub Release.
5. Check the files, then publish the draft by hand.

The builds are unsigned until there are certificates to sign them with.

### The hotkey patch

[marelevn/global-hotkey-patched](https://github.com/marelevn/global-hotkey-patched)
is `global-hotkey` 0.8.0 with `Code::LaunchApp2`, the Calculator key, added
to its Windows and X11 key tables, which upstream lacks. The root
`Cargo.toml` pulls it in through `[patch.crates-io]`, pinned to a commit.
That repository's `PATCH.md` shows the change and how to re-apply it when
`global-hotkey` is upgraded.

### Fonts

JetBrains Mono lacks most currency signs Soos can print (`₹`, `₩`, `₺`,
`฿` and more). `assets/fonts/DejaVuSansMono-Currency.ttf` has just those,
cut from DejaVu Sans Mono 2.37 with fontTools, so the fallback is a few KB
instead of a whole font:

```sh
pyftsubset DejaVuSansMono.ttf --text="₹₩₺₱₪₦₸₡₲₵₭฿₨" \
  --output-file=assets/fonts/DejaVuSansMono-Currency.ttf
```

DejaVu has no `₼`, `₾` or `﷼`, so those show as boxes. A sign added to
soos-core's currencies that JetBrains Mono lacks belongs in the `--text`
string above and in the test in `crates/soos-app/src/style.rs`.

### Icons

`assets/icons/` (`soos.ico`, `soos.icns`, `hicolor/{32x32,256x256}.png`) is
made from `assets/logo.svg` with an SVG rasterizer such as `resvg`. The logo
draws its letters as text in JetBrains Mono ExtraBold, so that font must be
installed when you rasterize it.

`soos.icns` and `soos.ico` come from a 1024 px PNG of the logo, through
`scripts/macos-icon.py` (needs Pillow). The Mac icon isn't the logo as
drawn: Apple's grid puts an 824 px body with a 185 px corner radius on a
1024 px canvas, and the edge-to-edge logo looks oversized next to other
Dock icons. For the same reason the app passes no window icon on macOS --
eframe would otherwise set the full-bleed one over the bundle's `.icns`.
The `.ico` keeps the full-bleed logo, at 16, 24, 32, 48, 64 and 256 px.
