// Recorder — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// Nothing runs at import: declare surfaces here, begin work in `start`.
//
// Rust records and owns the file from the first picture to the last; this
// file only shows it. While a recording runs the strip holds a red dot, the
// time recorded and two buttons, Pause and Stop. When the file is closed a
// notice says so: its mark plays the film, its button shows it in its folder.
// There is no section: the recording starts from its shortcut (or from the
// island's Utilities), and the files are in the folder the settings name.

import "./style.css";
import { Bridge } from "../../core/bridge";
import { defineFeature, type FeatureCtx, type Notice } from "../../core/feature";
import { displayKeys } from "../../core/shortcuts";
import { t } from "../../i18n/i18n";
import { btn, card, field, kbd, row, stripRow, toggle } from "../../ui/components";
import { icon } from "../../ui/icons";
import { clear, h } from "../../views/dom";

/** What Rust says of the recording (`state`), and when it said it. */
interface Rec {
  recording: boolean;
  paused: boolean;
  /** Milliseconds recorded when this was said, pauses left out. */
  elapsed: number;
  at: number;
}

interface Saved {
  name: string;
  seconds: number;
  bytes: number;
  /** "user", or what ended the recording by itself: "disk", "closed", "lock", "sleep", "off", "failed". */
  why: string;
}

const IDLE: Rec = { recording: false, paused: false, elapsed: 0, at: 0 };
let rec: Rec = IDLE;
/** The feature's context while it runs: the strip's buttons call Rust through it. */
let app: FeatureCtx | null = null;

/** "0:07", and "1:02:03" past the hour. */
function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const two = (n: number) => String(n).padStart(2, "0");
  return s >= 3600 ? `${Math.floor(s / 3600)}:${two(Math.floor((s % 3600) / 60))}:${two(s % 60)}` : `${Math.floor(s / 60)}:${two(s % 60)}`;
}

const recorded = () => (rec.elapsed + (rec.paused ? 0 : Date.now() - rec.at)) / 1000;

function size(bytes: number): string {
  const mb = bytes / 1_000_000;
  return mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : mb >= 10 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
}

/** What ended a recording that the user did not stop. */
const ENDED: Record<string, string> = {
  disk: "The disk is almost full",
  closed: "The window was closed",
  lock: "The computer was locked",
  sleep: "The computer went to sleep",
  failed: "Windows stopped the recording",
};

function savedNotice(cx: FeatureCtx, s: Saved): Notice {
  const reason = ENDED[s.why];
  // The mark is a button: it plays the film. Nothing is opened without a click.
  const play = h(
    "button",
    {
      class: `isl-ico f-recorder-play ${reason ? "warn" : "ok"}`,
      title: t("Play the recording"),
      "aria-label": t("Play the recording"),
      onclick: (e: Event) => {
        e.stopPropagation();
        void cx.call("open");
      },
    },
    icon("play", 14),
  );
  return {
    kind: "capture",
    title: t("Recording saved"),
    detail: reason ? t(reason) : `${clock(s.seconds)} · ${size(s.bytes)}`,
    mark: play,
    action: { label: t("Show in folder"), run: () => void cx.call("reveal") },
  };
}

function settingsBody(cx: FeatureCtx): HTMLElement {
  const el = h("div", { class: "f-recorder-settings" });
  const load = Promise.all([
    cx.settings.get(),
    cx.call<{ folder: string; mics: { id: string; name: string }[] }>("caps"),
    Bridge.boot(),
    Bridge.featuresList(),
  ]);
  void load.then(([s, caps, boot, features]) => {
    const sw = (key: string, on: boolean) => toggle(on, (value) => void cx.settings.patch({ [key]: value }));
    const pick = (key: string, value: string, options: [string, string][]) => {
      const select = h("select", {}, ...options.map(([v, name]) => h("option", { value: v, text: name })));
      select.value = options.some(([v]) => v === value) ? value : options[0][0];
      select.addEventListener("change", () => void cx.settings.patch({ [key]: key === "fps" ? Number(select.value) : select.value }));
      return h("label", { class: "field" }, select, icon("chevd", 14));
    };
    const folder = field({ value: String(s.folder ?? ""), placeholder: caps?.folder ?? "" });
    folder.input.addEventListener("change", () => void cx.settings.patch({ folder: folder.input.value.trim() }));
    // The key as it is bound now: the user's own, or the one the feature comes with.
    const mine = features?.find((f) => f.id === cx.id)?.shortcuts ?? [];
    const keys = (id: string) => {
      const bound = boot?.settings.shortcuts?.[`${cx.id}:${id}`];
      const text = bound ? (bound.enabled ? bound.keys : "") : (mine.find((k) => k.id === id)?.default ?? "");
      return text ? kbd(displayKeys(text)) : h("span", { class: "f-recorder-none", text: t("Not set") });
    };
    const mics: [string, string][] = [["", t("The default microphone")], ...(caps?.mics ?? []).map((m): [string, string] => [m.id, m.name])];
    clear(el);
    el.append(
      card([
        row(t("Frame rate"), pick("fps", String(s.fps ?? 60), [["60", t("60 a second")], ["30", t("30 a second")]]), t("At most. A picture is written only when the screen changes.")),
        row(
          t("Quality"),
          pick("quality", String(s.quality ?? "balanced"), [["small", t("Small")], ["balanced", t("Balanced")], ["high", t("High")]]),
          t("Small records half the pixels. Balanced and High keep every pixel; High spends more on fast motion."),
        ),
        row(t("Show the pointer"), sw("pointer", s.pointer !== false)),
        row(t("Capture border"), sw("border", s.border === true), t("The border Windows draws around what is recorded. Windows 10 always draws it.")),
      ]),
      card([
        row(t("Record the computer's sound"), sw("systemAudio", s.systemAudio !== false), t("Everything the computer plays, as its own track.")),
        row(t("Record the microphone"), sw("microphone", s.microphone === true), t("As a second track. The microphone is only opened while a recording runs.")),
        row(t("Microphone"), pick("mic", String(s.mic ?? ""), mics)),
      ]),
      card([
        row(t("Folder"), folder, t("Leave empty for a folder of the app's in your Videos.")),
        row(t("Recordings"), btn(t("Open the folder"), "secondary", () => void cx.call("folder"))),
      ]),
      card([
        row(t("Start or stop a screen recording"), keys("toggle"), t("Records the display the pointer is on.")),
        row(t("Record the window in front"), keys("window")),
        row(t("Pause or resume the recording"), keys("pause"), t("Change the keys in Settings, under Shortcuts.")),
      ]),
      h("p", {
        class: "f-recorder-note",
        text: t("Recordings are MP4 files made by Windows' own tools. If your player shows only one sound track, it plays the computer's sound; the microphone is the second track. A recording stops by itself when the disk is almost full, when the recorded window closes, or when the computer locks or goes to sleep."),
      }),
    );
  });
  return el;
}

export default defineFeature({
  id: "recorder",

  activities: [
    {
      id: "recording",
      // Before a timer: a recording is what the strip is for while it runs.
      order: -0.5,
      tick: true,
      hold: true,
      isActive: () => rec.recording,
      render() {
        const el = stripRow({
          mark: h("i", { class: rec.paused ? "f-recorder-dot paused" : "f-recorder-dot" }),
          title: rec.paused ? t("Paused") : t("Recording"),
          value: clock(recorded()),
          action: { label: t("Stop"), run: () => void app?.call("stop") },
        });
        // Pause sits before Stop: the strip has both.
        el.lastElementChild?.before(
          h("button", {
            class: "btn sm ghost",
            text: rec.paused ? t("Resume") : t("Pause"),
            onclick: (e: Event) => {
              e.stopPropagation();
              void app?.call("pause");
            },
          }),
        );
        return el;
      },
    },
  ],

  settings: settingsBody,

  start(cx) {
    app = cx;
    const show = (s: Omit<Rec, "at"> | null) => {
      rec = { ...(s ?? IDLE), at: Date.now() };
      cx.island.activityChanged("recording");
    };
    // A recording outlives the page: one that runs is shown again.
    void cx.call<Omit<Rec, "at">>("state").then((s) => {
      if (s?.recording) show(s);
    });
    const offs = [
      cx.on<Omit<Rec, "at">>("state", show),
      cx.on<Saved>("saved", (s) => cx.island.notice(savedNotice(cx, s))),
      cx.on<{ message: string }>("failed", (e) =>
        cx.island.notice({ kind: "capture", title: t("Nothing was recorded"), detail: t(e.message), icon: "record", tone: "bad" }),
      ),
      cx.on<{ track: string }>("silent", (e) => {
        if (e.track === "microphone") cx.island.notice({ kind: "accessory", title: t("The microphone is not available"), detail: t("Recording without it"), icon: "mic", tone: "warn" });
      }),
    ];
    return () => {
      offs.forEach((off) => off());
      app = null;
      show(null);
    };
  },
});
