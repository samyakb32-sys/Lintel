// Clipboard — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// Nothing runs at import: declare surfaces here, begin work in `start`.
//
// One section: what was copied, newest first, as compact rows. A click puts an
// entry back and pastes it into the window the user was in. The page never
// reads the clipboard: Rust hands it the entries (crates/clipboard).

import "./style.css";
import { defineFeature, type FeatureCtx, type SectionHost } from "../../core/feature";
import { t } from "../../i18n/i18n";
import { btn, card, field, iconBtn, menu, row, stripRow, toggle, type MenuItem } from "../../ui/components";
import { icon } from "../../ui/icons";
import { clear, h } from "../../views/dom";

type Kind = "text" | "link" | "image" | "files";
type Filter = "all" | "text" | "link" | "code" | "image" | "files";

interface Entry {
  id: string;
  kind: Kind;
  /** The text or the address (its first 4 000 characters); for files, their paths, one a line. */
  text: string;
  chars: number;
  width: number;
  height: number;
  size: number;
  /** The program it was copied in (`chrome.exe`) and where that program is. */
  app: string;
  exe: string;
  at: number;
  pinned: boolean;
}

interface Listing {
  entries: Entry[];
  /** Copies left out since the start because they were, or looked, secret. */
  skipped: number;
  recording: boolean;
}

interface Pasted {
  pasted: boolean;
  copied?: boolean;
  elevated?: boolean;
  why?: string;
}

/** Case, accents and width folded, so "resume" finds "Résumé". */
const fold = (s: string) => s.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();

/** A text of several lines that reads like a program. Worked out here, never stored. */
const isCode = (e: Entry) =>
  e.kind === "text" && /\n/.test(e.text) && /[{};]|=>|^\s*(def|fn|function|class|import|const|let|var|#include|SELECT)\b/m.test(e.text);

const shows = (e: Entry, f: Filter) =>
  f === "all" || (f === "code" ? isCode(e) : f === "text" ? e.kind === "text" && !isCode(e) : e.kind === f);

const fileNames = (e: Entry) => e.text.split("\n").map((p) => p.split(/[\\/]/).pop() ?? p);

function ago(cx: FeatureCtx, at: number): string {
  const min = Math.floor((Date.now() - at) / 60_000);
  if (min < 1) return cx.t("just now");
  if (min < 60) return cx.t("{n} min", { n: min });
  if (min < 1440) return cx.t("{n} h", { n: Math.floor(min / 60) });
  return cx.t("{n} d", { n: Math.floor(min / 1440) });
}

function label(cx: FeatureCtx, e: Entry): string {
  if (e.kind === "image") return cx.t("Image {w} × {h}", { w: e.width, h: e.height });
  if (e.kind === "files") {
    const names = fileNames(e);
    return names.length === 1 ? names[0] : `${names[0]}  ·  ${cx.t("{count} files", { count: names.length })}`;
  }
  return e.text.trim().replace(/\s+/g, " ").slice(0, 200);
}

/** What a paste came to, as a notice and as a line for the page. */
function pasteText(cx: FeatureCtx, r: Pasted | null): string {
  if (!r) return cx.t("Copied, but it could not be pasted. Press Ctrl+V where you want it.");
  if (r.pasted) return cx.t("Pasted");
  if (r.copied) return cx.t("Copied");
  return r.elevated
    ? cx.t("Copied. That window runs as administrator, so press Ctrl+V there yourself.")
    : cx.t("Copied, but it could not be pasted. Press Ctrl+V where you want it.");
}

// Shared by the section, the activity and `start`: the copies of the last moments.
let burst: number[] = [];
let burstTimer = 0;
const burstOn = () => burst.length >= 2;

function buildClipboard(cx: FeatureCtx): SectionHost {
  const t = cx.t;
  let all: Entry[] = [];
  let skipped = 0;
  let recording = true;
  let filter: Filter = "all";
  let query = "";
  /** The row the arrow keys are on. */
  let cursor = 0;
  const selected = new Set<string>();
  let anchor: string | null = null;
  let dragging = false;
  let note = "";
  let noteTimer = 0;
  let confirmTimer = 0;
  const pictures = new Map<string, string>();
  const appIcons = new Map<string, string>();

  const rows = h("div", { class: "f-clipboard-rows list", role: "listbox", "aria-multiselectable": "true" });
  const empty = h("div", { class: "f-clipboard-empty" });
  const status = h("span", { class: "isl-meta" });
  const tabs = h("div", { class: "f-clipboard-tabs", role: "tablist" });
  const search = field({ icon: "search", placeholder: t("Search"), onInput: (v) => { query = v; cursor = 0; draw(); } });
  search.classList.add("f-clipboard-search");
  search.hidden = true;
  const searchBtn = iconBtn("search", t("Search"), () => openSearch());
  const pauseBtn = iconBtn("pause", t("Pause recording"), () => void cx.settings.patch({ recording: !recording }));
  const clearBtn = iconBtn("trash", t("Clear history"), () => {
    // Twice within three seconds: the first click only asks.
    if (!clearBtn.classList.contains("danger")) {
      clearBtn.classList.replace("ghost", "danger");
      clearBtn.title = t("Click again to clear");
      confirmTimer = window.setTimeout(disarm, 3000);
      return;
    }
    disarm();
    void cx.call("clear");
  });
  const disarm = () => {
    window.clearTimeout(confirmTimer);
    clearBtn.classList.replace("danger", "ghost");
    clearBtn.title = t("Clear history");
  };
  const el = h(
    "div",
    { class: "f-clipboard-page" },
    h("div", { class: "isl-row" }, tabs, search, h("span", { class: "isl-gap" }), searchBtn, pauseBtn, clearBtn),
    rows,
    empty,
    h("div", { class: "isl-row" }, status),
  );

  const openSearch = () => {
    search.hidden = false;
    tabs.hidden = true;
    search.input.focus();
  };
  const closeSearch = () => {
    search.hidden = true;
    tabs.hidden = false;
    search.input.value = "";
    query = "";
    draw();
  };

  const visible = (): Entry[] => {
    const tokens = fold(query).split(/\s+/).filter(Boolean);
    return all.filter((e) => {
      if (!shows(e, filter)) return false;
      if (tokens.length === 0) return true;
      const hay = fold(`${e.kind === "files" ? fileNames(e).join(" ") : e.text} ${e.app} ${e.kind}`);
      return tokens.every((tok) => hay.includes(tok));
    });
  };
  const chosen = () => all.filter((e) => selected.has(e.id));

  // ── Drawing ─────────────────────────────────────────────────────────────────

  const say = (text: string, ms = 2500) => {
    window.clearTimeout(noteTimer);
    note = text;
    if (ms > 0) noteTimer = window.setTimeout(() => say("", 0), ms);
    paint();
  };

  /** The status line, the selection and the cursor: what changes without the entries changing. */
  const paint = () => {
    const list = visible();
    [...rows.children].forEach((node, i) => {
      const id = (node as HTMLElement).dataset.id ?? "";
      node.classList.toggle("on", selected.has(id));
      node.classList.toggle("f-clipboard-cursor", i === cursor && list.length > 0);
      node.setAttribute("aria-selected", String(selected.has(id)));
    });
    status.textContent =
      note ||
      (selected.size > 0
        ? t("{count} selected", { count: selected.size })
        : !recording
          ? t("Recording is paused. Nothing you copy is kept.")
          : skipped === 1
            ? t("1 copy left out as a key")
            : skipped > 1
              ? t("{count} copies left out as a key", { count: skipped })
              : "");
    pauseBtn.title = recording ? t("Pause recording") : t("Resume recording");
    pauseBtn.replaceChildren(icon(recording ? "pause" : "play", 14));
    pauseBtn.classList.toggle("f-clipboard-paused", !recording);
    clearBtn.disabled = all.every((e) => e.pinned);
  };

  const setPicture = (box: HTMLElement, url: string) => box.replaceChildren(h("img", { src: url, alt: "", draggable: "false" }));

  /** Left end of a row: the picture, the program it came from, or the kind. */
  const mark = (e: Entry) => {
    const kindIcon = e.kind === "link" ? "link" : e.kind === "image" ? "image" : e.kind === "files" ? "file" : isCode(e) ? "code" : "text";
    const box = h("span", { class: "f-clipboard-mark" }, icon(kindIcon, 16));
    const own = e.kind === "image" || e.kind === "files";
    const key = own ? e.id : e.exe;
    const known = own ? pictures.get(key) : appIcons.get(key);
    if (known) setPicture(box, known);
    else if (key)
      void cx.call<string>(own ? "thumb" : "icon", own ? { id: e.id } : { exe: e.exe }).then((url) => {
        if (!url) return;
        (own ? pictures : appIcons).set(key, url);
        setPicture(box, url);
      });
    box.classList.toggle("f-clipboard-picture", own);
    return box;
  };

  const rowOf = (e: Entry, place: number) => {
    const acts = h(
      "span",
      { class: "f-clipboard-acts" },
      iconBtn("copy", t("Copy"), () => copy([e])),
      iconBtn("pin", e.pinned ? t("Unpin") : t("Pin"), () => void cx.call("pin", { id: e.id, on: !e.pinned })),
      iconBtn("trash", t("Delete"), () => void cx.call("remove", { ids: [e.id] })),
    );
    // The buttons act on their own: a click on one is not a click on the row.
    acts.addEventListener("click", (ev) => ev.stopPropagation());
    acts.addEventListener("pointerdown", (ev) => ev.stopPropagation());
    const node = h(
      "div",
      { class: `list-item f-clipboard-row${e.pinned ? " f-clipboard-pinned" : ""}`, role: "option", tabindex: "-1", "data-id": e.id, title: label(cx, e) },
      mark(e),
      h("span", { class: e.kind === "link" || isCode(e) ? "f-clipboard-mono" : "", text: label(cx, e) }),
      e.pinned ? h("span", { class: "f-clipboard-pin" }, icon("pin", 13)) : null,
      h("small", { text: [e.app.replace(/\.exe$/i, ""), ago(cx, e.at)].filter(Boolean).join(" · ") }),
      place < 9 ? h("span", { class: "kbd", text: String(place + 1) }) : null,
      acts,
    );
    node.addEventListener("pointerdown", (ev) => press(ev, e));
    node.addEventListener("click", (ev) => click(ev, e));
    node.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      if (!selected.has(e.id)) {
        selected.clear();
        paint();
      }
      entryMenu(e, { x: ev.clientX, y: ev.clientY });
    });
    return node;
  };

  const tabOf = (f: Filter, name: string, iconName: string) => {
    const count = all.filter((e) => shows(e, f)).length;
    const node = h(
      "button",
      { class: `tab${filter === f ? " on" : ""}`, role: "tab", title: name, "aria-selected": String(filter === f), onclick: () => { filter = f; cursor = 0; draw(); } },
      icon(iconName, 13),
      filter === f ? h("span", { text: name }) : null,
      h("small", { text: String(count) }),
    );
    node.disabled = count === 0 && f !== "all";
    return node;
  };

  const draw = () => {
    for (const id of [...selected]) if (!all.some((e) => e.id === id)) selected.delete(id);
    for (const key of [...pictures.keys()]) if (!all.some((e) => e.id === key)) pictures.delete(key);
    tabs.replaceChildren(
      tabOf("all", t("All"), "clipboard"),
      tabOf("text", t("Text"), "text"),
      tabOf("link", t("Links"), "link"),
      tabOf("code", t("Code"), "code"),
      tabOf("image", t("Images"), "image"),
      tabOf("files", t("Files"), "file"),
    );
    const list = visible();
    cursor = Math.max(0, Math.min(cursor, list.length - 1));
    clear(rows);
    list.forEach((e, i) => rows.append(rowOf(e, i)));
    rows.hidden = list.length === 0;
    empty.hidden = list.length > 0;
    empty.replaceChildren(
      icon("clipboard", 20),
      h("span", { text: all.length === 0 ? (recording ? t("Nothing copied yet. What you copy appears here.") : t("Recording is paused. Nothing you copy is kept.")) : t("Nothing matches.") }),
    );
    paint();
  };

  const refresh = async () => {
    const got = await cx.call<Listing>("list");
    if (!got) return;
    ({ entries: all, skipped, recording } = got);
    draw();
  };

  // ── Acting on entries ───────────────────────────────────────────────────────

  const ids = (list: Entry[]) => list.map((e) => e.id);

  const paste = async (list: Entry[]) => {
    if (list.length === 0) return;
    const result = await cx.call<Pasted>("paste", { ids: ids(list) });
    say(pasteText(cx, result), result?.pasted || result?.copied ? 1500 : 6000);
    selected.clear();
    paint();
  };

  const copy = (list: Entry[]) => {
    void cx.call("copy", { ids: ids(list) });
    say(t("Copied"), 1500);
  };

  /** A plain click pastes; with Ctrl or Shift it builds a selection instead. */
  const click = (ev: MouseEvent, e: Entry) => {
    const list = visible();
    if (ev.ctrlKey) {
      if (!selected.delete(e.id)) selected.add(e.id);
      anchor = e.id;
      return paint();
    }
    const from = list.findIndex((x) => x.id === anchor);
    if (ev.shiftKey && from >= 0) {
      const to = list.indexOf(e);
      selected.clear();
      for (const x of list.slice(Math.min(from, to), Math.max(from, to) + 1)) selected.add(x.id);
      return paint();
    }
    void paste(selected.has(e.id) ? chosen() : [e]);
  };

  /** A press that moves 3 px becomes a drag of the row, or of the selection it is part of. */
  const press = (ev: PointerEvent, e: Entry) => {
    if (ev.button !== 0 || dragging) return;
    const { clientX: x, clientY: y } = ev;
    const move = (m: PointerEvent) => {
      if (Math.hypot(m.clientX - x, m.clientY - y) < 3) return;
      done();
      void dragOut(selected.has(e.id) ? chosen() : [e]);
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      window.removeEventListener("pointercancel", done);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done);
    window.addEventListener("pointercancel", done);
  };

  const dragOut = async (list: Entry[]) => {
    dragging = true;
    const release = cx.island.hold();
    try {
      // Returns when the button is released, wherever that is.
      await cx.call("drag", { ids: ids(list) });
    } finally {
      // The release of the button over the island must not count as a click on the row.
      window.setTimeout(() => (dragging = false), 50);
      release();
    }
  };

  let openMenu: HTMLElement | null = null;

  const entryMenu = (e: Entry, at: { x: number; y: number }) => {
    const list = selected.size > 0 ? chosen() : [e];
    const one = list.length === 1;
    const items: (MenuItem | null)[] = [
      { label: t("Paste"), icon: "clipboard", onClick: () => void paste(list) },
      { label: t("Copy"), icon: "copy", onClick: () => copy(list) },
    ];
    if (one) {
      items.push({ label: e.pinned ? t("Unpin") : t("Pin"), icon: "pin", onClick: () => void cx.call("pin", { id: e.id, on: !e.pinned }) });
      if (e.kind !== "text") items.push({ label: t("Open"), icon: "external", onClick: () => void cx.call("open", { id: e.id }) });
      if (e.kind === "files" || e.kind === "image") items.push({ label: t("Show in folder"), icon: "search", onClick: () => void cx.call("reveal", { id: e.id }) });
    }
    items.push(null, { label: t("Delete"), icon: "trash", danger: true, onClick: () => void cx.call("remove", { ids: ids(list) }) });
    openMenu = menu(items, at);
  };

  // ── Keys (the island has the keyboard after an explicit open) ───────────────

  const onKey = (ev: KeyboardEvent) => {
    const list = visible();
    const typing = document.activeElement === search.input;
    if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
      cursor = Math.max(0, Math.min(list.length - 1, cursor + (ev.key === "ArrowDown" ? 1 : -1)));
      paint();
      rows.children[cursor]?.scrollIntoView({ block: "nearest" });
    } else if (ev.key === "Enter") {
      // Shift+Enter copies without pasting.
      const what = selected.size > 0 ? chosen() : list.slice(cursor, cursor + 1);
      if (ev.shiftKey) copy(what);
      else void paste(what);
    } else if (ev.ctrlKey && /^[1-9]$/.test(ev.key)) {
      void paste(list.slice(Number(ev.key) - 1, Number(ev.key)));
    } else if (!typing && ev.key.length === 1 && !ev.ctrlKey && !ev.altKey && !ev.metaKey) {
      // A letter starts a search; the field takes it from here.
      openSearch();
      return;
    } else return;
    ev.preventDefault();
    ev.stopPropagation();
  };

  let off: (() => void)[] = [];
  return {
    el,
    show() {
      // Opens on the last copy: no filter, no search, the first row under the cursor.
      filter = "all";
      cursor = 0;
      if (!search.hidden) closeSearch();
      window.addEventListener("keydown", onKey, true);
      off = [() => window.removeEventListener("keydown", onKey, true), cx.on("changed", () => void refresh())];
      // The section is what the two quick copies pointed to.
      burst = [];
      cx.island.activityChanged("copies");
      void refresh();
    },
    hide() {
      for (const fn of off) fn();
      off = [];
      disarm();
      selected.clear();
      openMenu?.remove();
    },
    sync() {},
    command(command) {
      if (command !== "back") return false;
      if (selected.size > 0) {
        selected.clear();
        paint();
        return true;
      }
      if (!search.hidden) {
        closeSearch();
        return true;
      }
      return false;
    },
  };
}

// ── Settings ──────────────────────────────────────────────────────────────────

function settingsBody(cx: FeatureCtx): HTMLElement {
  const t = cx.t;
  const el = h("div", { class: "f-clipboard-settings" });

  const build = (s: Record<string, unknown>) => {
    const sw = (key: string, fallback: boolean) => toggle(s[key] === undefined ? fallback : s[key] === true, (on) => void cx.settings.patch({ [key]: on }));
    const pick = (key: string, options: [number, string][]) => {
      const select = h("select", {}, ...options.map(([value, name]) => h("option", { value: String(value), text: name })));
      select.value = String(s[key] ?? options[0][0]);
      select.addEventListener("change", () => void cx.settings.patch({ [key]: Number(select.value) }));
      return h("label", { class: "field" }, select);
    };
    const words = (key: string, placeholder: string) => {
      const f = field({ value: String(s[key] ?? ""), placeholder });
      f.input.addEventListener("change", () => void cx.settings.patch({ [key]: f.input.value }));
      return f;
    };
    const days = (n: number): [number, string] => [n, n === 0 ? t("For good") : n === 1 ? t("1 day") : t("{n} days", { n, count: n })];
    const secs = (n: number): [number, string] => [
      n,
      n === 0 ? t("Never") : n < 60 ? t("{n} seconds", { n, count: n }) : n === 60 ? t("1 minute") : t("{n} minutes", { n: n / 60, count: n / 60 }),
    ];

    // The link cleaner, tried on a link of the user's own: nothing is copied.
    const result = h("div", { class: "f-clipboard-tried" });
    const tester = field({
      icon: "link",
      placeholder: t("Paste a link to see it cleaned"),
      onInput: (text) => {
        if (!text.trim()) return result.replaceChildren();
        void cx.call<{ url: string; removed: string[] } | null>("clean", { text }).then((c) => {
          if (tester.input.value !== text) return;
          const line = !c ? t("Not a link") : c.removed.length === 0 ? t("Nothing to clean") : t("Removed: {names}", { names: c.removed.join(", ") });
          result.replaceChildren(c && c.removed.length > 0 ? h("code", { text: c.url }) : "", h("small", { text: line }));
        });
      },
    });

    /** A button that deletes on its second click. */
    const sure = (name: string, all: boolean) => {
      let armed = 0;
      const b = btn(name, "danger", () => {
        if (!armed) {
          armed = window.setTimeout(() => { armed = 0; b.firstElementChild!.textContent = name; }, 3000);
          b.firstElementChild!.textContent = t("Click again to delete");
          return;
        }
        window.clearTimeout(armed);
        armed = 0;
        void cx.call("clear", { all }).then(() => (b.firstElementChild!.textContent = t("Deleted")));
      });
      return b;
    };

    el.replaceChildren(
      h("div", { class: "section-title", text: t("History") }),
      card([
        row(t("Record what I copy"), sw("recording", true), t("Kept on this PC only. Copies that a password manager marks as secret are never recorded.")),
        row(t("Record images and files too"), sw("media", true)),
        row(t("Leave out text that looks like a key"), sw("skipSensitive", true), t("Only text that looks like a real secret key counts: a known key start, a long random string or a private key. Ordinary text and code are kept.")),
        row(t("Apps to leave out"), words("ignoredApps", "notepad.exe"), t("Program names separated by commas, such as notepad.exe. Password managers are always left out.")),
        row(t("Entries to keep"), pick("limit", [50, 100, 250, 500].map((n) => [n, String(n)]))),
        row(t("Keep entries for"), pick("keepDays", [0, 1, 7, 30, 90].map(days)), t("Pinned entries are always kept.")),
        row(t("A click copies without pasting"), sw("copyOnly", false)),
        row(t("Show a notice for each copy"), sw("notify", false)),
      ]),
      h("div", { class: "section-title", text: t("Auto clear") }),
      card([
        row(t("Empty the clipboard after"), pick("clearAfter", [0, 10, 20, 30, 60, 120, 300, 600].map(secs)), t("Counted from the last copy. The history is not touched.")),
        row(t("Empty it when the screen locks"), sw("clearOnLock", false)),
        row(t("Empty it when the PC sleeps"), sw("clearOnSleep", false)),
        row(t("Empty it when the display turns off"), sw("clearOnDisplayOff", false)),
        row(t("Also empty the Windows clipboard history (Win+V)"), sw("clearWindowsHistory", false)),
      ]),
      h("div", { class: "section-title", text: t("Links") }),
      card([
        row(t("Remove tracking parameters from copied links"), sw("cleanUrls", false), t("The link on the clipboard is replaced by the clean one.")),
        row(t("More parameters to remove"), words("cleanExtra", "ref"), t("Names separated by commas, such as ref.")),
        row(t("Try a link"), tester),
        result,
      ]),
      h("div", { class: "section-title", text: t("Delete the history") }),
      card([
        row(t("Clear history"), sure(t("Clear history"), false), t("Deletes the entries from this PC. Pinned entries stay.")),
        row(t("Delete everything, pinned entries too"), sure(t("Delete"), true)),
      ]),
    );
  };

  void cx.settings.get().then(build);
  return el;
}

// ── The feature ───────────────────────────────────────────────────────────────

export default defineFeature({
  id: "clipboard",
  voice: () => import("./voice"),
  // Two copies in a row: for half a minute the strip leads to the Clipboard section.
  // It says how many, never what.
  activities: [
    {
      id: "copies",
      order: 6.5,
      section: "clipboard",
      isActive: burstOn,
      render: () => stripRow({ icon: "clipboard", tone: "info", title: t("Clipboard"), detail: t("{count} new copies", { count: burst.length }) }),
    },
  ],
  sections: [{ id: "clipboard", title: "Clipboard", icon: "clipboard", letter: "v", height: 300, keyboard: true, keys: ["ArrowUp", "ArrowDown", "Enter", "Shift+Enter"], build: buildClipboard }],
  settings: settingsBody,
  shortcuts: { open: (cx) => cx.island.open("clipboard") },
  // Rows for the command bar: the entries that match, and the link cleaner.
  commands: async (query, cx) => {
    const got = await cx.call<Listing>("list");
    const tokens = fold(query).split(/\s+/).filter(Boolean);
    const found = (got?.entries ?? [])
      .filter((e) => tokens.length > 0 && tokens.every((tok) => fold(e.kind === "files" ? fileNames(e).join(" ") : e.text).includes(tok)))
      .slice(0, 8)
      .map((e) => ({
        id: `clipboard:${e.id}`,
        title: label(cx, e),
        subtitle: cx.t("Clipboard"),
        icon: e.kind === "link" ? "link" : e.kind === "image" ? "image" : e.kind === "files" ? "file" : "text",
        run: async () => void (await cx.call("paste", { id: e.id })),
      }));
    return [
      ...found,
      {
        id: "clipboard:clean",
        title: cx.t("Clean the copied link"),
        icon: "link",
        keywords: ["url", "tracking", "utm"],
        run: async () => {
          const c = await cx.call<{ removed: string[] } | null>("clean-clipboard");
          const detail = !c ? cx.t("Not a link") : c.removed.length === 0 ? cx.t("Nothing to clean") : cx.t("Removed: {names}", { names: c.removed.join(", ") });
          cx.island.notice({ kind: "clipboard", title: cx.t("Link cleaned"), detail, icon: "link", section: "clipboard" });
        },
      },
    ];
  },
  start: (cx) => {
    let notify = false;
    void cx.settings.get().then((s) => (notify = s.notify === true));
    const offs = [
      cx.settings.onChange((s) => (notify = s.notify === true)),
      cx.on<{ kind: Kind }>("captured", ({ kind }) => {
        // Two copies within ten seconds: the strip leads to the section for the next thirty.
        const now = Date.now();
        burst = [...burst.filter((at) => now - at < 10_000), now];
        window.clearTimeout(burstTimer);
        burstTimer = window.setTimeout(() => {
          burst = [];
          cx.island.activityChanged("copies");
        }, 30_000);
        if (burstOn()) cx.island.redraw("copies");
        if (burst.length === 2) cx.island.activityChanged("copies");
        if (notify) {
          const name = kind === "link" ? "link" : kind === "image" ? "image" : kind === "files" ? "file" : "text";
          cx.island.notice({ kind: "clipboard", title: cx.t("Copied"), icon: name, section: "clipboard" });
        }
      }),
      cx.on<{ removed: string[] }>("cleaned", ({ removed }) =>
        cx.island.notice({ kind: "clipboard", title: cx.t("Link cleaned"), detail: cx.t("Removed: {names}", { names: removed.join(", ") }), icon: "link", tone: "ok", section: "clipboard" }),
      ),
      cx.on("cleared", () => cx.island.notice({ kind: "clipboard", title: cx.t("Clipboard cleared"), icon: "clipboard", section: "clipboard" })),
      cx.on<Pasted>("paste-failed", (r) =>
        cx.island.notice({
          kind: "clipboard",
          // One row of the strip: the reason in two words (a sentence would be cut).
          title: cx.t("Could not paste as plain text"),
          detail: r.elevated ? cx.t("Administrator window") : cx.t("Try again"),
          icon: "alert",
          tone: "warn",
        }),
      ),
    ];
    return () => {
      for (const off of offs) off();
      window.clearTimeout(burstTimer);
      burst = [];
    };
  },
});
