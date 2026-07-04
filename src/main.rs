#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod app;
mod launch;
mod password;
mod proxy;
mod register;
mod settings;
mod steam;
mod storage;
mod web;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Steam Account Manager"),
        ..Default::default()
    };

    eframe::run_native(
        "Steam Account Manager",
        options,
        Box::new(|cc| Ok(Box::new(app::SteamAccountManagerApp::new(cc)))),
    )
}
