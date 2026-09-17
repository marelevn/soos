# Soos for PowerToys Run

A PowerToys Run plugin that hands your query to `soos --json` and shows the
answer. Enter copies it to the clipboard.

## Requires

The `soos` binary on `PATH`, or set the `SOOS_EXE` environment variable to
its full path. Build it with `cargo build --release -p soos-cli`.

## Build

```
dotnet build -c Release -p:Platform=x64
```

(swap `x64` for `ARM64` on Arm devices).

## Install

1. Close PowerToys.
2. Copy `bin/x64/Release/net9.0-windows10.0.26100.0/` to
   `%LOCALAPPDATA%\Microsoft\PowerToys\PowerToys Run\Plugins\Soos\`.
3. Reopen PowerToys.

## Use

`Alt+Space`, then `=` followed by an expression, e.g. `=20 inches in cm`.
Enter copies the result.
