//! The phone shell in a desktop window of the phone's size:
//! `cargo run -p of-mobile --example desktop -- <dir> [width height]`
//! (points; the S25 Ultra in landscape is about 832 × 384 at Samsung's default zoom).

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let dir = args.next().map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("of-mobile-desktop"));
    let w = args.next().and_then(|v| v.parse().ok()).unwrap_or(832.0);
    let h = args.next().and_then(|v| v.parse().ok()).unwrap_or(384.0);
    of_mobile::run_desktop(dir, [w, h])
}

/// The phone runs the library itself; this example is for desktops only.
#[cfg(target_os = "android")]
fn main() {}
