use crate::db::{Db, Item};
use crate::hotkey;
use clipboard_rs::{Clipboard, ClipboardContent};
use enigo::{Direction, Enigo, Keyboard, Key, Settings};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;

#[tauri::command]
pub fn search_items(db: State<'_, Arc<Mutex<Db>>>, query: String) -> Vec<Item> {
    db.lock()
        .map(|db| db.search(&query))
        .unwrap_or_default()
}

#[tauri::command]
pub fn toggle_pin(db: State<'_, Arc<Mutex<Db>>>, id: i64) -> bool {
    db.lock().map(|db| db.toggle_pin(id)).unwrap_or(false)
}

#[tauri::command]
pub fn toggle_lock(db: State<'_, Arc<Mutex<Db>>>, id: i64) -> bool {
    db.lock().map(|db| db.toggle_lock(id)).unwrap_or(false)
}

/// Saves the item as a permanent snippet, returning its assigned hotkey slot (0 = none free).
#[tauri::command]
pub fn save_item(db: State<'_, Arc<Mutex<Db>>>, id: i64) -> i64 {
    db.lock().map(|db| db.save_item(id)).unwrap_or(0)
}

#[tauri::command]
pub fn unsave_item(db: State<'_, Arc<Mutex<Db>>>, id: i64) {
    if let Ok(db) = db.lock() {
        db.unsave_item(id);
    }
}

/// Deletes an item. Returns false if it is locked (protected from deletion).
#[tauri::command]
pub fn delete_item(db: State<'_, Arc<Mutex<Db>>>, id: i64) -> bool {
    db.lock().map(|db| db.delete(id)).unwrap_or(false)
}

#[tauri::command]
pub fn hide_overlay(app: tauri::AppHandle) {
    hotkey::hide_overlay(&app);
}

/// Copies the given content to the clipboard and simulates Ctrl+V into the
/// foreground application. `plain` strips any formatting.
pub fn do_paste(text: String, html: Option<String>, rtf: Option<String>, plain: bool) -> Result<(), String> {
    let ctx = clipboard_rs::ClipboardContext::new().map_err(|e| format!("clipboard: {e}"))?;
    if plain || (html.is_none() && rtf.is_none()) {
        ctx.set(vec![ClipboardContent::Text(text)])
            .map_err(|e| format!("clipboard set: {e}"))?;
    } else {
        let mut contents = vec![ClipboardContent::Text(text)];
        if let Some(html) = html {
            contents.push(ClipboardContent::Html(html));
        }
        if let Some(rtf) = rtf {
            contents.push(ClipboardContent::Rtf(rtf));
        }
        ctx.set(contents).map_err(|e| format!("clipboard set: {e}"))?;
    }

    std::thread::sleep(Duration::from_millis(80));

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("enigo: {e}"))?;
    enigo.key(Key::Control, Direction::Press).map_err(|e| format!("enigo: {e}"))?;
    enigo.key(Key::V, Direction::Click).map_err(|e| format!("enigo: {e}"))?;
    enigo.key(Key::Control, Direction::Release).map_err(|e| format!("enigo: {e}"))?;
    Ok(())
}

/// Hides the overlay, then pastes the item (with formatting) into the foreground app.
#[tauri::command]
pub fn paste_item(
    app: tauri::AppHandle,
    db: State<'_, Arc<Mutex<Db>>>,
    id: i64,
    plain: bool,
) -> Result<(), String> {
    let item = db
        .lock()
        .map_err(|_| "db locked".to_string())?
        .get(id)
        .ok_or_else(|| "item not found".to_string())?;
    let (text, html, rtf) = item;

    // Give the overlay a chance to disappear before the paste lands.
    hotkey::hide_overlay(&app);

    // Pasting makes the entry the most recent one.
    if let Ok(db) = db.lock() {
        db.bump(id);
    }

    do_paste(text, html, rtf, plain)
}