//! Embeds `soos.ico` in the Windows .exe, so Explorer and the taskbar show
//! the icon for the file itself (the window sets its own at runtime).
//! Checks `CARGO_CFG_TARGET_OS`, since `cfg!` in a build script describes
//! the host.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/icons/soos.ico")
            .compile()
            .expect("embed the Windows exe icon");
    }
}
