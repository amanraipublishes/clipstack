mod commands;
mod db;
mod hotkey;
mod monitor;

use std::sync::{Arc, Mutex};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{menu::Menu, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();

            let db = db::Db::open(&handle)?;
            app.manage(Arc::new(Mutex::new(db)));
            monitor::spawn(handle.clone());
            hotkey::register(&handle)?;

            if let Some(win) = app.get_webview_window("main") {
                let blur_win = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        let _ = blur_win.hide();
                    }
                });
            }

            let show = Menu::with_items(
                app,
                &[
                    &tauri::menu::MenuItem::with_id(app, "toggle", "Show / hide", true, None::<&str>)?,
                    &tauri::menu::MenuItem::with_id(app, "quit", "Quit ClipStack", true, None::<&str>)?,
                ],
            )?;
            let tray = TrayIconBuilder::with_id("main-tray")
                .icon(
                    app.default_window_icon()
                        .ok_or_else(|| std::io::Error::other("no app icon"))?
                        .clone(),
                )
                .tooltip("ClipStack")
                .menu(&show)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => hotkey::toggle_overlay(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        hotkey::toggle_overlay(tray.app_handle());
                    }
                });
            tray.build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search_items,
            commands::toggle_pin,
            commands::toggle_lock,
            commands::save_item,
            commands::unsave_item,
            commands::delete_item,
            commands::hide_overlay,
            commands::paste_item,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
