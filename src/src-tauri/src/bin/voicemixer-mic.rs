#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[path = "mic_router/actions.rs"]
mod actions;
#[path = "mic_router/config.rs"]
mod config;
#[path = "mic_router/delay.rs"]
mod delay;
#[path = "mic_router/engine.rs"]
mod engine;
#[cfg(windows)]
#[path = "mic_router/hotkeys.rs"]
mod hotkeys;
#[cfg(windows)]
#[path = "mic_router/settings_ui.rs"]
mod settings_ui;
#[path = "mic_router/telemetry.rs"]
mod telemetry;
#[cfg(windows)]
#[path = "mic_router/ui.rs"]
mod ui;
#[path = "mic_router/cli.rs"]
mod cli;
#[path = "mic_router/resident.rs"]
mod resident;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("{error}");
        #[cfg(windows)]
        if !std::env::args()
            .any(|arg| matches!(arg.as_str(), "--check" | "--init" | "--status" | "--stop"))
        {
            ui::error(&error);
        }
        std::process::exit(1);
    }
}
