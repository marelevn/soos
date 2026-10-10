<div align="center">

<img src="assets/icons/hicolor/256x256.png" width="120" alt="Soos logo">

# Soos

A notepad-style calculator for Windows, macOS and Linux: every line is an
expression, and its answer shows up beside it.

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white)](https://github.com/marelevn/soos/releases/latest/download/soos-windows-x86_64.zip)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/marelevn/soos/releases/latest/download/Soos.dmg)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://github.com/marelevn/soos/releases/latest/download/soos-linux-x86_64.tar.gz)

![Soos, mid-calculation](assets/screenshot.png)

[![CI](https://github.com/marelevn/soos/actions/workflows/ci.yml/badge.svg)](https://github.com/marelevn/soos/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/marelevn/soos)](https://github.com/marelevn/soos/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

## Install

- **Windows** (64-bit Intel or AMD): unzip to a folder you'll keep, then run
  `soos-app.exe`.
- **macOS** (Apple silicon or Intel): open the `.dmg`, then drag Soos to
  Applications.
- **Linux** (64-bit Intel or AMD): extract the `.tar.gz` to a folder you can
  write to, then run `soos-app`.

Soos keeps your tabs and settings in a `data` folder beside the program, so
put it where it can stay. On macOS they live in
`~/Library/Application Support/Soos/`.

The builds aren't code-signed yet, so Windows and macOS warn you the first
time you open Soos. [How to open it anyway](docs/install-help.md).

Soos goes online for two things, and sends no user data with either:
refreshing exchange rates, and checking for a new version when you click the
version number in the status bar.

## Quick guide

    Rent: 1800                     1,800
    Food: 4 * 150                    600
    Bus pass: 12 * 2.5                30
    sum                            2,430

    rent = 1800                    1,800
    rent * 12                     21,600
    prev / 2                      10,800
    avg                           11,400

`Rent:` is a label; only what follows it is calculated. A blank line, a
`# heading` or a label on its own line (`Costs:`) starts a new block, and
`sum` (or `total`) adds up the block above it. `avg` (or `average`) averages
that block, and `prev` is the previous result. A variable like `rent` can be
used on every line below it.
[Totals and variables](docs/totals.md).

## Features

- [Phrases](docs/phrasing.md): `5% on 30`, `4 plus 4`, `20 sq cm`.
- [Currency](docs/money-and-units.md): `$8 * 3`, `20 EUR in USD`. Rates are
  saved on your computer, so it works offline.
- [Dates and time zones](docs/dates-and-time.md): `today + 3 days`,
  `3PM PST in Tokyo`.
- [Tabs](docs/the-app.md#tabs): up to 9 scratchpads, each with its own text.
  Soos saves them when you close it.
- [Your own units](docs/money-and-units.md#your-own-converters): define them
  in a small table, kept apart from your text.
- [Tray icon and hotkey](docs/the-app.md#tray-dock-and-hotkey) (Windows and
  macOS): bring Soos up from any app.
- [In your terminal](docs/soos-cli.md): `soos-cli '20 inches in cm'` prints
  `50.8 cm`.

## How it calculates

Soos calculates with [fend](https://github.com/printfn/fend) and adds rules
of its own:

- Currencies round to their usual decimals: `$1234.567` shows as
  `$1,234.57`. `≈` marks a result that isn't exact, like `1/3`.
  [Rounding](docs/money-and-units.md#money)
- A date on its own is written `@2026-12-25`. Without the `@`, Soos shows an
  error instead of subtracting. [Dates](docs/dates-and-time.md)
- `in` is also the unit inches: `3 in` is 3 inches and `5 ft 11 in` is 5 feet
  11 inches. To convert, put `in` between two units: `20 inches in cm`.
  [Units](docs/money-and-units.md#units)

| [![Percent on, off and of what is](docs/img/percent.svg)](docs/phrasing.md#percent) | [![35 days before and after a date](docs/img/dates.svg)](docs/dates-and-time.md) |
|---|---|

## Guide

Every topic, with examples: [the guide](docs/README.md).

<details>
<summary><b>PowerToys Run (Windows)</b></summary>

Soos has a PowerToys Run plugin: type `=20 inches in cm` in PowerToys Run
and the answer appears. You build it from this repository; see the
[plugin guide](docs/powertoys.md).

</details>

<details>
<summary><b>Alfred (macOS)</b></summary>

Soos has no Alfred workflow. Raycast has a calculator built in, and
Quicksilver has one in its Calculator plugin.

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
