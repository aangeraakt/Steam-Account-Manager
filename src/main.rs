#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod app;
mod launch;
mod steam;
mod storage;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 720.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("Steam Account Manager"),
        ..Default::default()
    };

    eframe::run_native(
        "Steam Account Manager",
        options,
        Box::new(|cc| Ok(Box::new(app::SteamAccountManagerApp::new(cc)))),
    )
}
