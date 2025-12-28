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

    // 1.5. Initialize WAN Service
    let wan_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let config_dir = p2p_core::config::get_config_dir().unwrap_or(std::path::PathBuf::from("."));
    let download_dir = p2p_core::config::get_download_dir();
    let identity_manager = p2p_core::identity::IdentityManager::new(config_dir);

    let wan_event_tx = tx_event.clone();
    let wan_service = wan_runtime.block_on(async {
        let secret_key = identity_manager
            .load_or_generate()
            .await
            .expect("Failed to load identity");
        p2p_wan::ConnectionListener::new(secret_key, download_dir, wan_event_tx)
            .await
            .expect("Failed to create WAN listener")
    });
    let wan_service = std::sync::Arc::new(wan_service);

    // Spawn listener loop
    let ws_clone = wan_service.clone();
    wan_runtime.spawn(async move {
        if let Err(e) = ws_clone.listen().await {
            tracing::error!("WAN Listener error: {}", e);
        }
    });

    // 2. Spawn Backend thread
    let backend_tx_event = tx_event.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async move {
            run_backend(rx_cmd, backend_tx_event).await;
        });
    });

    // 3. Configure window options
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    // 4. Run App
    let wan_rt_handle = wan_runtime.handle().clone();
    eframe::run_native(
        "LAN P2P Transfer",
        options,
        Box::new(move |cc| {
            // Initialize phosphor icons font
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(MyApp::new(
                tx_cmd,
                rx_event,
                tx_event,
                wan_service,
                wan_rt_handle,
            )))
        }),
    )
}
