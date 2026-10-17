use crate::commands;
use crate::db::Db;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutEvent, ShortcutState};

pub const HOTKEY: &str = "Ctrl+Shift+Space";
/// Snippet slots 1-9 -> instant paste without opening the UI.
const SLOT_HOTKEYS: [&str; 9] = [
    "Ctrl+Alt+1",
    "Ctrl+Alt+2",
    "Ctrl+Alt+3",
    "Ctrl+Alt+4",
    "Ctrl+Alt+5",
    "Ctrl+Alt+6",
    "Ctrl+Alt+7",
    "Ctrl+Alt+8",
    "Ctrl+Alt+9",
];

const WINDOW_WIDTH: f64 = 520.0;

/// Centers the overlay on the monitor containing the cursor, in the upper third.
pub fn position_overlay(win: &WebviewWindow) {
    let cursor = win.cursor_position().ok();
    let monitors = win.available_monitors().unwrap_or_default();
    let target = cursor
        .and_then(|pos| {
            monitors.iter().find(|m| {
                let p = m.position();
                let s = m.size();
                pos.x >= p.x as f64
                    && pos.x < (p.x as f64 + s.width as f64)
                    && pos.y >= p.y as f64
                    && pos.y < (p.y as f64 + s.height as f64)
            })
        })
        .or_else(|| monitors.first());

    if let Some(monitor) = target {
        let pos = monitor.position();
        let size = monitor.size();
        let scale = monitor.scale_factor();
        let w = WINDOW_WIDTH * scale;
        let x = pos.x as f64 + (size.width as f64 - w) / 2.0;
        let y = pos.y as f64 + size.height as f64 * 0.22;
        let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }
}

pub fn toggle_overlay(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        position_overlay(&win);
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.emit("overlay-shown", ());
    }
}

pub fn hide_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

pub fn register(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    app.global_shortcut()
        .on_shortcut(HOTKEY, |app, _shortcut, event: ShortcutEvent| {
            if event.state == ShortcutState::Pressed {
                toggle_overlay(app);
            }
        })?;

    // One global shortcut per snippet slot -> paste immediately, no UI.
    for (slot, hotkey) in SLOT_HOTKEYS.iter().enumerate() {
        app.global_shortcut().on_shortcut(*hotkey, move |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                let item = app
                    .state::<Arc<Mutex<Db>>>()
                    .lock()
                    .ok()
                    .and_then(|db| db.get_by_slot(slot as i64 + 1));
                if let Some((text, html, rtf)) = item {
                    let _ = commands::do_paste(text, html, rtf, false);
                }
            }
        })?;
    }
    Ok(())
}