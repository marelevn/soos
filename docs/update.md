# Update Soos

Soos saves your tabs when you close it, so quit it before you update. The
downloads hold no `data` folder, so unpacking over the old copy keeps your
tabs.

## Windows

1. Quit Soos: right-click its tray icon, then click **Quit**. Closing the
   window only hides it.
2. Extract the new `.zip` into the same folder as before, and replace the
   files when Windows asks.
3. Start Soos.

If you'd rather keep the old folder: extract the new `.zip` somewhere new,
and move the old `data` folder into the new `soos-windows-x86_64` folder.

## macOS

Your tabs live in `~/Library/Application Support/Soos/`, not in the app, so
replacing the app keeps them. If you installed with Homebrew: quit Soos
and run `brew upgrade --cask soos` instead of these steps.

1. Quit Soos: right-click its menu bar icon, then click **Quit**.
2. Open the new `.dmg` and drag Soos to Applications, replacing the old one.
3. The new copy is unsigned too: run `xattr -cr /Applications/Soos.app`
   again, as in [Install Soos](install-help.md#macos-says-soosapp-not-opened).
4. Open Soos.

## Linux

1. Close Soos.
2. Extract the new archive over the old folder (here, `~/.local/opt`, as in
   [Install Soos](install-help.md#linux)):

   ```
   tar xzf soos-linux-x86_64.tar.gz -C ~/.local/opt
   ```

3. Start Soos.

## Android

Download the new `soos-android.apk` and open it. Android installs it over
the old Soos and keeps your tabs: every release is signed with the same
key, which is how Android knows it's the same app.
