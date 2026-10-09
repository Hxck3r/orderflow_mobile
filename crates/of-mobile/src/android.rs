//! The Android entry point and what winit 0.30 does not carry there.
//!
//! `android_main` is called by android-activity's GameActivity glue on a
//! thread of its own, once per activity instance. The Java `MainActivity`
//! (android/app/src/main/java/com/orderflow/mobile/MainActivity.java) keeps
//! one activity alive for the process (back moves it to the background,
//! rotation does not recreate it, a real destroy ends the process) and hands
//! over through JNI what winit drops: the system-bar and cutout insets and a
//! hovering S Pen. The soft keyboard's text is read from GameActivity's own
//! buffer.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use android_activity::AndroidApp;
use android_activity::input::{TextInputState, TextSpan};
use eframe::egui;

use crate::diag::Screen;
use crate::shell::{Hover, MobileApp, Platform};
use crate::{WINDOW_TITLE, logger, paths, settings};

/// How many times `android_main` ran in this process.
static LAUNCHES: AtomicU32 = AtomicU32::new(0);

/// Insets from `MainActivity.nativeInsets`, physical pixels.
static INSETS: [AtomicI32; 4] = [const { AtomicI32::new(0) }; 4];
static INSETS_KNOWN: AtomicBool = AtomicBool::new(false);
/// The latest hover from `MainActivity.nativeHover`; the last one wins, so a
/// pen that leaves right after a move is gone, not stuck at the move.
static HOVER: Mutex<Option<Hover>> = Mutex::new(None);
/// Wakes the interface when Java hands something over between frames.
static WAKER: OnceLock<Mutex<Option<egui::Context>>> = OnceLock::new();

fn wake() {
    if let Some(ctx) = WAKER.get().and_then(|w| w.lock().ok()).and_then(|ctx| ctx.clone()) {
        ctx.request_repaint();
    }
}

/// `MainActivity.nativeInsets(left, top, right, bottom)`: system bars, the
/// camera cutout and the soft keyboard as the window currently has them,
/// physical pixels.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_orderflow_mobile_MainActivity_nativeInsets(
    _env: *mut std::ffi::c_void,
    _this: *mut std::ffi::c_void,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) {
    for (slot, v) in INSETS.iter().zip([left, top, right, bottom]) {
        slot.store(v, Ordering::Relaxed);
    }
    INSETS_KNOWN.store(true, Ordering::Release);
    wake();
}

/// `MainActivity.nativeHover(gone, x, y)`: an S Pen over the screen without
/// touching it, window pixels; `gone` when it leaves.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_orderflow_mobile_MainActivity_nativeHover(_env: *mut std::ffi::c_void, _this: *mut std::ffi::c_void, gone: u8, x: f32, y: f32) {
    if let Ok(mut hover) = HOVER.lock() {
        *hover = Some(if gone != 0 { Hover::Gone } else { Hover::At(x, y) });
    }
    wake();
}

struct AndroidPlatform {
    app: AndroidApp,
}

impl Platform for AndroidPlatform {
    fn screen(&self) -> Option<Screen> {
        let window = self.app.native_window()?;
        let insets_px = if INSETS_KNOWN.load(Ordering::Acquire) { INSETS.each_ref().map(|v| v.load(Ordering::Relaxed)) } else { [0; 4] };
        Some(Screen { window_px: [window.width().max(0) as u32, window.height().max(0) as u32], insets_px })
    }

    fn take_hover(&self) -> Option<Hover> {
        HOVER.lock().ok().and_then(|mut h| h.take())
    }

    fn keyboard_text(&self) -> Option<String> {
        Some(self.app.text_input_state().text)
    }

    fn reset_keyboard(&self) {
        self.app.set_text_input_state(TextInputState { text: String::new(), selection: TextSpan { start: 0, end: 0 }, compose_region: None });
    }

    fn set_waker(&self, ctx: &egui::Context) {
        if let Ok(mut slot) = WAKER.get_or_init(|| Mutex::new(None)).lock() {
            *slot = Some(ctx.clone());
        }
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let launch = LAUNCHES.fetch_add(1, Ordering::SeqCst) + 1;
    if launch == 1 {
        // Settings in the app's private folder; data and logs in the external
        // app folder (Android/data/<package>/files), reachable over USB, so
        // history recorded on the PC can be copied in (plan §6).
        let internal = app.internal_data_path();
        let external = app.external_data_path().or_else(|| internal.clone());
        // Without folders from the system, of_storage's fallbacks apply.
        if let (Some(config), Some(local)) = (internal, external) {
            paths::init(config.join("config"), local);
        }
        logger::init();
    }
    log::info!("of-mobile {} starting (launch {launch} in this process)", env!("CARGO_PKG_VERSION"));
    if launch > 1 {
        // winit 0.30 creates one event loop per process, ever: a recreated
        // activity cannot get a second one. MainActivity keeps its one
        // activity alive and ends the process on a real destroy, so this is
        // a safety net: leave, and the next launch starts a fresh process.
        log::warn!("the activity was recreated in a live process: restarting the process");
        log::logger().flush();
        std::process::exit(0);
    }

    let settings = crate::shell::load_settings();
    let mut options = eframe::NativeOptions { android_app: Some(app.clone()), renderer: eframe::Renderer::Wgpu, ..Default::default() };
    // The graphics API from the settings, as the PC version's `main.rs` maps
    // it; `Auto` keeps egui-wgpu's choice (Vulkan first on Android).
    if let Some(backends) = backends(settings.performance.graphics) {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends = backends;
        options.wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(setup);
    }

    let platform: Box<dyn Platform> = Box::new(AndroidPlatform { app });
    let result = eframe::run_native(WINDOW_TITLE, options, Box::new(move |cc| Ok(Box::new(MobileApp::new(cc, settings, Some(platform))))));
    match result {
        Ok(()) => log::info!("the interface stopped"),
        Err(err) => log::error!("the interface failed: {err}"),
    }
    log::logger().flush();
}

fn backends(graphics: settings::GraphicsBackend) -> Option<eframe::wgpu::Backends> {
    match graphics {
        settings::GraphicsBackend::Vulkan => Some(eframe::wgpu::Backends::VULKAN),
        settings::GraphicsBackend::OpenGl => Some(eframe::wgpu::Backends::GL),
        settings::GraphicsBackend::Dx12 | settings::GraphicsBackend::Auto => None,
    }
}
