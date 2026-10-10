# PowerToys Run plugin (Windows)

Type `=` and an expression in PowerToys Run; the answer appears as a result,
as the app would show it. Enter copies the plain value (no thousands
separators).

There is no ready-made plugin to download: you build it from this
repository, which needs the .NET 9 SDK.

1. Put `soos-cli.exe` on your `PATH`: add the folder it is in (see
   [soos-cli on Windows](soos-cli.md#windows)). Or leave `PATH` alone and
   point the plugin at it with an environment variable. In PowerShell, with
   the real path to your `soos-cli.exe`:

   ```powershell
   [Environment]::SetEnvironmentVariable("SOOS_EXE", "C:\Tools\Soos\soos-cli.exe", "User")
   ```

2. Build the plugin:

   ```
   cd integrations\powertoys
   dotnet build -c Release -p:Platform=x64
   ```

   On an Arm device, use `-p:Platform=ARM64`, and `ARM64` instead of `x64` in
   the output folder below.

3. Quit PowerToys (tray icon, then **Exit**), copy the *contents* of
   `integrations\powertoys\bin\x64\Release\net9.0-windows10.0.26100.0\` to
   `%LOCALAPPDATA%\Microsoft\PowerToys\PowerToys Run\Plugins\Soos\`, and
   start PowerToys again.
4. Press `Alt+Space`, then type `=20 inches in cm`.

## Troubleshooting

- *"soos-cli.exe not found":* PowerToys was started before `PATH` or
  `SOOS_EXE` was set -- restart PowerToys (or sign out and back in).
- *PowerToys' own Calculator answers too:* it also uses `=`. Give Soos a
  different activation keyword in PowerToys Settings, then PowerToys Run,
  then Plugins, then Soos. Or turn off the built-in Calculator plugin.
- *Currency results say `no rates yet`:* converting between currencies
  needs rates, which couldn't be fetched and aren't saved yet -- open the
  app once while online.
- *A converter from the app isn't recognized:* open and close the app's
  converters window (`↔`) so it saves them for `soos-cli`.
