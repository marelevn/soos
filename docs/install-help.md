# Install help

Download Soos from the [latest release](https://github.com/marelevn/soos/releases/latest).
The builds aren't code-signed yet, so Windows and macOS warn you the first
time you open it.

## Where to put it

Soos keeps your tabs and settings in a `data` folder beside the program (on
macOS, in `~/Library/Application Support/Soos/`), so use a folder you'll keep
and can write to.

### Windows

Unzip the download (right-click it, then **Extract All**) into a folder
you'll keep, and run `soos-app.exe`. To use `soos-cli` in a terminal too, see
[soos-cli on Windows](soos-cli.md#windows).

### macOS

Open the `.dmg` and drag Soos to Applications.

### Linux

Extract the archive into a folder you can write to, such as `~/.local/opt`:

```
mkdir -p ~/.local/opt
tar xzf soos-linux-x86_64.tar.gz -C ~/.local/opt
```

Run `~/.local/opt/soos-linux-x86_64/soos-app`. To add Soos to your
applications menu, the archive's `soos.desktop` needs the real paths first:

```
d=~/.local/opt/soos-linux-x86_64
mkdir -p ~/.local/share/applications
sed "s|^Exec=.*|Exec=$d/soos-app|; s|^Icon=.*|Icon=$d/soos.png|" \
  "$d/soos.desktop" > ~/.local/share/applications/soos.desktop
```

## macOS says "Soos is damaged and can't be opened"

Gatekeeper shows this for any unsigned app, and the download is fine. Open
Terminal (press Cmd+Space, type `Terminal`, press Return) and run this, which
removes the "downloaded from the internet" mark:

```
xattr -cr /Applications/Soos.app
```

Adjust the path if you moved Soos, then open it again.

## Windows shows "Windows protected your PC"

That's SmartScreen, for an unsigned app. Click **More info**, then **Run
anyway**.
