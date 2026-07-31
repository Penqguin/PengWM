mod app;
mod config;
mod connection;
mod macos;
mod theme;

use std::sync::mpsc;

use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    let bar_config = config::BarConfig::load();
    let theme = theme::resolve(&bar_config);
    let corner_radius = config::resolve_corner_radius(&bar_config);

    macos::set_accessory_activation_policy();

    let (msg_tx, msg_rx) = mpsc::channel();

    eframe::run_native(
        "pengwm-bar",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_decorations(false)
                .with_always_on_top()
                .with_transparent(true)
                .with_has_shadow(false)
                .with_resizable(false)
                .with_visible(bar_config.visible)
                .with_inner_size([160.0, bar_config.thickness.max(1) as f32]),
            ..Default::default()
        },
        Box::new(move |cc| {
            let egui_ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                connection::subscribe(msg_tx, egui_ctx);
            });
            Ok(Box::new(app::BarApp::new(
                msg_rx,
                bar_config,
                theme,
                corner_radius,
            )))
        }),
    )
}
