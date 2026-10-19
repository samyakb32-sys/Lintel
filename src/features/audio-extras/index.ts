// Audio extras — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// Nothing runs at import: surfaces are declared here, work begins in `start`.
//
// Rust (crates/audio-extras) listens to what the computer plays, and only while
// this side says somebody is looking: `watch { bars, mascot }` whenever the
// playing bars or the mascot come on screen or leave it. `levels` then moves
// the bars, wherever a feature has put some. `beat` is for the mascot, whose
// drawing is not ours: the island listens to that one. `volume` is a volume key
// applied by Rust (fine steps, replace mode), shown as the volume notice.

import "./style.css";
import { defineFeature, type FeatureCtx } from "../../core/feature";
import { card, row, toggle } from "../../ui/components";
import { h } from "../../views/dom";

const DEFAULTS = { liveBars: true, bounce: true, fineSteps: false, replaceHud: false };
type Options = typeof DEFAULTS;

/** On the bars that follow the music: their own motion stops (style.css). */
const LIVE = "f-audio-extras-live";
/** The speaker with one wave (as the volume notice of `huds-power` draws it). */
const VOLUME_LOW = "M4 9.5h3.5L12 6v12l-4.5-3.5H4zM15.5 9a4.2 4.2 0 0 1 0 6";

let opts: Options = DEFAULTS;
/** What Rust was last told is on screen. */
let seen = { bars: false, mascot: false };
/** "Reduce motion" in Windows: the bars keep still. Read in `start`. */
let still: MediaQueryList | null = null;

/** Which band a bar reads when there are fewer bars than bands: evenly, not all in the bass (`barIndex`, NotchAudioLevelSupport.swift). */
const bandOf = (bar: number, bars: number, bands: number) => Math.min(bands - 1, Math.floor(((bar + 0.5) / bars) * bands));

function release(el: HTMLElement) {
  el.classList.remove(LIVE);
  for (const bar of el.children) (bar as HTMLElement).style.transform = "";
}

/** The heights of every playing bar on the island; `null` gives them back their own motion. */
function paint(levels: number[] | null) {
  for (const el of document.querySelectorAll<HTMLElement>(`.${LIVE}`)) {
    if (!levels || el.classList.contains("paused")) release(el);
  }
  if (!levels) return;
  // debt: the bars are found on the page (whoever shows something playing draws `.bars`): components.ts has no way to hand them a live source.
  for (const el of document.querySelectorAll<HTMLElement>("#island .bars:not(.paused)")) {
    el.classList.add(LIVE);
    const bars = el.children;
    for (let i = 0; i < bars.length; i++) {
      const level = levels[bandOf(i, bars.length, levels.length)] ?? 0;
      (bars[i] as HTMLElement).style.transform = `scaleY(${(0.2 + 0.8 * level).toFixed(3)})`;
    }
  }
}

/** Tells Rust when the playing bars or the mascot came on screen or left it. */
function look(cx: FeatureCtx, always = false) {
  // debt: read off the page (#island[data-mode], #bot-canvas): IslandApi cannot say whether the strip is out or the mascot shown.
  const island = document.getElementById("island");
  const mode = island?.dataset.mode ?? "hidden";
  const out = mode !== "hidden";
  // The strip's row stays in the page behind the open island, and the open island's views behind the strip.
  const playing = mode === "open" ? ".bars:not(.paused):not(#strip *)" : "#strip .bars:not(.paused)";
  const next = {
    bars: out && opts.liveBars && !still?.matches && island?.querySelector(playing) != null,
    mascot: out && opts.bounce && document.getElementById("bot-canvas")?.style.opacity === "1",
  };
  if (!always && next.bars === seen.bars && next.mascot === seen.mascot) return;
  // Rust keeps the sound a moment longer, for a skip: the bars do not wait for it.
  if (seen.bars && !next.bars) paint(null);
  seen = next;
  void cx.call("watch", seen);
}

function onVolume(cx: FeatureCtx, v: { level: number; mute: boolean }) {
  const muted = v.mute || v.level < 0.005;
  cx.island.notice({
    kind: "volume",
    title: cx.t(muted ? "Muted" : "Volume"),
    level: muted ? 0 : v.level,
    icon: muted ? "mute" : v.level < 0.34 ? VOLUME_LOW : "volume",
  });
}

function settingsPage(cx: FeatureCtx): HTMLElement {
  const el = h("div", { class: "f-audio-extras-settings" });
  void cx.settings.get().then((stored) => {
    const now = { ...DEFAULTS, ...stored } as Options;
    const item = (key: keyof Options, label: string, hint: string) =>
      row(cx.t(label), toggle(now[key], (on) => void cx.settings.patch({ [key]: on })), cx.t(hint));
    el.append(
      h("div", { class: "section-title", text: cx.t("Listening") }),
      card([
        item("liveBars", "Bars follow the music", "The playing bars show the real sound, from bass to treble. Every app that sounds moves them."),
        item("bounce", "Mascot bounces to the beat", "While music plays the mascot lands on the song's beats. Finding the tempo takes about ten seconds of the song."),
      ]),
      h("p", {
        class: "f-audio-extras-note",
        text: cx.t("The sound your computer plays is checked a few seconds at a time, only while the bars or the mascot are on screen. Nothing is recorded or sent."),
      }),
      h("div", { class: "section-title", text: cx.t("Volume keys") }),
      card([
        item("fineSteps", "Finer volume steps", "Shift with a volume key moves the level by 1/64 instead of 2 %."),
        item(
          "replaceHud",
          "Replace the Windows volume flyout",
          "The volume keys change the volume here, and the island shows the level instead of the Windows pop-up. If you change the volume another way, such as the Windows slider, the pop-up still appears.",
        ),
      ]),
    );
  });
  return el;
}

export default defineFeature({
  id: "audio-extras",

  settings: settingsPage,

  start: (cx) => {
    still = matchMedia("(prefers-reduced-motion: reduce)");
    const apply = (stored: object) => {
      opts = { ...DEFAULTS, ...stored } as Options;
      look(cx);
    };
    // Nothing is polled: the island's mode, what the strip holds and the mascot's canvas say when to look again.
    const watcher = new MutationObserver(() => look(cx));
    const island = document.getElementById("island");
    const mascot = document.getElementById("bot-canvas");
    if (island) watcher.observe(island, { attributes: true, attributeFilter: ["data-mode"], childList: true, subtree: true });
    if (mascot) watcher.observe(mascot, { attributes: true, attributeFilter: ["style"] });
    const offs = [
      cx.settings.onChange(apply),
      cx.on<number[] | null>("levels", (levels) => {
        if (!levels || seen.bars) paint(levels);
      }),
      cx.on<{ level: number; mute: boolean }>("volume", (v) => onVolume(cx, v)),
    ];
    void cx.settings.get().then((stored) => {
      opts = { ...DEFAULTS, ...stored } as Options;
      // Once whatever is seen: a page that was reloaded starts Rust from here.
      look(cx, true);
    });

    return () => {
      watcher.disconnect();
      for (const off of offs) off();
      paint(null);
      seen = { bars: false, mascot: false };
      opts = DEFAULTS;
      still = null;
    };
  },
});
