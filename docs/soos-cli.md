# soos-cli

`soos-cli` prints what the app would show beside a line: same engine,
currency symbols, rounding, thousands separators and error labels. It ships
next to the app in every release, and uses the app's exchange rates and your
converters. It always reads and prints a decimal point (`1,234.5`), whatever
the app's [decimal comma](the-app.md#decimal-comma) setting.

```
$ soos-cli '20 inches in cm'
50.8 cm
$ soos-cli 'Rent: $1800 * 12'
$21,600.00
$ soos-cli '3PM PST in Tokyo'
Tuesday, 29 September 2026 07:00 JST
$ printf '1200\n340\nsum\n' | soos-cli -
1,540
```

(The time-zone answer depends on today's date.)

To run it from any terminal, see [Put soos-cli on your PATH](soos-cli-path.md).

## Using it

### Input

- All non-option arguments are joined into one line. Quote the expression
  anyway: shells treat `$` and `*` specially (see [Quoting](#quoting)).
- With `-` instead of an expression, the input is read from stdin and can
  span several lines. It is calculated like a tab in the app (`sum`, `prev`
  and variables work across lines).
- Only the last line with a result or an error is printed, and `--json`
  works the same way. An error on an earlier line doesn't show:
  `printf 'metr\n5\n' | soos-cli -` prints `5`.
- As in the app, `// comments`, `"notes"` and labels are skipped: in
  `Rent: 1800` only `1800` is calculated.

### Options

| Option | What it does |
|---|---|
| `--json` | Prints one line of JSON; see [JSON output](#json-output). |
| `--high-precision` | Keeps every digit instead of rounding currencies: the app's `±` toggle. |
| `-h`, `--help` | Prints help. |
| `-V`, `--version` | Prints the version. |
| `--` | Everything after it is the expression, even if it starts with `--`. |

### JSON output

```
$ soos-cli --json '$2469'
{"ok":true,"result":"$2,469.00","value":"$2469.00"}
$ soos-cli --json 'metr'
{"detail":"unknown identifier 'metr'","error":"unknown metr","ok":false}
```

The keys come in alphabetical order. They hold:

| Key | Holds |
|---|---|
| `ok` | whether there is a result |
| `result` | what the app shows |
| `value` | what clicking the result in the app copies: no thousands separators or `≈`, and a currency keeps its symbol |
| `error` | the short label of an error |
| `detail` | the full error message, which the app shows on hover |

### Exit status

| Code | Means |
|---|---|
| 0 | The result is on stdout. |
| 1 | An error (the short label on stderr, then the full message indented on the next line), or nothing to calculate. |
| 2 | A usage mistake. |

### Rates and converters

- Currency lines need an exchange rate. If the saved rates are more than 6
  hours old, `soos-cli` fetches new ones first (for at most 5 seconds), and
  otherwise answers from what it has.
- After a failed fetch it doesn't try again for 10 minutes, so a launcher
  such as PowerToys Run stays fast.
- With no saved rates and no network, a document in one currency still
  works. A line that needs a second currency says `no rates yet`.
- `soos-cli` uses the converters you've set in the app (`↔`), once you
  close the converters window.
- Settings such as `±` stay in the app; `soos-cli` takes `--high-precision`
  instead.

### Data folder

The app and `soos-cli` share rates and converters only when they use the
same data folder:

- **Windows and Linux:** a `data` folder next to the real binary. Run the
  `soos-cli` that came with your app, or a symlink to it. A copy elsewhere
  keeps its own `data` folder, and so does a
  `cargo install --path crates/soos-cli` build (in `~/.cargo/bin/data/`);
  neither sees the converters you made in the app.
- **macOS:** always `~/Library/Application Support/Soos/`, wherever the
  binary lives, so a copy or a `cargo install` build shares it too.

### Quoting

| Shell | Quotes | Why |
|---|---|---|
| bash, zsh | single | In double quotes `$840` is read as a variable: bash turns it into `40`, zsh into nothing. Unquoted, zsh reads `*` as a filename pattern: `soos-cli 4*3` says `zsh: no matches found`. |
| PowerShell | single | In double quotes `$840` is read as a variable. |
| Command Prompt (`cmd.exe`) | double | In a `.bat` file, write `%` as `%%` (`"20%% off 40"`); typed at the prompt, a single `%` is fine. |

```
soos-cli '$840 * 2'          # bash, zsh, PowerShell
soos-cli "$840 * 2"          # Command Prompt
```

### Scripting

Use `--json` and read `value`: the result without thousands separators. A
currency keeps its symbol: for `1 USD to EUR` it is something like `€0.89`,
not `0.89`, so strip it if another tool needs a bare number. To read
`value`:

```
soos-cli --json '1 USD to EUR' | jq -r .value
```

In PowerShell:

```powershell
soos-cli --json '1 USD to EUR' | ConvertFrom-Json | Select-Object -ExpandProperty value
```
