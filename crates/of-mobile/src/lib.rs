//! OrderFlow Terminal for Android.
//!
//! The PC version (git submodule `upstream/orderflow`, never modified) is
//! compiled here as it is: its engine crates are dependencies, and the modules
//! of its `of-app` binary are this crate's modules, read from the submodule
//! through `#[path]`. Three of them touch the desktop and are replaced by our
//! own files with the same functions: `paths`, `desktop`, `sound`.

// The PC version's modules, unchanged. Whatever only its desktop window uses
// is dead code here.
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/alerts.rs"]
mod alerts;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/app.rs"]
mod app;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/book/mod.rs"]
mod book;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/chart/mod.rs"]
mod chart;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/demo/mod.rs"]
mod demo;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/dom/mod.rs"]
mod dom;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/drawings.rs"]
mod drawings;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/format.rs"]
mod format;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/logger.rs"]
mod logger;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/render/mod.rs"]
mod render;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/replay.rs"]
mod replay;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/settings/mod.rs"]
mod settings;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/traded.rs"]
mod traded;
#[allow(dead_code)]
#[path = "../../../upstream/orderflow/crates/of-app/src/window_frame.rs"]
mod window_frame;

// The mobile shell.
#[cfg(target_os = "android")]
mod android;
mod diag;
mod gestures;
mod ime;
mod shell;

// Replacements for the PC version's desktop modules.
mod desktop;
mod paths;
mod sound;

/// The PC version's `main.rs` defines this at its crate root.
pub const WINDOW_TITLE: &str = "OrderFlow Terminal";

/// Runs the phone shell in a desktop window of the phone's size, for checks
/// on a development machine; `dir` takes the settings, data and logs.
#[cfg(not(target_os = "android"))]
pub fn run_desktop(dir: std::path::PathBuf, size_points: [f32; 2]) -> eframe::Result {
    paths::init(dir.join("config"), dir.join("local"));
    logger::init();
    log::info!("of-mobile {} on the desktop", env!("CARGO_PKG_VERSION"));
    let settings = shell::load_settings();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title(WINDOW_TITLE).with_inner_size(size_points),
        renderer: eframe::Renderer::Wgpu,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(WINDOW_TITLE, options, Box::new(move |cc| Ok(Box::new(shell::MobileApp::new(cc, settings, None)))))
}
