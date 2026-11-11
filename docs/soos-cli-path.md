# Put soos-cli on your PATH

[Install Soos](install-help.md#where-to-put-it) first. Then make the
`soos-cli` that came with it reachable from any terminal, without copying
it: a copy keeps its own data (see [Data folder](soos-cli.md#data-folder)). On macOS
and Linux, use a symlink; on Windows, add the folder to your `PATH`.

## macOS

`soos-cli` is inside the app bundle:
`/Applications/Soos.app/Contents/MacOS/soos-cli`. If you installed Soos with
Homebrew, it's already linked: skip to step 3.

1. Clear the quarantine flag once, as in
   [install help](install-help.md#macos-says-soosapp-not-opened).
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

## Linux

The release `.tar.gz` holds `soos-app` and `soos-cli` side by side.

1. [Extract it](install-help.md#linux), if you haven't.
2. Symlink `soos-cli` into a folder on your `PATH`. Change the first path if
   you extracted Soos somewhere else:

   ```
   mkdir -p ~/.local/bin
   ln -sf ~/.local/opt/soos-linux-x86_64/soos-cli ~/.local/bin/soos-cli
   ```

3. Open a new terminal and try it:

   ```
   soos-cli '20 inches in cm'
   ```

The symlink is followed back to the real binary, so the CLI still uses the
app's `data/` folder. Most distributions put `~/.local/bin` on `PATH`. If
`soos-cli` isn't found, yours doesn't: add
`export PATH="$HOME/.local/bin:$PATH"` to your `~/.bashrc` or `~/.zshrc`,
and open a new terminal.

## Windows

The release zip holds `soos-app.exe` and `soos-cli.exe` side by side.

1. [Unzip it](install-help.md#windows), if you haven't.
2. Press Start, type "Edit environment variables for your account", and open
   it.
3. Select `Path`, click **Edit**, then **New**, and paste the path of the
   folder that holds `soos-cli.exe`. Click **OK** in each window.
4. Open a **new** terminal and try it:

   ```
   soos-cli "20 inches in cm"
   ```

Results can contain symbols such as `€` or `≈`, which a terminal shows
correctly. If you pipe results into PowerShell: run this first, or the
symbols come out garbled:

```powershell
[Console]::OutputEncoding = [Text.Encoding]::UTF8
```
