//! Embeds `soos.ico` as a real PE resource on Windows, so Explorer, the
//! taskbar and Alt-Tab show the app icon for the raw .exe file -- the
//! *window* itself already gets its icon at runtime via `with_icon()` in
//! `main.rs`, this is the one gap that leaves. No-op on macOS/Linux.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/icons/soos.ico")
            .compile()
            .expect("embed the Windows exe icon");
    }
}
