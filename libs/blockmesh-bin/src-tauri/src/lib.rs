use std::env;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};

use clap::Parser;
#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;
use tauri::Manager;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_autostart::MacosLauncher;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use block_mesh_common::app_config::AppConfig;
use block_mesh_common::cli::CliArgs;
use block_mesh_common::constants::DeviceType;
use logger_general::tracing::setup_tracing;

use crate::background::channel_receiver;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::commands::open_main_window;
use crate::commands::{
    check_token, get_app_config, get_home_url, get_ore_status, get_task_status, login, logout,
    register, set_app_config, toggle_miner,
};
use crate::run_events::on_run_events;
use crate::system_tray::set_dock_visible;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::system_tray::setup_tray;
use crate::tauri_state::{AppState, ChannelMessage};
use crate::tauri_storage::setup_storage;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::windows_events::on_window_event;

mod background;
mod blockmesh;
mod commands;
mod ore;
mod run_events;
mod system_tray;
mod tauri_state;
mod tauri_storage;
mod windows_events;
pub static SYSTEM: OnceLock<Mutex<sysinfo::System>> = OnceLock::new();

pub static CHANNEL_MSG_TX: OnceLock<broadcast::Sender<ChannelMessage>> = OnceLock::new();

pub fn run() -> anyhow::Result<ExitCode> {
    let (incoming_tx, incoming_rx) = broadcast::channel::<ChannelMessage>(2);
    let args = CliArgs::parse();
    let mut config = if let Some(command) = args.command {
        AppConfig::from(command)
    } else {
        AppConfig::default()
    };
    config.device_id = config.device_id.or(Some(Uuid::new_v4()));
    setup_tracing(config.device_id.unwrap(), DeviceType::Desktop);

    let _ = CHANNEL_MSG_TX.set(incoming_tx.clone());

    let app_state = Arc::new(Mutex::new(AppState {
        config,
        tx: incoming_tx,
        rx: incoming_rx.resubscribe(),
        ore_pid: None,
        node_handle: None,
        uptime_handle: None,
        task_puller: None,
    }));

    tauri::async_runtime::set(tokio::runtime::Handle::current());
    tokio::spawn(channel_receiver(incoming_rx, app_state.clone()));
    let app_state = app_state.clone();
    let app = tauri::Builder::default();

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let app = app.plugin(tauri_plugin_autostart::init(
        MacosLauncher::LaunchAgent,
        Some(vec!["--minimized"]),
    ));
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let app = app.plugin(tauri_plugin_single_instance::init(
        move |app, _argv, _cwd| {
            open_main_window(app).unwrap();
        },
    ));
    let app = app.manage(app_state.clone()).setup(move |app| {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        setup_tray(app);
        #[cfg(desktop)]
        {
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
        }
        #[cfg(target_os = "macos")]
        {
            app.set_activation_policy(ActivationPolicy::Accessory);
        }
        let app_handle = app.app_handle();
        tauri::async_runtime::spawn(setup_storage(app_handle.clone()));
        let app_handle = app.app_handle();
        if args.minimized {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let window = app_handle.get_webview_window("main").unwrap();
                window.hide().unwrap();
            }
            set_dock_visible(false);
        } else {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            open_main_window(app.app_handle()).unwrap();
        }
        Ok(())
    });
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let app = app.on_window_event(on_window_event);
    app.invoke_handler(tauri::generate_handler![
        get_app_config,
        set_app_config,
        get_task_status,
        toggle_miner,
        get_ore_status,
        login,
        register,
        check_token,
        logout,
        get_home_url
    ])
    .build(tauri::generate_context!())
    .expect("error while running tauri application")
    .run(on_run_events);
    Ok(ExitCode::SUCCESS)
}
