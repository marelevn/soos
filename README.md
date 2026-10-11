<div align="center">

<img src="assets/icons/hicolor/256x256.png" width="120" alt="Soos logo">

# Soos

A notepad-style calculator for Windows, macOS and Linux: every line is an
expression, and its answer shows up beside it.

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white)](https://github.com/marelevn/soos/releases/latest/download/soos-windows-x86_64.zip)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/marelevn/soos/releases/latest/download/Soos.dmg)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://github.com/marelevn/soos/releases/latest/download/soos-linux-x86_64.tar.gz)

Windows 64-bit (Intel or AMD) · macOS (Apple silicon or Intel) · Linux
64-bit (Intel or AMD)

![Soos adding up rent, food and a bus pass, then a variable, prev and avg](assets/screenshot.png)

[![CI](https://github.com/marelevn/soos/actions/workflows/ci.yml/badge.svg)](https://github.com/marelevn/soos/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/marelevn/soos)](https://github.com/marelevn/soos/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

## Install

1. Download the file for your system from the buttons above.
2. Put Soos where it can stay:
   - **Windows / Linux:** extract the download into a folder you'll keep,
     not Downloads, because Soos saves your tabs next to itself. Then
     double-click `soos-app` (on Linux you can also run `./soos-app` from
     that folder).
   - **macOS:** open the `.dmg` and drag Soos to Applications (your tabs live
     in Application Support, so the app can be replaced).
3. Windows and macOS warn you the first time, because Soos isn't signed yet:
   - **Windows** says "Windows protected your PC": click **More info**, then
     **Run anyway**.
   - **macOS** shows “Soos.app” Not Opened. Nothing is wrong with it: click
     **Done**, not **Move to Trash**. Then open Terminal (Cmd+Space, type
     `Terminal`, press Return), paste this line and press Return:

     ```
     xattr -cr /Applications/Soos.app
     ```

     It prints nothing; that's normal. Then open Soos again. Older macOS
     versions may say "Soos is damaged and can't be opened" instead; the
     same steps work for it.

   [Install help](docs/install-help.md) has the details, and how to update
   or uninstall.

## Quick guide

Open Soos and type a calculation, like `12 * 3` (`*` means times); the
answer appears on the right. A whole document looks like this:

    Rent: 1800                     1,800
    Food: 4 * 150                    600
    Bus pass: 12 * 2.5                30
    sum                            2,430

    rent = 1800                    1,800
    rent * 12                     21,600
    prev / 2                      10,800
    avg                           11,400

- `Rent:` is a label: only what follows it is calculated.
- A blank line, a `# heading` or a label alone on its line (`Costs:`) starts
  a new block.
- `sum` (or `total`) adds up the lines above it in its block.
- `avg` (or `average`) averages them. Every line with a result counts,
  `rent = 1800` included: the second block's `avg` is 11,400.
- `prev` is the previous result.
- `rent = 1800` makes a variable for every line below it. A label doesn't
  make a variable: `Rent:` above it doesn't count.

More in [Totals and variables](docs/totals.md).

Soos goes online for two things, and sends no user data with either:
refreshing exchange rates, and checking for a new version when you click the
version number in the status bar.

## Features

- [Phrases](docs/phrasing.md): `5% on 30`, `4 plus 4`, `20 sq cm`.
- [Currency](docs/money-and-units.md): `$8 * 3`, `20 EUR in USD`, with saved
  rates for offline use.
- [Dates and time zones](docs/dates-and-time.md): `today + 3 days`,
  `3PM PST in Tokyo`.
- [Tabs](docs/the-app.md#tabs): up to 9 scratchpads, saved when you close
  Soos.
- [Your own units](docs/money-and-units.md#your-own-converters): a small
  table, kept apart from your text.
- [Tray icon and hotkey](docs/the-app.md#tray-dock-and-hotkey): bring Soos
  up from any app (Windows and macOS).
- [In a terminal](docs/soos-cli.md): `soos-cli '20 inches in cm'` prints
  `50.8 cm`.

## How it calculates

Soos calculates with [fend](https://github.com/printfn/fend) and adds rules
of its own:

- `,` groups thousands and `.` is the decimal point, so `1,5` is 15. The
  `0.1` button in the status bar swaps them.
  [Decimal comma](docs/the-app.md#decimal-comma)
- Currencies round to their usual decimals: `$1234.567` shows as
  `$1,234.57`. `≈` marks a result that isn't exact, like `1/3`.
  [Rounding](docs/money-and-units.md#rounding)
- A date on its own is written `@2026-12-25`. Without the `@`, Soos shows an
  error instead of subtracting. [Dates](docs/dates-and-time.md)
- A time of day takes a capital `AM` or `PM`: `3PM`. Lowercase `3pm` is 3
  picometres. [Times of day](docs/dates-and-time.md#times-of-day)
- `in` is also the unit inches: `5 ft 11 in` is 5 feet 11 inches. Between
  two units it converts: `20 inches in cm`.
  [Units](docs/money-and-units.md#units)

| [![Percent of, on, off and of what is](docs/img/percent.svg)](docs/phrasing.md#percent) | [![35 days before and after a date](docs/img/dates.svg)](docs/dates-and-time.md) |
|---|---|

## Guide

Every topic, with examples: [the guide](docs/README.md).

<details>
<summary><b>PowerToys Run (Windows)</b></summary>

Type `=20 inches in cm` in PowerToys Run and the answer appears. You build
the plugin from this repository; see the
[plugin guide](docs/powertoys.md).

</details>

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) to build from source, and for the
code layout, the checks CI runs and the release process.

## License

[MIT](LICENSE)

## Credits

- [fend](https://github.com/printfn/fend), which does the calculating.
- [egui/eframe](https://github.com/emilk/egui) and other MIT/Apache-2.0
  crates, listed in the `Cargo.toml` files.
- The app's typeface is [JetBrains Mono](https://www.jetbrains.com/lp/mono/),
  Regular and Bold, under the [SIL Open Font License 1.1](assets/fonts/OFL.txt).
  Currency signs it lacks come from a subset of
  [DejaVu Sans Mono](https://dejavu-fonts.github.io/), under
  [its license](assets/fonts/DejaVu-LICENSE.txt).
- Exchange rates from [Frankfurter](https://frankfurter.dev).
- Thanks to [Numi](https://github.com/nikolaeu/numi) for the idea.
