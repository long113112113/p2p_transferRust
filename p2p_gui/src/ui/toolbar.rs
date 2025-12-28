use crate::app::AppUIState;
use eframe::egui;
use egui_phosphor::regular::{DESKTOP_TOWER, FOLDER_SIMPLE, GLOBE, QR_CODE};

pub fn show(ctx: &egui::Context, state: &mut AppUIState) {
    egui::SidePanel::right("right_toolbar")
        .resizable(false)
        .default_width(150.0)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(8.0);

                // Devices button
                if ui
                    .selectable_label(state.show_devices, format!("{} Devices", DESKTOP_TOWER))
                    .clicked()
                {
                    state.show_devices = !state.show_devices;
                }

                // WAN Connect button
                if ui
                    .selectable_label(state.show_wan_connect, format!("{} WAN", GLOBE))
                    .clicked()
                {
                    state.show_wan_connect = !state.show_wan_connect;
                }

                // Transfers button
                if ui
                    .selectable_label(state.show_files, format!("{} Files", FOLDER_SIMPLE))
                    .clicked()
                {
                    state.show_files = !state.show_files;
                }
                //QR code button
                if ui
                    .selectable_label(state.show_qrcode, format!("{} QR Code", QR_CODE))
                    .clicked()
                {
                    state.show_qrcode = !state.show_qrcode;
                }
            });
        });
}
