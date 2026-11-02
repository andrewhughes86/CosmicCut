//! L5 desktop UI on egui + wgpu, laid out like CosmicDMS: a ribbon over a dockable workspace.
//! Swappable: everything below talks only to the engine.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![allow(non_snake_case)]

mod app;
mod automation;
mod cut_panel;
mod dock;
pub mod icon_svg;
mod library_dialog;
mod machine_panel;
pub mod navigation;
mod op_dialog;
mod panes;
mod param_form;
mod post_panel;
mod ribbon;
mod stock_panel;
pub mod theme;
mod tools;
pub mod ui_kit;
mod viewport;

pub use CosmicCut_engine::Session;
pub use app::{CosmicApp, LaunchOptions};
pub use automation::Automation;
pub use panes::parse_console_line;

/// A command for the app to run after this frame: id and JSON parameters.
pub type Request = (String, serde_json::Value);

pub const APP_ID: &str = "CosmicCut";

/// The application icon's master (`assets/icons/app/cosmiccut.svg`, a 256-unit square).
pub const APP_ICON_SVG: &str = include_str!("../../../assets/icons/app/cosmiccut.svg");

/// The application icon for the window and the taskbar, rendered at `px` pixels; None if it
/// cannot be drawn (the window then has the system's default).
pub fn app_icon(px: u32) -> Option<egui::IconData> {
    use resvg::{tiny_skia, usvg};
    let px = px.clamp(16, 1024);
    std::panic::catch_unwind(|| {
        let tree = usvg::Tree::from_str(APP_ICON_SVG, &usvg::Options::default()).ok()?;
        let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
        let scale = px as f32 / 256.0;
        resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        // The window system wants straight (not premultiplied) alpha.
        let rgba = pixmap.pixels().iter().flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        });
        Some(egui::IconData { rgba: rgba.collect(), width: px, height: px })
    })
    .ok()
    .flatten()
}

/// Native window configuration: wgpu renderer, sensible default and minimum sizes.
#[cfg(not(target_arch = "wasm32"))]
pub fn native_options() -> eframe::NativeOptions {
    let viewport = egui::ViewportBuilder::default();
    let viewport = match app_icon(256) {
        Some(icon) => viewport.with_icon(std::sync::Arc::new(icon)),
        None => viewport,
    };
    eframe::NativeOptions {
        viewport: viewport
            .with_title("CosmicCut")
            // The ribbon's tab strip is the title bar, with its own window controls.
            .with_decorations(false)
            .with_app_id(APP_ID)
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([960.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        // The viewport renders offscreen with its own depth and MSAA targets.
        depth_buffer: 0,
        multisampling: 0,
        centered: true,
        ..Default::default()
    }
}

/// Opens the desktop window and runs until it closes.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(session: Session, options: LaunchOptions) -> eframe::Result {
    eframe::run_native(APP_ID, native_options(), Box::new(move |cc| Ok(Box::new(CosmicApp::new(cc, session, options)))))
}

/// Runs in a browser, drawing into `canvas`. Settings and the library are kept in the
/// browser's local storage.
#[cfg(target_arch = "wasm32")]
pub async fn run_web(
    canvas: web_sys::HtmlCanvasElement,
    session: Session,
    options: LaunchOptions,
) -> Result<(), wasm_bindgen::JsValue> {
    let web_options = eframe::WebOptions { depth_buffer: 0, ..Default::default() };
    eframe::WebRunner::new()
        .start(canvas, web_options, Box::new(move |cc| Ok(Box::new(CosmicApp::new(cc, session, options)))))
        .await
}

/// Hands files the commands wrote to the browser as downloads.
#[cfg(target_arch = "wasm32")]
pub(crate) fn deliver_downloads() {
    use wasm_bindgen::JsCast as _;
    for (name, bytes) in CosmicCut_io::files::take_downloads() {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
        let props = web_sys::BlobPropertyBag::new();
        props.set_type("application/octet-stream");
        let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &props) else { continue };
        let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { continue };
        if let Some(a) = document.create_element("a").ok().and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok())
        {
            a.set_href(&url);
            a.set_download(&name);
            a.click();
        }
        let _ = web_sys::Url::revoke_object_url(&url);
    }
}

#[cfg(test)]
mod app_icon_tests {
    #[test]
    fn the_app_icon_renders() {
        let icon = super::app_icon(64);
        assert!(icon.as_ref().is_some_and(|i| i.width == 64 && i.rgba.len() == 64 * 64 * 4));
        // Opaque in the middle, clear in the corners (a rounded tile).
        let px = |x: usize, y: usize| icon.as_ref().map_or(0, |i| i.rgba[(y * 64 + x) * 4 + 3]);
        assert_eq!(px(32, 32), 255);
        assert_eq!(px(0, 0), 0);
    }
}
