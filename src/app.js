const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const listEl = document.getElementById("list");
const emptyEl = document.getElementById("empty");
const countEl = document.getElementById("count");
const searchEl = document.getElementById("search");
const statusEl = document.getElementById("status");

let items = [];
let query = "";
let selected = 0;

/* ---------------- icons ---------------- */

const SVG = {
  lock: '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>',
  pin: '<svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor"><path d="M14 3v7l3 3v2h-5v7l-1 1-1-1v-7H5v-2l3-3V3h6z"/></svg>',
  save: '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M5 3h12l3 3v15H4V3z"/><rect x="8" y="3" width="8" height="6" rx="1"/><path d="M7 13h10v8H7z"/></svg>',
  unsave: '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M5 3h12l3 3v15H4V3z"/><path d="M8 13h8"/></svg>',
  plain: '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><rect x="4" y="4" width="16" height="16" rx="2"/><path d="M5 9h2M7 9h8c1 0 1 1 1 1 0 1-.3 1.7-1 1.7H9c-1 0-1 .8-1 .8V12l7 6"/></svg>',
  trash: '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 7h16M9 7V5h6v2m1 0-1 13H9L8 7"/></svg>',
};

/* ---------------- render ---------------- */

function timeAgo(ms) {
  const s = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (s < 5) return "now";
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  const d = Math.floor(h / 24);
  if (d < 7) return `${d}d`;
  return new Date(ms).toLocaleDateString();
}

function kindLabel(kind) {
  return kind === "html" ? "HTML" : kind === "rtf" ? "RTF" : "TEXT";
}

function actionButton(cls, svg, title, onClick, active) {
  const b = document.createElement("button");
  b.className = `action ${cls}${active ? " active" : ""}`;
  b.title = title;
  b.innerHTML = svg;
  b.addEventListener("click", (e) => e.stopPropagation());
  b.addEventListener("click", onClick);
  return b;
}

function render() {
  listEl.innerHTML = "";
  let lastGroup = null;
  let indexInList = 0;

  items.forEach((item, idx) => {
    const group = item.saved ? "snippets" : item.pinned ? "pinned" : "recent";
    if (group !== lastGroup) {
      const header = document.createElement("li");
      header.className = "group";
      header.textContent =
        group === "snippets" ? "Snippets" : group === "pinned" ? "Pinned" : "Recent";
      listEl.appendChild(header);
      lastGroup = group;
    }

    const li = document.createElement("li");
    li.className = `item${idx === selected ? " selected" : ""}`;
    li.dataset.idx = idx;

    const body = document.createElement("div");
    body.className = "body";

    const preview = document.createElement("div");
    preview.className = "preview";
    preview.textContent = item.preview || "(empty)";

    const meta = document.createElement("div");
    meta.className = "meta";

    const kind = document.createElement("span");
    kind.className = `kind ${item.kind}`;
    kind.textContent = kindLabel(item.kind);
    meta.appendChild(kind);

    if (item.saved && item.slot > 0) {
      const slot = document.createElement("span");
      slot.className = "badge slot";
      slot.textContent = item.slot;
      slot.title = `Paste with Ctrl+Alt+${item.slot}`;
      meta.appendChild(slot);
    }
    if (item.locked) {
      const lock = document.createElement("span");
      lock.className = "badge locked";
      lock.innerHTML = SVG.lock;
      lock.title = "Locked — protected from deletion";
      meta.appendChild(lock);
    }
    if (item.pinned) {
      const pin = document.createElement("span");
      pin.className = "badge pin";
      pin.innerHTML = SVG.pin;
      pin.title = "Pinned";
      meta.appendChild(pin);
    }

    const time = document.createElement("span");
    time.className = "time";
    time.textContent = timeAgo(item.created_at);
    meta.appendChild(time);

    body.appendChild(preview);
    body.appendChild(meta);
    li.appendChild(body);

    const actions = document.createElement("div");
    actions.className = "actions";

    actions.appendChild(
      actionButton("", SVG.save, item.saved ? "Unsave snippet" : "Save as snippet (Ctrl+S)", () => toggleSave(idx), item.saved)
    );
    actions.appendChild(
      actionButton("", SVG.pin, item.pinned ? "Unpin (Ctrl+P)" : "Pin (Ctrl+P)", () => togglePin(idx), item.pinned)
    );
    actions.appendChild(
      actionButton("", SVG.lock, item.locked ? "Unlock" : "Lock against deletion (Ctrl+L)", () => toggleLock(idx), item.locked)
    );
    actions.appendChild(
      actionButton("", SVG.plain, "Paste as plain text (Shift+Enter)", () => paste(idx, true))
    );
    actions.appendChild(
      actionButton("danger", SVG.trash, "Delete (Del)", () => remove(idx), false)
    );
    if (item.locked) actions.lastChild.style.display = "none";

    li.appendChild(actions);

    li.addEventListener("mousedown", (e) => {
      selected = idx;
      markSelected();
    });
    li.addEventListener("click", () => paste(idx, false));

    listEl.appendChild(li);
    indexInList += 1;
  });

  countEl.textContent = items.length ? String(items.length) : "";
  emptyEl.hidden = items.length > 0;
}

function markSelected() {
  listEl.querySelectorAll(".item").forEach((li) => {
    const isSel = Number(li.dataset.idx) === selected;
    li.classList.toggle("selected", isSel);
    if (isSel) li.scrollIntoView({ block: "nearest" });
  });
}

async function refresh() {
  const res = await invoke("search_items", { query });
  items = res || [];
  if (selected >= items.length) selected = Math.max(0, items.length - 1);
  if (items.length === 0) selected = 0;
  render();
}

/* ---------------- actions ---------------- */

function toast(msg) {
  statusEl.textContent = msg;
  statusEl.hidden = false;
  clearTimeout(statusEl._t);
  statusEl._t = setTimeout(() => (statusEl.hidden = true), 1800);
}

async function paste(idx, plain) {
  const item = items[idx];
  if (!item) return;
  try {
    await invoke("paste_item", { id: item.id, plain });
  } catch (err) {
    toast(String(err));
  }
}

async function togglePin(idx) {
  const item = items[idx];
  if (!item) return;
  await invoke("toggle_pin", { id: item.id });
  toast(item.pinned ? "Unpinned" : "Pinned");
  await refresh();
}

async function toggleLock(idx) {
  const item = items[idx];
  if (!item) return;
  const locked = await invoke("toggle_lock", { id: item.id });
  toast(locked ? "Locked — deletion blocked" : "Unlocked");
  await refresh();
}

async function toggleSave(idx) {
  const item = items[idx];
  if (!item) return;
  if (item.saved) {
    await invoke("unsave_item", { id: item.id });
    toast("Removed from snippets");
  } else {
    const slot = await invoke("save_item", { id: item.id });
    if (slot === -1) {
      toast("All 9 slots used — unsave one first");
      return;
    }
    toast(slot > 0 ? `Saved — paste with Ctrl+Alt+${slot}` : "Saved");
  }
  await refresh();
}

async function remove(idx) {
  const item = items[idx];
  if (!item) return;
  const deleted = await invoke("delete_item", { id: item.id });
  if (!deleted) {
    toast("Locked — unlock to delete");
    return;
  }
  await refresh();
}

/* ---------------- keys ---------------- */

function moveSelection(delta) {
  if (!items.length) return;
  const next = selected + delta;
  if (next >= 0 && next < items.length) {
    selected = next;
    markSelected();
  }
}

function onKey(e) {
  if (e.key === "ArrowDown") {
    e.preventDefault();
    moveSelection(1);
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    moveSelection(-1);
  } else if (e.key === "Enter") {
    e.preventDefault();
    paste(selected, e.shiftKey);
  } else if (e.key === "Escape") {
    e.preventDefault();
    invoke("hide_overlay");
  } else if (e.ctrlKey && e.key.toLowerCase() === "s") {
    e.preventDefault();
    toggleSave(selected);
  } else if (e.ctrlKey && e.key.toLowerCase() === "p") {
    e.preventDefault();
    togglePin(selected);
  } else if (e.ctrlKey && e.key.toLowerCase() === "l") {
    e.preventDefault();
    toggleLock(selected);
  } else if (e.key === "Delete" || e.key === "Backspace") {
    e.preventDefault();
    remove(selected);
  }
}

/* ---------------- wiring ---------------- */

let debounce;
searchEl.addEventListener("input", () => {
  query = searchEl.value;
  selected = 0;
  clearTimeout(debounce);
  debounce = setTimeout(refresh, 60);
});
searchEl.addEventListener("keydown", onKey);

listen("clipboard-changed", () => refresh());
listen("overlay-shown", () => {
  refresh();
  searchEl.focus();
});

refresh();