# Soos for PowerToys Run

A PowerToys Run plugin that hands your query to `soos-cli --json` and shows
the answer exactly as the Soos app would. Enter copies the plain value (no
thousands separators), like clicking a result in the app.

Step-by-step setup and troubleshooting: see **Extra guides → PowerToys Run
plugin** in the [main README](../../README.md#extra-guides).

## Requires

- `soos-cli.exe` on `PATH`, or the `SOOS_EXE` environment variable set to
  its full path. It ships next to `soos-app.exe` in the Windows release zip.
- The .NET 9 SDK, to build the plugin.

## Build

```
dotnet build -c Release -p:Platform=x64
```

(swap `x64` for `ARM64` on Arm devices).

## Install

1. Close PowerToys.
2. Copy the contents of `bin/x64/Release/net9.0-windows10.0.26100.0/` to
   `%LOCALAPPDATA%\Microsoft\PowerToys\PowerToys Run\Plugins\Soos\`.
3. Reopen PowerToys.

## Use

`Alt+Space`, then `=` followed by an expression, e.g. `=20 inches in cm`.
