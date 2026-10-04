<div align="center">

<img src="assets/icons/hicolor/256x256.png" width="120" alt="Soos logo">

# Soos

A notepad-style calculator for Windows, macOS and Linux: every line is an
expression, and its answer shows up beside it.

[![CI](https://github.com/marelevn/soos/actions/workflows/ci.yml/badge.svg)](https://github.com/marelevn/soos/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/marelevn/soos)](https://github.com/marelevn/soos/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/marelevn/soos/total)](https://github.com/marelevn/soos/releases)
[![Rust](https://img.shields.io/badge/rust-1.95%2B-orange.svg)](CONTRIBUTING.md#building)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

![Soos, mid-calculation](assets/screenshot.png)

</div>

## Quick guide

    Rent: 1800                     1,800
    Food: 4 * 150                    600
    Bus pass: 12 * 2.5                30
    sum                            2,430

    rent = 1800                    1,800
    rent * 12                     21,600
    20% off 40                        32

`Rent:` is a label; only what follows it is calculated. A blank line starts
a new block. `sum` (or `total`) adds up the results in the block above it,
`avg` (or `average`) averages them, and `prev` is the previous result.
Variables like `rent` carry down the page.

## Install

Download for your platform from the
[latest release](https://github.com/marelevn/soos/releases/latest):

- **Windows** -- unzip `soos-windows-x86_64.zip`, run `soos-app.exe`.
- **macOS** (Apple silicon or Intel) -- open the `.dmg`, drag Soos to
  Applications.
- **Linux** -- extract the `.tar.gz`, run `soos-app`.

The builds aren't code-signed yet, so Windows SmartScreen and macOS
Gatekeeper warn on first launch.

> **"Soos is damaged and can't be opened" on macOS?**
>
> It isn't damaged; that's how Gatekeeper refuses an unsigned app. Clear
> the quarantine flag in Terminal (adjust the path if you moved Soos), then
> open it again:
> ```
> xattr -cr /Applications/Soos.app
> ```

Soos goes online for two things, and sends no user data with either:
refreshing exchange rates, and checking for a new version when you click
the version number in the status bar.

## Build from source

```
git clone https://github.com/marelevn/soos
cd soos
cargo build --release
./target/release/soos-app     # the GUI
./target/release/soos-cli "20 inches in cm"
```

Needs Rust 1.95 or newer. On Debian or Ubuntu, install the GUI toolkit's
headers first:

```
sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
```

## Features

Click `?` in the status bar for a worked example: natural-language
phrasing, currency, units, dates and running totals. On your machine only
the exchange rates and today's date will differ.

- **Tabs** -- up to 9, each its own scratchpad. `+` in the tab strip or
  Cmd/Ctrl+T to add one; the `×` on a tab, a middle-click or Cmd/Ctrl+W to
  close it; Cmd/Ctrl+Shift+T to bring back the last one you closed;
  Cmd/Ctrl+1-9 or Ctrl+Tab / Ctrl+Shift+Tab to switch.
- **The result column** -- every result ends on the column's right edge,
  and totals are bold. An error on the line you're typing waits until you
  pause, and a slow document never holds up the typing: until its results
  are ready, the old ones stay, dimmed.
- **Natural-language phrasing** -- `5% on 30`, `6% off 40 EUR`,
  `20% of what is 30 cm`, `$8 times 3`, `20 ml in tea spoons`.
- **Live currency** -- central-bank rates from
  [Frankfurter](https://frankfurter.dev), no API key, cached on disk so
  conversion keeps working offline. Before the first rates arrive, a
  document in one currency still works (`$8 * 3`): only converting between
  two needs rates. The status bar says when the rates are more than a day
  old. Click a result to copy it without thousands separators
  (`$1234567.89`); it pastes back into Soos as the same amount.
- **Your own converters** -- click the `↔` symbol in the status bar
  for a small table (unit, aliases, base, factor) kept separate from the
  document, so clearing or rewriting your scratchpad never loses one. A bad
  alias is dropped rather than failing the whole row.
- **The status bar icons**, left to right: `↔` converters, `?` the
  example, `▲` always on top, `±` high precision (every digit of a currency
  amount instead of cents), the theme (`◌` follows the system, `○` light,
  `●` dark) and `⌨` the global hotkey. Hover over one for what it does.
  Linux has no `▲` or `⌨` (see below).
- **Desktop integration** -- a tray icon: click it to show the window,
  right-click it for Quit. On Windows and macOS, closing the window hides
  it to the tray (on macOS, the Dock icon goes too), or quits if there's
  no tray, and a global hotkey shows and hides it (the Calculator key by
  default on Windows, Ctrl+Shift+Space on macOS; click `⌨` and press a
  new one). On Linux, closing quits, and there's no global hotkey or
  always on top: Wayland can't hide a window, raise it from a hotkey, or
  keep it on top. Some Linux desktops (GNOME without an AppIndicator
  extension) show no tray. Cmd/Ctrl +/-/0 zooms the text.
- **`soos-cli`** for terminals and scripts: `soos-cli '20 inches in cm'`
  prints what the app shows beside that line, using the app's rates and
  your converters. A PowerToys Run plugin is a thin shell over it -- see
  [Extra guides](#extra-guides).

## How it calculates

- **Blocks.** Blank lines, `# headings` and `Label:` lines end a block.
  Every result in a block counts toward its `sum`, including a line like
  `rent = 1800`. If any line in the block has an error, `sum` and `avg`
  show an error too rather than a total that quietly leaves it out. `avg`
  of an empty block is an error; `sum` of one is 0.
- **Units in a total.** Values in one unit, or in convertible units, are
  added as such (`1 m` + `50 cm` = `1.5 m`). One unit plus plain numbers
  totals in that unit (`5 m`, `3` gives `8 m`). Two units that don't
  convert (`5 m`, `3 kg`) are an error.
- **Reserved words.** `sum`, `total`, `avg`, `average`, `prev`, `today`,
  `tomorrow`, `yesterday`, `now`, `in`, `to`, `of`, `on`, `off`, `as`,
  `into` and `times` can't be variable or converter names.
- **Percent.** `5% on 30` adds 5%, `6% off 40` takes it off, and so do
  `30 + 5%` and `40 - 6%`, as on a desk calculator. `100 * 15%` is 15 and
  `100 / 20%` is 500. `20% of what is 30` finds the whole, `10 as a % of
  40` gives 25%. `fee on cost` needs `fee` to hold a percent (`fee = 8%`).
- **Rounding.** Currencies are rounded half up to their usual decimals (none
  for JPY, KRW, VND and IDR; two for the rest) unless `±` is on. A result
  that isn't exact shows `≈` and four decimals, or four significant digits
  below 1; hover it, copy it or turn `±` on for every digit. An exact
  result shows all of its digits. `prev`, `sum` and `avg` use the full
  value, not the digits shown: `1/3`, then `prev * 3`, is exactly 1, and
  `sqrt(2)`, then `prev^2`, is `≈ 2`.
- **Currency symbols.** `$` is US dollars; Australian and Singapore
  dollars and Chinese yuan are `A$`, `S$` and `CN¥`, so no two currencies
  look the same. These and `€ £ ¥ ₩ ₹ ₫ ฿ ₱ Rp RM` can also be typed; any
  other currency is shown with its code (`12 CAD`).
- **Dates.** `today`, `tomorrow`, `yesterday`, `now`, and any of them
  plus or minus minutes, hours (`now` only), days, weeks, months or years.
  A date is written `@2026-12-25`; without the `@`, `2026-12-25` is an
  error rather than a subtraction.
- **Time zones.** `3pm PST in Tokyo`, `now in UTC`: a city, an IANA name or
  an abbreviation. Regional abbreviations follow daylight saving (`PST` in
  July means PDT); `GMT` is always UTC+0; `IST` is India and `CST` US
  Central.
- **Units Soos adds.** `px`, `pt` (1/72 inch), `pc`, `rem` and `em`
  (16 px) and `ch` (8 px). `pt` is points, not pints: write `pint`.
- **Units that read two ways.** `5 ft 11 in` and `3 in` are inches, not a
  conversion; `sin 30 deg` is the sine of 30 degrees, like `sin(30 deg)`.

## Extra guides

<details>
<summary><b>soos-cli: the basics (all platforms)</b></summary>

`soos-cli` prints what the app would show beside a line: same engine,
currency symbols, rounding, thousands separators and error labels. It ships
next to the app in every release and uses the app's exchange-rate cache and
your converters.

```
$ soos-cli '20 inches in cm'
50.8 cm
$ soos-cli 'Rent: $1800 * 12'
$21,600.00
$ soos-cli '3pm PST in Tokyo'
2026-09-29 07:00 JST
$ printf '1200\n340\nsum\n' | soos-cli -
1,540
```

(The time-zone answer depends on today's date.)

**Input.** All non-option arguments are joined into one line. Quote the
expression anyway, since shells treat `$`, `*` and `%` specially (see
**Quoting** below). With `-` instead of an expression, the input is read from
stdin and can span several lines: it's calculated like a tab in the app
(`sum`, `prev` and variables work across lines), and the last line with a
result is printed. Labels (`Rent:`), `// comments` and `"notes"` are
ignored, as in the app.

**Options.**

| Option | What it does |
|---|---|
| `--json` | One line of JSON (keys in alphabetical order): `{"ok":true,"result":"$2,469.00","value":"$2469.00"}` or `{"detail":"unknown identifier 'metr'","error":"unknown metr","ok":false}`. `result` is what the app shows, `value` what clicking it in the app copies (no thousands separators or `≈`; a currency keeps its symbol), `detail` the full error message (the app's hover text). |
| `--high-precision` | Keep every digit instead of rounding currencies -- the app's `±` toggle. |
| `-h`, `--help` / `-V`, `--version` | Help / version. |
| `--` | Everything after it is the expression, even if it starts with `--`. |

**Exit status.** 0 with the result on stdout; 1 for an error (short label on
stderr, then the full message indented on the next line) or when there's
nothing to calculate; 2 for a usage mistake.

**Rates and converters.** Currency lines need a cached exchange rate. If the
cache is more than 6 hours old, `soos-cli` fetches fresh rates first (for at
most 5 seconds) and otherwise answers from the cache; after a failed fetch
it doesn't try again for 10 minutes, so an offline launcher stays fast.
With no cache and no network, a document in one currency still works, and
a line that needs a second currency reports `no rates yet`.
Converters you add in the app (`↔`) are saved for `soos-cli` when you close
the converters window, and whenever the app saves. To share both, run the
`soos-cli` that came with your app, or a symlink to it; a copy elsewhere
keeps its own data, except on macOS. See your platform's guide.

**Quoting (bash, zsh).** Use single quotes. In double quotes `$840` is
expanded as a shell variable, and unquoted, zsh treats `*` as a filename
pattern (`zsh: no matches found`). Windows shells differ; see its guide.

```
soos-cli '$840 * 2'          # right
soos-cli "$840 * 2"          # wrong: the shell turns $840 into "40"
```

**Scripting.** Use `--json` and read `value`: the result without thousands
separators. A currency keeps its symbol (`€0.92`), so strip it if another
tool needs a bare number:

```
soos-cli --json '1 USD to EUR' | jq -r .value
```

</details>

<details>
<summary><b>soos-cli on macOS</b></summary>

`soos-cli` is inside the app bundle, next to the app itself:
`/Applications/Soos.app/Contents/MacOS/soos-cli`.

1. Install Soos from the `.dmg` and clear the quarantine flag once (the
   build is unsigned, so macOS refuses to run it otherwise -- see
   [Install](#install)):

   ```
   xattr -cr /Applications/Soos.app
   ```

2. Put it on your `PATH` with a symlink, so it stays current when you
   update the app:

   ```
   sudo mkdir -p /usr/local/bin
   sudo ln -sf /Applications/Soos.app/Contents/MacOS/soos-cli /usr/local/bin/soos-cli
   ```

3. Open a new terminal and try it:

   ```
   soos-cli '20 inches in cm'
   ```

**Data.** On macOS the app and `soos-cli` always share
`~/Library/Application Support/Soos/` (rates, converters, settings),
wherever the binary lives -- so a copy, or one built with
`cargo install --path crates/soos-cli`, sees the same rates and converters.

</details>

<details>
<summary><b>soos-cli on Linux</b></summary>

The release `.tar.gz` holds `soos-app` and `soos-cli` side by side. Both keep
their data (rates, converters, settings) in a `data/` folder next to the
real binary, so they share it as long as they stay in the same folder.

1. Extract the archive somewhere permanent that you can write to (the
   `data/` folder is created there), e.g. `~/.local/opt`:

   ```
   mkdir -p ~/.local/opt
   tar xzf soos-linux-x86_64.tar.gz -C ~/.local/opt
   ```

2. Put `soos-cli` on your `PATH` with a **symlink**. The symlink is followed
   back to the real binary, so the CLI still uses the app's `data/` folder.
   A *copy* would keep its own, separate `data/` and not see the
   converters you define in the app.

   ```
   mkdir -p ~/.local/bin
   ln -sf ~/.local/opt/soos-linux-x86_64/soos-cli ~/.local/bin/soos-cli
   ```

   Most distributions put `~/.local/bin` on `PATH`. If `soos-cli` isn't
   found, add `export PATH="$HOME/.local/bin:$PATH"` to your `~/.bashrc`
   or `~/.zshrc` and open a new terminal.

3. Try it:

   ```
   soos-cli '20 inches in cm'
   ```

4. Optionally, add Soos to your applications menu. The archive's
   `soos.desktop` needs the real paths first:

   ```
   d=~/.local/opt/soos-linux-x86_64
   sed "s|^Exec=.*|Exec=$d/soos-app|; s|^Icon=.*|Icon=$d/soos.png|" \
     "$d/soos.desktop" > ~/.local/share/applications/soos.desktop
   ```

**Built from source?** `cargo install --path crates/soos-cli` installs to
`~/.cargo/bin` and keeps its data in `~/.cargo/bin/data/`, separate from the
app's. Prefer the symlink above to share rates and converters.

</details>

<details>
<summary><b>soos-cli on Windows</b></summary>

The release zip holds `soos-app.exe` and `soos-cli.exe` side by side. Both
keep their data (rates, converters, settings) in a `data` folder next to the
`.exe`, so they share it as long as they stay in the same folder.

1. Unblock the zip (the build is unsigned) and extract it somewhere
   permanent that you can write to. In PowerShell, from the folder you
   downloaded it to:

   ```powershell
   Unblock-File .\soos-windows-x86_64.zip
   Expand-Archive .\soos-windows-x86_64.zip "$env:LOCALAPPDATA\Programs\Soos"
   ```

   That gives you `%LOCALAPPDATA%\Programs\Soos\soos-windows-x86_64\` with
   both `.exe` files in it.

2. Add that folder to your user `PATH`. In PowerShell:

   ```powershell
   $dir = "$env:LOCALAPPDATA\Programs\Soos\soos-windows-x86_64"
   $path = [Environment]::GetEnvironmentVariable("Path", "User")
   [Environment]::SetEnvironmentVariable("Path", "$path;$dir", "User")
   ```

   (Or: Start → "Edit environment variables for your account" → `Path` →
   **New**.) Don't copy `soos-cli.exe` somewhere else instead: a copy uses
   the `data` folder next to *itself* and won't see the app's converters.

3. Open a **new** terminal and try it:

   ```
   soos-cli "20 inches in cm"
   ```

**Quoting.**

- *PowerShell:* use single quotes -- in double quotes `$840` is read as a
  variable.

  ```powershell
  soos-cli '$840 * 2'
  soos-cli --json '1 USD to EUR' | ConvertFrom-Json | Select-Object -ExpandProperty value
  ```

- *Command Prompt (cmd.exe):* use double quotes. In a `.bat` file, write
  `%` as `%%` (`"20%% off 40"`); typed at the prompt, a single `%` is fine.

  ```
  soos-cli "$840 * 2"
  ```

**Unicode output.** Results can contain symbols such as `€` or `≈`.
Printed straight to the terminal they show correctly (in the legacy console,
given a font that has them). When you *pipe* `soos-cli`'s output into
PowerShell, it arrives as UTF-8 but PowerShell decodes it with the console's
code page, so set this first or the symbols come out garbled:

```powershell
[Console]::OutputEncoding = [Text.Encoding]::UTF8
```

</details>

<details>
<summary><b>Alfred (macOS)</b></summary>

On macOS, use Raycast or Quicksilver, which calculate on their own: Raycast
out of the box, Quicksilver with its Calculator plugin.

</details>

<details>
<summary><b>PowerToys Run plugin (Windows)</b></summary>

Type `=` and an expression in PowerToys Run; the answer appears as a result,
exactly as the app would show it. Enter copies the plain value (no
thousands separators).

1. Set up `soos-cli.exe` on your `PATH` (see **soos-cli on Windows** above).
   Or leave `PATH` alone and point the plugin at it with an environment
   variable:

   ```powershell
   [Environment]::SetEnvironmentVariable("SOOS_EXE", "$env:LOCALAPPDATA\Programs\Soos\soos-windows-x86_64\soos-cli.exe", "User")
   ```

2. Build the plugin (needs the .NET 9 SDK):

   ```
   cd integrations\powertoys
   dotnet build -c Release -p:Platform=x64
   ```

   Use `-p:Platform=ARM64` on Arm devices, and the matching output folder
   below.

3. Quit PowerToys (tray icon → **Exit**), copy the *contents* of
   `integrations\powertoys\bin\x64\Release\net9.0-windows10.0.26100.0\` to
   `%LOCALAPPDATA%\Microsoft\PowerToys\PowerToys Run\Plugins\Soos\`, and
   start PowerToys again.
4. `Alt+Space`, then `=20 inches in cm`.

**Troubleshooting.**

- *"soos-cli.exe not found":* PowerToys was started before `PATH` or
  `SOOS_EXE` was set -- restart PowerToys (or sign out and back in).
- *PowerToys' own Calculator answers too:* it also uses `=`. Give Soos a
  different activation keyword in PowerToys Settings → PowerToys Run →
  Plugins → Soos, or turn off the built-in Calculator plugin.
- *Currency results say `no rates yet`:* converting between currencies
  needs rates, which couldn't be fetched and aren't cached yet -- open the
  app once while online.
- *A converter from the app isn't recognised:* open and close the app's
  converters window (`↔`) so it saves them for `soos-cli`.

</details>

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the code layout, the checks CI
runs, and the release process.

## License

[MIT](LICENSE)

Built on [fend](https://github.com/printfn/fend),
[egui/eframe](https://github.com/emilk/egui) and other MIT/Apache-2.0
crates, listed in the `Cargo.toml` files.
The app's typeface is [JetBrains Mono](https://www.jetbrains.com/lp/mono/),
Regular and Bold, under the [SIL Open Font License 1.1](assets/fonts/OFL.txt);
currency signs it lacks come from a subset of
[DejaVu Sans Mono](https://dejavu-fonts.github.io/), under
[its license](assets/fonts/DejaVu-LICENSE.txt).
Rates from [Frankfurter](https://frankfurter.dev); thanks to
[Numi](https://github.com/nikolaeu/numi) for the idea.
