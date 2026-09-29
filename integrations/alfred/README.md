# Soos for Alfred

A Script Filter that hands what you type to `soos-cli --alfred` and shows the
answer exactly as the Soos app would. Enter copies the plain value (no
thousands separators), like clicking a result in the app.

Step-by-step setup and troubleshooting: see **Extra guides → Alfred
workflow** in the [main README](../../README.md#extra-guides).

## Requires

- Alfred with the Powerpack (workflows are a Powerpack feature).
- `soos-cli`. The workflow uses the copy inside `/Applications/Soos.app`
  (or `~/Applications/Soos.app`) that the `.dmg` installs, or one in
  `/opt/homebrew/bin`, `/usr/local/bin` or on `PATH`. Alfred runs scripts
  with a minimal `PATH`, which is why those locations are checked directly.

## Build

```
./build.sh
```

Produces `soos.alfredworkflow`. Double-click it to import into Alfred.

## Use

Type `=` followed by an expression, e.g. `=20 inches in cm`.

## Notes

`soos-cli --alfred` always exits 0: an error shows as the result row (short
label, full message underneath) instead of Alfred's error sheet -- see
`crates/soos-cli/src/main.rs`.
