# Uninstall Soos

## Android

Uninstall it as you would any app. That deletes your tabs and settings too.

## Windows and Linux

1. Quit Soos.
2. Delete its folder. That deletes your tabs and settings too: they are in
   its `data` folder.
3. Remove what you set up around it, if anything:
   - Windows: the folder in your `PATH`, the
     [`SOOS_EXE`](powertoys.md#install) variable, and the PowerToys plugin
     folder
     (`%LOCALAPPDATA%\Microsoft\PowerToys\PowerToys Run\Plugins\Soos\`).
   - Linux: `~/.local/share/applications/soos.desktop` and the
     `~/.local/bin/soos-cli` link.

## macOS

If you installed with Homebrew: quit Soos and run `brew uninstall --cask soos`.
That keeps your tabs; `brew uninstall --cask --zap soos` deletes them too.
Otherwise:

1. Quit Soos.
2. Drag Soos from Applications to the Trash.
3. To delete your tabs and settings too, delete
   `~/Library/Application Support/Soos/`.
4. If you linked `soos-cli`, remove the link:
   `sudo rm /usr/local/bin/soos-cli`.
