use crate::app::AppUIState;
use eframe::egui;
use egui_phosphor::regular::{DESKTOP_TOWER, FOLDER_SIMPLE};

pub fn show(ctx: &egui::Context, state: &mut AppUIState) {
    egui::SidePanel::right("right_toolbar")
        .resizable(false)
        .default_width(150.0)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(8.0);

                // Devices button with phosphor icon
                if ui
                    .selectable_label(state.show_devices, format!("{} Devices", DESKTOP_TOWER))
                    .clicked()
                {
                    state.show_devices = !state.show_devices;
                }

                // Transfers button with phosphor icon
                if ui
                    .selectable_label(state.show_files, format!("{} Files", FOLDER_SIMPLE))
                    .clicked()
                {
                    state.show_files = !state.show_files;
                }
            });
        });
}
