# soos-cli

`soos-cli` prints what the app would show beside a line: same engine,
currency symbols, rounding, thousands separators and error labels. It ships
next to the app in every release, and uses the app's exchange-rate cache and
your converters.

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

[macOS](#macos) · [Linux](#linux) · [Windows](#windows) ·
[Using it](#using-it)

## Put it on your PATH

[Install Soos](install-help.md#where-to-put-it) first. Then make `soos-cli`
reachable from any terminal without copying it: a copy keeps its own data
(see [Rates and converters](#rates-and-converters)). On macOS and Linux, use
a symlink; on Windows, add the folder to your `PATH`.

### macOS

`soos-cli` is inside the app bundle, next to the app itself:
`/Applications/Soos.app/Contents/MacOS/soos-cli`.

1. Clear the quarantine flag once, as in
   [install help](install-help.md#macos-says-soos-is-damaged-and-cant-be-opened).
   macOS refuses to run the unsigned `soos-cli` otherwise.

2. Symlink it, so it stays current when you update the app:

   ```
   sudo mkdir -p /usr/local/bin
   sudo ln -sf /Applications/Soos.app/Contents/MacOS/soos-cli /usr/local/bin/soos-cli
   ```

3. Open a new terminal and try it:

   ```
   soos-cli '20 inches in cm'
   ```

### Linux

The release `.tar.gz` holds `soos-app` and `soos-cli` side by side. After
[extracting it](install-help.md#linux):

1. Symlink `soos-cli` into a folder on your `PATH`. The symlink is followed
   back to the real binary, so the CLI still uses the app's `data/` folder.

   ```
   mkdir -p ~/.local/bin
   ln -sf ~/.local/opt/soos-linux-x86_64/soos-cli ~/.local/bin/soos-cli
   ```

   Change the first path if you extracted Soos somewhere else.

   Most distributions put `~/.local/bin` on `PATH`. If `soos-cli` isn't
   found, add `export PATH="$HOME/.local/bin:$PATH"` to your `~/.bashrc`
   or `~/.zshrc` and open a new terminal.

2. Try it:

   ```
   soos-cli '20 inches in cm'
   ```

### Windows

The release zip holds `soos-app.exe` and `soos-cli.exe` side by side. After
[unzipping it](install-help.md#windows), add the folder they are in to your
user `PATH`.

1. Press Start, type "Edit environment variables for your account", and open
   it.
2. Select `Path`, click **Edit**, then **New**, and paste the folder's path.
   Click **OK** in each window.
3. Open a **new** terminal and try it:

   ```
   soos-cli "20 inches in cm"
   ```

Results can contain symbols such as `€` or `≈`. Printed straight to the
terminal they show correctly (in the legacy console, given a font that has
them). When you *pipe* `soos-cli`'s output into PowerShell, it arrives as
UTF-8 but PowerShell decodes it with the console's code page, so run this
first or the symbols come out garbled:

```powershell
[Console]::OutputEncoding = [Text.Encoding]::UTF8
```

## Using it

### Input

All non-option arguments are joined into one line. Quote the expression
anyway, since shells treat `$` and `*` specially (see
[Quoting](#quoting)).

With `-` instead of an expression, the input is read from stdin and can span
several lines. It's calculated like a tab in the app (`sum`, `prev` and
variables work across lines), and the last line with a result is printed.
As in the app, `// comments` and `"notes"` are skipped, and so is a label:
in `Rent: 1800` only `1800` is calculated.

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

The keys come in alphabetical order:

- `ok`: whether there is a result.
- `result`: what the app shows.
- `value`: what clicking the result in the app copies. It has no thousands
  separators or `≈`, and a currency keeps its symbol.
- `error`: the short label of an error.
- `detail`: the full error message, which the app shows on hover.

### Exit status

- 0: the result is on stdout.
- 1: an error (the short label on stderr, then the full message indented on
  the next line), or nothing to calculate.
- 2: a usage mistake.

### Rates and converters

Currency lines need an exchange rate. If the saved rates are more than 6
hours old, `soos-cli` fetches new ones first (for at most 5 seconds) and
otherwise answers from what it has. After a failed fetch it doesn't try
again for 10 minutes, so a launcher such as PowerToys Run stays fast.

With no saved rates and no network, a document in one currency still works.
A line that needs a second currency reports `no rates yet`.

Converters you add in the app (`↔`) are saved for `soos-cli` when you close
the converters window, and whenever the app saves.

The app and `soos-cli` share their rates and converters only when they use
the same data folder. On Windows and Linux that is a `data` folder next to
the real binary, so run the `soos-cli` that came with your app, or a symlink
to it. A copy elsewhere keeps its own `data` folder, and so does a build from
`cargo install --path crates/soos-cli` (it keeps it in `~/.cargo/bin/data/`);
neither sees the converters you made in the app. On macOS the data folder is
always `~/Library/Application Support/Soos/`, wherever the binary lives, so a
copy or a `cargo install` build shares it too.

Settings such as `±` stay in the app; `soos-cli` takes `--high-precision`
instead.

### Quoting

#### bash and zsh

Use single quotes. In double quotes the shell reads `$840` as a variable, and
unquoted, zsh treats `*` as a filename pattern (`zsh: no matches found`).

```
soos-cli '$840 * 2'          # right
soos-cli "$840 * 2"          # wrong: the shell turns $840 into "40"
```

#### PowerShell

Use single quotes. In double quotes `$840` is read as a variable.

```powershell
soos-cli '$840 * 2'
soos-cli --json '1 USD to EUR' | ConvertFrom-Json | Select-Object -ExpandProperty value
```

#### Command Prompt (cmd.exe)

Use double quotes. In a `.bat` file, write `%` as `%%` (`"20%% off 40"`);
typed at the prompt, a single `%` is fine.

```
soos-cli "$840 * 2"
```

### Scripting

Use `--json` and read `value`: the result without thousands separators. A
currency keeps its symbol (`€0.92`), so strip it if another tool needs a bare
number:

```
soos-cli --json '1 USD to EUR' | jq -r .value
```
