use crate::db::Db;
use clipboard_rs::{Clipboard, ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

struct ClipHandler {
    app: tauri::AppHandle,
    db: Arc<Mutex<Db>>,
    last: String,
}

impl ClipboardHandler for ClipHandler {
    fn on_clipboard_change(&mut self) {
        let ctx = match clipboard_rs::ClipboardContext::new() {
            Ok(c) => c,
            Err(_) => return,
        };
        let text = match ctx.get_text() {
            Ok(t) if !t.is_empty() => t,
            _ => return,
        };
        if text == self.last {
            return;
        }
        self.last = text.clone();

        let html = ctx.has(clipboard_rs::ContentFormat::Html).then(|| ctx.get_html().ok()).flatten();
        let rtf = ctx.has(clipboard_rs::ContentFormat::Rtf).then(|| ctx.get_rich_text().ok()).flatten();

        let id = {
            let db = match self.db.lock() {
                Ok(d) => d,
                Err(_) => return,
            };
            db.insert(&text, html, rtf)
        };
        if let Some(id) = id {
            let item = {
                let db = match self.db.lock() {
                    Ok(d) => d,
                    Err(_) => return,
                };
                db.search("").into_iter().find(|i| i.id == id)
            };
            if let Some(item) = item {
                let _ = self.app.emit("clipboard-changed", item);
            }
        }
    }
}

/// Watches the system clipboard on a background thread and persists new text clips.
pub fn spawn(app: tauri::AppHandle) {
    let db = app.state::<Arc<Mutex<Db>>>().inner().clone();
    std::thread::spawn(move || {
        let mut watcher = match ClipboardWatcherContext::new() {
            Ok(w) => w,
            Err(_) => return,
        };
        watcher.add_handler(ClipHandler {
            app,
            db,
            last: String::new(),
        });
        watcher.start_watch();
    });
}