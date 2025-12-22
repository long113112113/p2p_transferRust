#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui;
use p2p_core::{AppCommand, AppEvent, run_backend};
use std::thread;
use tokio::sync::mpsc;

mod app;
mod ui;

use app::MyApp;

fn main() -> Result<(), eframe::Error> {
    // 0. Initialize logging
    tracing_subscriber::fmt::init();

    // 1. Create channels (bounded with capacity 1000 for backpressure)
    let (tx_cmd, rx_cmd) = mpsc::channel::<AppCommand>(1000);
    let (tx_event, rx_event) = mpsc::channel::<AppEvent>(1000);

    // 2. Spawn Backend thread
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async move {
            run_backend(rx_cmd, tx_event).await;
        });
    });

    // 3. Configure window options
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    // 4. Run App
    eframe::run_native(
        "LAN P2P Transfer",
        options,
        Box::new(|cc| {
            // Initialize phosphor icons font
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(MyApp::new(tx_cmd, rx_event)))
        }),
    )
}
