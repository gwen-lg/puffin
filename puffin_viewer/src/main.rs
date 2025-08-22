//! Remote puffin viewer, connecting to a [`puffin_http::PuffinServer`].

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), eframe::Error> {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    use puffin::FrameView;
    use puffin_viewer::{Arguments, PuffinViewer, Source};

    let opt: Arguments = argh::from_env();

    puffin::set_scopes_on(true); // so we can profile ourselves

    let (opt_url, opt_path) = opt.get();
    let source = if let Some(path) = opt_path {
        let mut file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(err) => {
                log::error!("Failed to open {:?}: {err:#}", path.display());
                std::process::exit(1);
            }
        };

        match FrameView::read(&mut file) {
            Ok(frame_view) => Source::FilePath(path, frame_view),
            Err(err) => {
                log::error!("Failed to load {:?}: {err:#}", path.display());
                std::process::exit(1);
            }
        }
    } else {
        Source::Http(puffin_http::Client::new(opt_url))
    };

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../icon.png")).unwrap();
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_app_id("puffin_viewer")
            .with_drag_and_drop(true)
            .with_inner_size([1400.0, 1000.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "puffin viewer",
        native_options,
        Box::new(|cc| Ok(Box::new(PuffinViewer::new(source, cc.storage)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
