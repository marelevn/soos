# Soos for Alfred

A Script Filter that hands expressions to `soos-cli --alfred` and copies the
answer to your clipboard.

## Requires

The `soos-cli` binary on `PATH` (Alfred's script environment has a minimal PATH,
so `/opt/homebrew/bin` and `/usr/local/bin` are also checked directly). Build
it with `cargo build --release -p soos-cli` and put the resulting binary
somewhere on PATH, or `cargo install --path crates/soos-cli`.

## Build

```
./build.sh
```

Produces `soos.alfredworkflow`. Double-click it to import into Alfred.

## Use

Type `=` followed by an expression, e.g. `=20 inches in cm`. Enter copies the
result.

## Notes

`soos-cli --alfred` always exits 0 (errors show as the result row instead of
Alfred's error sheet) -- see `crates/soos-cli/src/main.rs`.
