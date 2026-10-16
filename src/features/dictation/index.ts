// Dictation — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// The mic button lives in the chat view's `.chat-mic-slot`. The microphone is opened by the click and
// closed on every path out: second click, fold, chat hidden, feature off, page unload (Rust also stops it on quit).

import "./style.css";
import { defineFeature, type FeatureCtx } from "../../core/feature";
import { btn, card, kbd, row, stripRow } from "../../ui/components";
import { t } from "../../i18n/i18n";
import { State } from "../../core/state";
import { Bridge } from "../../core/bridge";
import { h } from "../../views/dom";
import { icon } from "../../ui/icons";
import { picker } from "./picker";

interface Err { code: string; privacy: boolean; message: string }

// The hold-to-dictate shortcut: Rust reports its state, the strip shows it.
const HOLD = "dictation-hold";
let hold = { state: "idle", level: 0 };

/**
 * The combination as a button. A click on it (or on the row's name) listens: the keys show as they
 * are held, letting go saves, Esc cancels, Backspace removes, and a press that cannot be the
 * combination says why. The rules are in picker.ts.
 */
function recorder(cx: FeatureCtx): { el: HTMLElement; listen: () => void } {
  let chord = "";
  let stop: (() => void) | null = null;
  const b = h("button", { class: "btn secondary sm", title: cx.t("Click, then press the keys") });
  const clear = btn(cx.t("Clear"), "ghost", () => { stop?.(); save(""); });
  clear.classList.add("sm");
  const note = h("span", { class: "f-dictation-hold-note" });
  const keys = (c: string, empty: string) => b.replaceChildren(...(c ? c.split("+").map((k) => kbd(k)) : [h("span", { text: empty })]));
  const say = (text: string, warn = false) => { note.textContent = text; note.classList.toggle("warn", warn); };
  const show = () => {
    b.classList.remove("recording");
    keys(chord, cx.t("Not set"));
    say("");
    // Not `hidden`: the display of .btn wins over it, and Clear stayed on screen with nothing to clear.
    clear.style.display = chord ? "" : "none";
  };
  const save = (c: string) => { chord = c; show(); void cx.settings.patch({ hold: c }); };
  const listen = () => {
    if (stop) return;
    const pick = picker();
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat) return;
      const r = e.type === "keydown" ? pick.down(e) : pick.up(e);
      if (r.kind === "held") return keys(r.keys, cx.t("Press keys…"));
      if (r.kind === "refused") return say(cx.t(r.why, r.vars), true);
      end();
      if (r.kind === "cancel") show();
      else save(r.kind === "save" ? r.chord : "");
    };
    const outside = (e: Event) => { if (!b.contains(e.target as Node)) cancel(); };
    const end = () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKey, true);
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("blur", cancel);
      stop = null;
      void Bridge.shortcutsSuspend(false);
    };
    const cancel = () => { end(); show(); };
    stop = cancel;
    b.classList.add("recording");
    keys("", cx.t("Press keys…"));
    say(cx.t("Press the keys, then let go. Esc cancels, Backspace removes it."));
    // As on the Shortcuts page: the app's own shortcuts and key hooks let go meanwhile. A combination
    // one of them holds (this one, once it is set) never reached this page, and nothing said so.
    void Bridge.shortcutsSuspend(true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKey, true);
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("blur", cancel);
  };
  b.addEventListener("click", () => (stop ? stop() : listen()));
  void cx.settings.get().then((v) => { if (!stop) { chord = typeof v.hold === "string" ? v.hold : ""; show(); } });
  show();
  return { el: h("span", { class: "f-dictation-hold" }, b, clear, note), listen };
}

function settingsPage(cx: FeatureCtx): HTMLElement {
  const msg = h("span", { class: "f-dictation-note" });
  const dl = btn(cx.t("Download speech model (about 19 MB)"), "secondary", () => {
    dl.disabled = true;
    msg.textContent = cx.t("Downloading...");
    void cx.call("download");
  });
  const select = h("select", {}, h("option", { value: "whistle", text: cx.t("Whistle (on this device)") }), h("option", { value: "windows", text: cx.t("Windows speech") })) as HTMLSelectElement;
  select.onchange = () => void cx.settings.patch({ engine: select.value });
  void cx.settings.get().then((s) => (select.value = s.engine === "windows" ? "windows" : "whistle"));
  const hold = recorder(cx);
  const holdRow = row(cx.t("Hold to dictate"), hold.el,
    cx.t("Hold the keys, speak, let go: the text is typed where the cursor is. Esc cancels. Nothing is typed into password fields or windows that run as administrator."));
  // The name of the row starts it too, as a label does for its field.
  holdRow.querySelector(".row-label")?.addEventListener("click", hold.listen);
  const state = () => cx.call<{ installed: boolean; downloading: boolean }>("status").then((r) => {
    dl.disabled = !!r?.downloading || !!r?.installed;
    msg.textContent = r?.downloading ? cx.t("Downloading...") : r?.installed ? cx.t("Speech model ready.") : "";
  });
  void state();
  cx.on("downloaded", () => void state());
  cx.on<{ message: string }>("download-error", (e) => { dl.disabled = false; msg.textContent = e.message; });
  return h("div", { class: "f-dictation-settings" },
    card([
      row(cx.t("Engine"), h("label", { class: "field" }, select, icon("chevd", 14)),
        cx.t("Whistle works on this computer and understands English, German, French, Spanish, Italian, Dutch and Polish. For other languages, use Windows speech, which needs online speech recognition turned on in Windows.")),
      holdRow,
      row(cx.t("Speech model"), dl, cx.t("Nothing is downloaded until you click. The files are checked before they are used.")),
      msg,
    ]),
  );
}

export default defineFeature({
  id: "dictation",

  settings: settingsPage,

  activities: [
    {
      id: HOLD,
      order: 2,
      hold: true,
      isActive: () => hold.state === "listening" || hold.state === "working",
      render: () =>
        hold.state === "working"
          ? stripRow({ icon: "mic", tone: "info", title: t("Writing it down") })
          : stripRow({ icon: "mic", tone: "bad", title: t("Listening"), detail: t("Release to type, Esc to cancel"), level: hold.level }),
    },
  ],

  start(cx) {
    let slot: HTMLElement | null = null;
    let btn: HTMLButtonElement | null = null;
    let listening = false;
    let msgTimer: number | undefined;
    let dead = false;

    const stop = () => {
      if (!listening) return;
      listening = false;
      draw();
      void cx.call("stop");
    };

    const draw = () => {
      if (!btn) return;
      btn.classList.toggle("on", listening);
      const label = cx.t(listening ? "Stop dictation" : "Dictate");
      btn.title = label;
      btn.setAttribute("aria-label", label);
      btn.setAttribute("aria-pressed", String(listening));
    };

    const say = (err: Err) => {
      if (!slot) return;
      slot.querySelector(".f-dictation-msg")?.remove();
      const msg = err.code === "nomodel"
        ? h("div", { class: "f-dictation-msg", text: cx.t("Download the speech model in Settings") })
        : err.privacy
        ? h("div", { class: "f-dictation-msg" }, h("span", { text: cx.t("Online speech recognition is off") }),
            h("button", { text: cx.t("Turn it on"), onclick: () => void cx.call("settings") }))
        : h("div", { class: "f-dictation-msg", title: `${err.code} ${err.message}`, text: cx.t("Dictation did not start") });
      slot.append(msg);
      clearTimeout(msgTimer);
      msgTimer = window.setTimeout(() => msg.remove(), 8000);
    };

    const field = () => slot?.closest(".isl-compose")?.querySelector("input") as HTMLInputElement | null;

    const insert = (text: string) => {
      const input = field();
      if (!input || input.disabled) return;
      const a = input.selectionStart ?? input.value.length;
      const b = input.selectionEnd ?? a;
      const pre = input.value.slice(0, a);
      const lead = pre && !/\s$/.test(pre) ? " " : "";
      input.value = pre + lead + text + input.value.slice(b);
      const at = a + lead.length + text.length;
      input.setSelectionRange(at, at);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    };

    const click = () => {
      if (listening) return stop();
      listening = true;
      draw();
      void cx.call("start");
    };

    const mount = async (el: HTMLElement) => {
      slot = el;
      const r = await cx.call<{ ok: boolean; error?: Err }>("available");
      if (dead || slot !== el) return;
      // Hidden when the recognizer cannot be created at all; kept when only the privacy switch is off.
      if (!r || (!r.ok && !r.error?.privacy)) return;
      btn = h("button", { class: "btn ghost icon f-dictation-btn" }, icon("mic", 14)) as HTMLButtonElement;
      btn.addEventListener("click", click);
      el.prepend(btn);
      draw();
    };

    const remount = () => { btn?.remove(); btn = null; stop(); slot = null; find(); };
    const find = () => {
      const el = document.querySelector<HTMLElement>(".chat-mic-slot");
      if (el && el !== slot) void mount(el);
    };
    find();
    const offSettings = cx.settings.onChange(() => remount());
    const watch = new MutationObserver(find);
    watch.observe(document.body, { childList: true, subtree: true });

    const offs = [
      cx.on<{ text: string }>("text", (p) => insert(p.text)),
      cx.on("ended", () => { listening = false; draw(); }),
      cx.on<{ state: string }>("hold", ({ state }) => {
        if (state === "blocked") {
          cx.island.notice({ kind: "mic", title: cx.t("Dictation"), detail: cx.t("Nothing typed: the field or window is not allowed"), icon: "mic", tone: "warn" });
          return;
        }
        hold = { state, level: 0 };
        cx.island.activityChanged(HOLD);
      }),
      cx.on<{ level: number }>("level", ({ level }) => {
        if (hold.state !== "listening") return;
        hold.level = level;
        cx.island.redraw(HOLD);
      }),
      cx.on("hold-error", () => cx.island.notice({ kind: "mic", title: cx.t("Dictation"), detail: cx.t("Dictation did not start"), icon: "mic", tone: "bad" })),
      cx.on<Err>("error", (e) => { listening = false; draw(); say(e); }),
    ];

    // Island folded, another view, or the chat not on screen: the microphone closes.
    const unsub = State.subscribe(() => {
      if (listening && (State.mode !== "expanded" || State.view !== "prompt" || !slot?.offsetParent)) stop();
    });
    const unload = () => stop();
    window.addEventListener("pagehide", unload);

    return () => {
      dead = true;
      hold = { state: "idle", level: 0 };
      stop();
      void cx.call("stop");
      watch.disconnect();
      offSettings();
      unsub();
      offs.forEach((f) => f());
      window.removeEventListener("pagehide", unload);
      clearTimeout(msgTimer);
      slot?.replaceChildren();
    };
  },
});
