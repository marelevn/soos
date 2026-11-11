# Install the PowerToys Run plugin

On Windows, the plugin lets you type `=` and an expression in PowerToys Run
and get the answer as the app would show it; Enter copies the plain value.
There is no download: you build it from this repository.

## Install

1. Install PowerToys and the .NET 9 SDK, and clone this repository.
2. Put `soos-cli.exe` on your `PATH`: add the folder it is in (see
   [Put soos-cli on your PATH](soos-cli-path.md#windows)).

   If you'd rather leave `PATH` alone: point the plugin at `soos-cli.exe`
   with an environment variable. In PowerShell, with its real path:

   ```powershell
   [Environment]::SetEnvironmentVariable("SOOS_EXE", "C:\Tools\Soos\soos-cli.exe", "User")
   ```

3. Build the plugin. On an Arm device, use `-p:Platform=ARM64`, and `ARM64`
   instead of `x64` in step 5's folder.

   ```
   cd integrations\powertoys
   dotnet build -c Release -p:Platform=x64
   ```

4. Quit PowerToys: tray icon, then **Exit**.
5. Copy the *contents* of
   `integrations\powertoys\bin\x64\Release\net9.0-windows10.0.26100.0\` to
   `%LOCALAPPDATA%\Microsoft\PowerToys\PowerToys Run\Plugins\Soos\`.
6. Start PowerToys again.
7. Press `Alt+Space`, then type `=20 inches in cm`.

## Troubleshooting

- *"soos-cli.exe not found":* PowerToys was started before `PATH` or
  `SOOS_EXE` was set. Restart PowerToys, or sign out and back in.
- *PowerToys' own Calculator answers too:* it also uses `=`. Give Soos a
  different activation keyword in PowerToys Settings, then PowerToys Run,
  then Plugins, then Soos. Or turn off the built-in Calculator plugin.
- *Currency results say `no rates yet`:* converting between currencies
  needs rates, which couldn't be fetched and aren't saved yet. Open the app
  once while online.
- *A converter from the app isn't recognized:* open and close the app's
  converters window (`↔`) so it saves them for `soos-cli`.
