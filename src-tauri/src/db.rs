use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;
use tauri::Manager;

const MAX_ITEMS: i64 = 500;
const MAX_CONTENT_BYTES: usize = 1_000_000;
const PREVIEW_CHARS: i64 = 300;
pub const MAX_SLOT: i64 = 9;

const COLUMNS: &str = "id, substr(content, 1, ?1), CASE WHEN html IS NOT NULL THEN 'html' WHEN rtf IS NOT NULL THEN 'rtf' ELSE 'text' END AS kind, pinned, locked, saved, slot, created_at, length(content)";

#[derive(Clone, Serialize)]
pub struct Item {
    pub id: i64,
    pub preview: String,
    pub kind: &'static str,
    pub pinned: bool,
    pub locked: bool,
    pub saved: bool,
    pub slot: i64,
    pub created_at: i64,
    pub chars: usize,
}

pub struct Db {
    conn: Connection,
    use_fts: bool,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Db {
    pub fn open(app: &tauri::AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let dir: PathBuf = app.path().app_data_dir()?;
        std::fs::create_dir_all(&dir)?;
        Self::open_dir(dir)
    }

    pub fn open_dir(dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&dir)?;
        let conn = Connection::open(dir.join("history.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                html TEXT,
                rtf TEXT,
                pinned INTEGER NOT NULL DEFAULT 0,
                saved INTEGER NOT NULL DEFAULT 0,
                slot INTEGER NOT NULL DEFAULT 0,
                locked INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_items_created ON items(created_at DESC);",
        )?;

        // Migration for databases created before saved/slot/locked existed.
        let cols = {
            let mut names = Vec::new();
            let mut stmt = conn.prepare("PRAGMA table_info(items)")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for row in rows {
                names.push(row?);
            }
            names
        };
        for col in ["saved", "slot", "locked"] {
            if !cols.iter().any(|c| c == col) {
                conn.execute(
                    &format!(
                        "ALTER TABLE items ADD COLUMN {col} INTEGER NOT NULL DEFAULT 0",
                    ),
                    [],
                )?;
            }
        }

        let use_fts = Self::init_fts(&conn);

        Ok(Self { conn, use_fts })
    }

    fn init_fts(conn: &Connection) -> bool {
        let Ok(_) = conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS items_fts USING fts5(
                content, content='items', content_rowid='id', tokenize='unicode61');
            CREATE TRIGGER IF NOT EXISTS items_ai AFTER INSERT ON items BEGIN
                INSERT INTO items_fts(rowid, content) VALUES (new.id, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS items_ad AFTER DELETE ON items BEGIN
                INSERT INTO items_fts(items_fts, rowid, content) VALUES ('delete', old.id, old.content);
            END;
            CREATE TRIGGER IF NOT EXISTS items_au AFTER UPDATE OF content ON items BEGIN
                INSERT INTO items_fts(items_fts, rowid, content) VALUES ('delete', old.id, old.content);
                INSERT INTO items_fts(rowid, content) VALUES (new.id, new.content);
            END;",
        ) else {
            return false;
        };
        // If the items table predates the FTS triggers, rebuild the index once.
        if let Ok(n) = conn.query_row("SELECT COUNT(*) FROM items_fts", [], |r| r.get::<_, i64>(0)) {
            let items: i64 = conn
                .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
                .unwrap_or(0);
            if n != items {
                let _ = conn.execute("INSERT INTO items_fts(items_fts) VALUES('rebuild')", []);
            }
        }
        true
    }

    fn row_to_item(r: &rusqlite::Row) -> rusqlite::Result<Item> {
        let kind: String = r.get(2)?;
        Ok(Item {
            id: r.get(0)?,
            preview: r.get(1)?,
            kind: match kind.as_str() {
                "html" => "html",
                "rtf" => "rtf",
                _ => "text",
            },
            pinned: r.get::<_, i64>(3)? != 0,
            locked: r.get::<_, i64>(4)? != 0,
            saved: r.get::<_, i64>(5)? != 0,
            slot: r.get(6)?,
            created_at: r.get(7)?,
            chars: r.get::<_, i64>(8)? as usize,
        })
    }

    /// Inserts a new clipboard entry, or bumps an existing duplicate to the top.
    pub fn insert(&self, text: &str, html: Option<String>, rtf: Option<String>) -> Option<i64> {
        if text.is_empty() || text.len() > MAX_CONTENT_BYTES {
            return None;
        }
        let now = now_ms();
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM items WHERE content = ?1 LIMIT 1", [text], |r| r.get(0))
            .ok();
        let id = if let Some(id) = existing {
            let _ = self
                .conn
                .execute("UPDATE items SET created_at = ?1 WHERE id = ?2", params![now, id]);
            id
        } else {
            let _ = self.conn.execute(
                "INSERT INTO items (content, html, rtf, pinned, saved, slot, locked, created_at)
                 VALUES (?1, ?2, ?3, 0, 0, 0, 0, ?4)",
                params![text, html, rtf, now],
            );
            self.conn.last_insert_rowid()
        };
        let _ = self.conn.execute(
            "DELETE FROM items
             WHERE locked = 0 AND saved = 0 AND pinned = 0 AND id NOT IN (
                SELECT id FROM items WHERE locked = 0 AND saved = 0 AND pinned = 0
                ORDER BY created_at DESC, id DESC LIMIT ?1)",
            [MAX_ITEMS],
        );
        Some(id)
    }

    pub fn search(&self, query: &str) -> Vec<Item> {
        let q = query.trim();
        if q.is_empty() {
            return self.recent();
        }
        if self.use_fts {
            if let Some(items) = self.search_fts(q) {
                return items;
            }
        }
        self.search_like(q)
    }

    const ORDER: &'static str = "saved DESC, slot ASC, pinned DESC, created_at DESC, id DESC";

    fn recent(&self) -> Vec<Item> {
        let mut stmt = match self.conn.prepare(&format!(
            "SELECT {COLUMNS}
             FROM items ORDER BY {} LIMIT 100",
            Self::ORDER
        )) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        self.collect(&mut stmt, params![PREVIEW_CHARS])
    }

    fn search_fts(&self, q: &str) -> Option<Vec<Item>> {
        let pattern = fts_pattern(q)?;
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT i.id, substr(i.content, 1, ?1), CASE WHEN i.html IS NOT NULL THEN 'html' WHEN i.rtf IS NOT NULL THEN 'rtf' ELSE 'text' END, i.pinned, i.locked, i.saved, i.slot, i.created_at, length(i.content)
                 FROM items_fts f JOIN items i ON i.id = f.rowid
                 WHERE items_fts MATCH ?2
                 ORDER BY {} LIMIT 100",
                Self::ORDER
            ))
            .ok()?;
        Some(self.collect(&mut stmt, params![PREVIEW_CHARS, pattern]))
    }

    fn search_like(&self, q: &str) -> Vec<Item> {
        let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let like = format!("%{escaped}%");
        let mut stmt = match self.conn.prepare(&format!(
            "SELECT {COLUMNS}
             FROM items WHERE content LIKE ?2 ESCAPE '\\'
             ORDER BY {} LIMIT 100",
            Self::ORDER
        )) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        self.collect(&mut stmt, params![PREVIEW_CHARS, like])
    }

    fn collect(&self, stmt: &mut rusqlite::Statement, params: impl rusqlite::Params) -> Vec<Item> {
        match stmt.query_map(params, Self::row_to_item) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn get(&self, id: i64) -> Option<(String, Option<String>, Option<String>)> {
        self.conn
            .query_row(
                "SELECT content, html, rtf FROM items WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()
    }

    pub fn get_by_slot(&self, slot: i64) -> Option<(String, Option<String>, Option<String>)> {
        self.conn
            .query_row(
                "SELECT content, html, rtf FROM items WHERE saved = 1 AND slot = ?1 LIMIT 1",
                [slot],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()
    }

    pub fn bump(&self, id: i64) {
        let _ = self
            .conn
            .execute("UPDATE items SET created_at = ?1 WHERE id = ?2", params![now_ms(), id]);
    }

    pub fn toggle_pin(&self, id: i64) -> bool {
        let pinned: i64 = self
            .conn
            .query_row("SELECT pinned FROM items WHERE id = ?1", [id], |r| r.get(0))
            .unwrap_or(0);
        let new = 1 - pinned;
        let _ = self
            .conn
            .execute("UPDATE items SET pinned = ?1 WHERE id = ?2", params![new, id]);
        new != 0
    }

    /// Toggles the delete-protection lock. Returns the new locked state.
    pub fn toggle_lock(&self, id: i64) -> bool {
        let locked: i64 = self
            .conn
            .query_row("SELECT locked FROM items WHERE id = ?1", [id], |r| r.get(0))
            .unwrap_or(0);
        let new = 1 - locked;
        let _ = self
            .conn
            .execute("UPDATE items SET locked = ?1 WHERE id = ?2", params![new, id]);
        new != 0
    }

    /// Marks an item as a permanent snippet and assigns the lowest free hotkey slot.
    /// Returns the assigned slot, or -1 if none of 1-9 is free (item left unchanged).
    pub fn save_item(&self, id: i64) -> i64 {
        let taken: Vec<i64> = {
            let mut stmt = match self
                .conn
                .prepare("SELECT slot FROM items WHERE saved = 1 AND slot BETWEEN 1 AND ?1 ORDER BY slot")
            {
                Ok(s) => s,
                Err(_) => return -1,
            };
            let rows = stmt.query_map([MAX_SLOT], |r| r.get(0)).ok();
            match rows {
                Some(rows) => rows.filter_map(|r| r.ok()).collect(),
                None => return -1,
            }
        };
        let Some(slot) = (1..=MAX_SLOT).find(|s| !taken.contains(s)) else {
            return -1;
        };
        let _ = self.conn.execute(
            "UPDATE items SET saved = 1, slot = ?1 WHERE id = ?2",
            params![slot, id],
        );
        slot
    }

    /// Removes the snippet status (keeps the item in history).
    pub fn unsave_item(&self, id: i64) {
        let _ = self
            .conn
            .execute("UPDATE items SET saved = 0, slot = 0 WHERE id = ?1", [id]);
    }

    /// Deletes an item unless it is locked. Returns whether it was deleted.
    pub fn delete(&self, id: i64) -> bool {
        let locked: i64 = self
            .conn
            .query_row("SELECT locked FROM items WHERE id = ?1", [id], |r| r.get(0))
            .unwrap_or(1);
        if locked != 0 {
            return false;
        }
        let _ = self.conn.execute("DELETE FROM items WHERE id = ?1", [id]);
        true
    }
}

/// Converts free text into a safe FTS5 prefix-match pattern, e.g. `foo* "bar baz"*`.
fn fts_pattern(q: &str) -> Option<String> {
    let tokens: Vec<String> = q
        .split_whitespace()
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn test_db() -> Db {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("clipstack-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Db::open_dir(dir).expect("open db")
    }

    #[test]
    fn insert_search_pin_delete() {
        let db = test_db();
        let id1 = db.insert("hello world rust", None, None).unwrap();
        db.insert("clipboard manager test", None, Some("{\\rtf1 hello}".into())).unwrap();
        db.insert("maccy features", Some("<b>maccy</b>".into()), None).unwrap();
        let id3 = db.insert("maccy features", Some("<b>maccy</b>".into()), None).unwrap();

        // duplicate returns the same row
        let hits = db.search("");
        let ids: Vec<i64> = hits.iter().map(|i| i.id).collect();
        assert_eq!(hits[0].id, id3, "recent order wrong, ids={ids:?}");
        assert_eq!(hits.len(), 3);

        // FTS prefix search
        let hits = db.search("hell");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id1);

        let hits = db.search("macc");
        assert_eq!(hits[0].id, id3);
        assert_eq!(hits[0].kind, "html");

        // quote/glob chars are safe for FTS
        let dbg = db.insert("100% done", None, None).unwrap();
        let hits = db.search("% don");
        assert!(hits.iter().any(|i| i.id == dbg));

        // pin floats to top
        db.toggle_pin(id1);
        let hits = db.search("");
        assert_eq!(hits[0].id, id1);

        // delete removes it
        assert!(db.delete(id1));
        let hits = db.search("");
        assert!(!hits.iter().any(|i| i.id == id1));
    }

    #[test]
    fn caps_history_size() {
        let db = test_db();
        let pinned_id = db.insert("pinned survivor", None, None).unwrap();
        db.toggle_pin(pinned_id);
        for i in 0..600 {
            db.insert(&format!("item {i}"), None, None);
        }
        let n: i64 = db.conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 501, "pinned item plus 500 trimmed others");
        let hits = db.search("");
        assert_eq!(hits[0].id, pinned_id, "pinned survives trimming");
    }

    #[test]
    fn snippets_and_locks() {
        let db = test_db();
        let id = db.insert("api_key_secret", None, None).unwrap();

        // saving assigns the lowest free slot
        assert_eq!(db.save_item(id), 1);
        assert_eq!(db.get_by_slot(1).as_ref().map(|v| v.0.as_str()), Some("api_key_secret"));
        assert!(db.search("").iter().find(|i| i.id == id).unwrap().saved);

        // second snippet takes the next slot
        let id2 = db.insert("team email", None, None).unwrap();
        assert_eq!(db.save_item(id2), 2);

        // unsaving frees the slot
        db.unsave_item(id2);
        assert!(db.get_by_slot(2).is_none());
        let id3 = db.insert("another snippet", None, None).unwrap();
        assert_eq!(db.save_item(id3), 2, "freed slot reused");

        // snippet in slot 1 survives trimming
        for i in 0..600 {
            db.insert(&format!("item {i}"), None, None);
        }
        assert!(db.get_by_slot(1).is_some(), "snippet survives trimming");
        let n: i64 = db.conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 502, "two saved snippets plus 500 trimmed others");
    }

    #[test]
    fn locked_items_are_protected() {
        let db = test_db();
        let id = db.insert("do not delete", None, None).unwrap();
        db.toggle_lock(id);
        assert!(!db.delete(id), "locked item cannot be deleted");
        assert!(db.search("").iter().any(|i| i.id == id));

        // trimming skips the locked item
        for i in 0..600 {
            db.insert(&format!("item {i}"), None, None);
        }
        let n: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM items WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "locked survives trimming");

        // unlock then delete works
        db.toggle_lock(id);
        assert!(db.delete(id));
    }

    #[test]
    fn fts_pattern_sanitizes() {
        assert_eq!(fts_pattern("a b").unwrap(), "\"a\"* \"b\"*");
        assert_eq!(fts_pattern("jo\"e").unwrap(), "\"jo\"\"e\"*");
        assert!(fts_pattern("").is_none());
    }
}