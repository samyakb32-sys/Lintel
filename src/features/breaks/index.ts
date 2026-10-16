// Breaks — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// Rust owns the clock, the rule, the fade and the screen-time file; this file draws the strip
// row of a break, the two sections (Breaks, Screen time) and the settings.

import "./style.css";
import { defineFeature, type FeatureCtx, type SectionHost } from "../../core/feature";
import { Sound } from "../../core/sound";
import { h } from "../../views/dom";
import { language, t, tn, weekdayShort } from "../../i18n/i18n";
import { icon } from "../../ui/icons";
import { btn, card, meter, row, setMeter, slider, stripRow, toggle } from "../../ui/components";

const P = "f-breaks-";

type Kind = "eye" | "move";

/** The `state` event and the answer of every break command (crates/breaks/src/lib.rs). */
interface Status {
  break: { kind: Kind; left: number; total: number } | null;
  /** Seconds at the PC until the next break of the kind; null = that reminder is off. */
  eye: number | null;
  move: number | null;
  held: boolean;
  fade: boolean;
  ev?: { phase: "soon" | "held" | "start" | "end"; kind: Kind; why: string };
}

interface Usage {
  on: boolean;
  total?: number;
  switches?: number;
  apps?: { name: string; secs: number; cat: string }[];
  /** Seconds by local date, "2026-10-11". */
  days?: Record<string, number>;
}

const NONE: Status = { break: null, eye: null, move: null, held: false, fade: false };
/** The same as WARN in crates/breaks/src/lib.rs: the notice before the screens fade. */
const WARN = 10;

let status: Status = NONE;
let at = Date.now(); // when `status` was received; a running break counts from here
let sound = true;
/** The island page's context, for the buttons of the strip row. */
let app: FeatureCtx | null = null;
/** The sections, while they are on screen: told when the state changed. */
let drawBreak: (() => void) | undefined;
let loadUsage: (() => void) | undefined;

const TITLE: Record<Kind, string> = { eye: "Rest your eyes", move: "Time to move" };
const LINE: Record<Kind, string> = { eye: "Look at something far away", move: "Stand up and stretch" };
const NAME: Record<Kind, string> = { eye: "Eye breaks", move: "Movement breaks" };
const MARK: Record<Kind, string> = { eye: "eye", move: "coffee" };
/** An hourglass: the Screen time tab. */
const HOURGLASS = "M6.5 3.5h11M6.5 20.5h11M8 3.5c0 4.5 4 5.5 4 8.5s-4 4-4 8.5M16 3.5c0 4.5-4 5.5-4 8.5s4 4 4 8.5";

const CATEGORY: Record<string, string> = {
  development: "Development",
  communication: "Communication",
  browsing: "Browsing",
  entertainment: "Entertainment",
  productivity: "Productivity",
  design: "Design",
  utilities: "Utilities",
  other: "Other",
};

// ── readings ─────────────────────────────────────────────────────────────────

const pad = (n: number) => String(n).padStart(2, "0");
/** Whole seconds the break on screen has left. */
const left = () => (status.break ? Math.max(0, Math.ceil(status.break.left - (Date.now() - at) / 1000)) : 0);
/** "0:17", "1:45". */
const clock = (s: number) => `${Math.floor(s / 60)}:${pad(s % 60)}`;
const minutes = (n: number) => t("{count} min", { count: n });
const seconds = (n: number) => t("{count} s", { count: n });
/** "3 h 12 min", "12 min", "45 s". */
function span(secs: number): string {
  const m = Math.floor(secs / 60);
  if (m >= 60) return t("{h} h {m} min", { h: Math.floor(m / 60), m: m % 60, count: Math.floor(m / 60) });
  return m >= 1 ? minutes(m) : seconds(secs);
}

export default defineFeature({
  id: "breaks",

  activities: [
    {
      // A break outranks a timer for the seconds it lasts.
      id: "break",
      order: -1,
      tick: true,
      hold: true,
      section: "break",
      isActive: () => status.break != null,
      render() {
        const b = status.break;
        if (!b) return stripRow({ icon: "eye", title: t("Breaks") });
        const el = stripRow({
          icon: MARK[b.kind],
          tone: "info",
          title: t(TITLE[b.kind]),
          value: clock(left()),
          action: { label: t("Skip"), run: () => void app?.call("skip") },
        });
        // Snooze sits before Skip: the strip has both.
        el.lastElementChild?.before(
          h("button", {
            class: "btn sm ghost",
            text: t("Snooze"),
            onclick: (e: Event) => {
              e.stopPropagation();
              void app?.call("snooze");
            },
          }),
        );
        return el;
      },
    },
  ],

  sections: [
    { id: "screen", title: "Screen time", icon: HOURGLASS, letter: "t", height: 268, build: screenPage },
    { id: "break", title: "Breaks", icon: "eye", height: 204, build: breakPage },
  ],

  settings: settingsPage,

  start(cx) {
    app = cx;
    const take = (s: Status | null) => {
      if (!s) return;
      const was = status.break != null;
      status = s;
      at = Date.now();
      const ev = s.ev;
      if (ev?.phase === "start" && sound) Sound.play("pop");
      // Eyes are elsewhere: the tone is what says the break is over.
      if (ev?.phase === "end" && ev.why === "done" && sound) Sound.play("finish");
      if (ev?.phase === "soon") {
        cx.island.notice({
          kind: "timer",
          title: tn("Break in {count} second", "Break in {count} seconds", WARN),
          detail: t("The screens will fade"),
          icon: MARK[ev.kind],
          tone: "info",
          section: "break",
          action: { label: t("Skip"), run: () => void cx.call("skip") },
        });
      }
      if (was || s.break) cx.island.activityChanged("break");
      drawBreak?.();
      if (!ev) loadUsage?.(); // the settings changed
    };
    void cx.settings.get().then((v) => (sound = v.sound !== false));
    const offs = [cx.settings.onChange((v) => (sound = v.sound !== false)), cx.on<Status>("state", take)];
    void cx.call<Status>("state").then(take);
    return () => {
      offs.forEach((off) => off());
      status = NONE;
      app = null;
    };
  },
});

// ── Breaks section ───────────────────────────────────────────────────────────

function breakPage(cx: FeatureCtx): SectionHost {
  const el = h("div", { class: `${P}brk` });
  let timer: number | undefined;
  let big: HTMLElement | undefined;
  let bar: HTMLElement | undefined;
  let ticks = 0;

  const paint = () => {
    if (big) big.textContent = clock(left());
    if (bar && status.break) setMeter(bar, 1 - left() / status.break.total);
  };

  /** One reminder while no break is on: its switch, when it is next due, and a break by hand. */
  const plan = (kind: Kind) => {
    const next = status[kind];
    const when =
      next == null ? t("Off")
      : next === 0 && status.held ? t("Waiting for a call or a full-screen app to end")
      : t("Next after {count} min at the PC", { count: Math.max(1, Math.ceil(next / 60)) });
    return card([
      h("div", { class: `${P}plan` },
        h("span", { class: `isl-ico${next == null ? "" : " info"}` }, icon(MARK[kind], 14)),
        h("b", { text: t(NAME[kind]) }),
        toggle(next != null, (on) => void cx.settings.patch({ [kind]: on }))),
      h("p", { text: when }),
      btn(t("Take one now"), "secondary", () => void cx.call("take", { kind })),
    ]);
  };

  const draw = () => {
    big = bar = undefined;
    const b = status.break;
    if (!b) {
      el.replaceChildren(h("div", { class: `${P}plans` }, plan("eye"), plan("move")));
      return;
    }
    big = h("div", { class: `${P}clock` });
    bar = meter(0, "info");
    el.replaceChildren(
      h("div", { class: `${P}now` },
        h("span", { class: "isl-ico info" }, icon(MARK[b.kind], 14)),
        h("b", { "data-shared": "title", text: t(TITLE[b.kind]) }),
        h("span", { text: t(LINE[b.kind]) })),
      big,
      bar,
      h("div", { class: `${P}actions` },
        btn(t("Snooze"), "secondary", () => void cx.call("snooze")),
        btn(t("Skip"), "ghost", () => void cx.call("skip"))),
    );
    paint();
  };

  /** Asks Rust again: the minutes to the next break move only while the person is at the PC. */
  const refresh = () =>
    void cx.call<Status>("state").then((s) => {
      if (!s || s.break || status.break) return;
      status = s;
      at = Date.now();
      draw();
    });

  return {
    el,
    show() {
      drawBreak = draw;
      draw();
      refresh();
      timer = window.setInterval(() => (status.break ? paint() : ++ticks % 20 === 0 && refresh()), 1000);
    },
    hide() {
      drawBreak = undefined;
      clearInterval(timer);
      timer = undefined;
    },
    sync() {},
  };
}

// ── Screen time section ──────────────────────────────────────────────────────

/** The category ring: one arc per category, in the token colours of style.css. */
function ring(parts: { cat: string; frac: number }[]): SVGSVGElement {
  const NS = "http://www.w3.org/2000/svg";
  const el = document.createElementNS(NS, "svg");
  el.setAttribute("viewBox", "0 0 36 36");
  el.setAttribute("class", `${P}ring`);
  el.setAttribute("aria-hidden", "true");
  // r = 100 / 2π: the circle is 100 long, so an arc's length is its percentage.
  const arc = (cls: string, len: number, from: number) => {
    const c = document.createElementNS(NS, "circle");
    const attrs = { cx: 18, cy: 18, r: 15.9155, class: cls, "stroke-dasharray": `${len} ${100 - len}`, "stroke-dashoffset": -from };
    for (const [k, v] of Object.entries(attrs)) c.setAttribute(k, String(v));
    el.append(c);
  };
  arc(`${P}track`, 100, 0);
  let from = 0;
  for (const p of parts) {
    const len = p.frac * 100;
    // A hair of space between two arcs; a sliver stays visible.
    arc(`${P}c-${p.cat}`, Math.max(0.6, len - (parts.length > 1 ? 0.9 : 0)), from);
    from += len;
  }
  return el;
}

function screenPage(cx: FeatureCtx): SectionHost {
  const el = h("div", { class: `${P}st` });
  let timer: number | undefined;
  let disarm: number | undefined;
  let armed = false; // Clear was clicked once and waits for the second click

  const load = () => void cx.call<Usage>("usage").then(draw);

  const clearBtn = () => {
    const b = btn(t(armed ? "Clear everything?" : "Clear"), armed ? "danger" : "ghost", () => {
      clearTimeout(disarm);
      if (armed) {
        armed = false;
        void cx.call<Usage>("clear").then(draw);
        return;
      }
      armed = true;
      disarm = window.setTimeout(() => {
        armed = false;
        load();
      }, 4000);
      load();
    });
    b.classList.add("sm");
    return b;
  };

  function draw(u: Usage | null) {
    if (!u) return;
    if (!u.on) {
      el.replaceChildren(
        h("div", { class: `${P}off` },
          h("b", { text: t("Screen time is off") }),
          h("p", { text: t("It counts the time each app is in front: app names and durations only, kept on this PC, never window titles.") }),
          btn(t("Turn on"), "primary", () => void cx.settings.patch({ screenTime: true }).then(load))),
      );
      return;
    }
    const apps = u.apps ?? [];
    const total = u.total ?? 0;

    // Left: the ring, the total in its middle, the three largest categories under it.
    const byCat = new Map<string, number>();
    for (const a of apps) byCat.set(a.cat, (byCat.get(a.cat) ?? 0) + a.secs);
    const parts = [...byCat].sort((a, b) => b[1] - a[1]).map(([cat, secs]) => ({ cat, frac: total ? secs / total : 0 }));
    const overview = h("div", { class: `${P}overview` },
      h("div", { class: `${P}dial` },
        ring(parts),
        h("div", {}, h("b", { text: span(total) }), h("small", { text: t("today") }))),
      ...parts.slice(0, 3).map((p) =>
        h("div", { class: `${P}key ${P}c-${p.cat}` },
          h("i"),
          h("span", { text: t(CATEGORY[p.cat] ?? "Other") }),
          h("small", { text: t("{count}%", { count: Math.round(p.frac * 100) }) }))),
    );

    // Middle: the apps by time; a long tail folds into one row.
    const top = apps.slice(0, apps.length > 6 ? 5 : 6);
    const tail = apps.slice(top.length);
    const rows = top.map((a) => ({ name: a.name, secs: a.secs, cat: a.cat }));
    if (tail.length) {
      rows.push({ name: tn("{count} other app", "{count} other apps", tail.length), secs: tail.reduce((n, a) => n + a.secs, 0), cat: "other" });
    }
    const most = Math.max(1, ...rows.map((r) => r.secs));
    const ranked = h("div", { class: `${P}apps` },
      h("div", { class: `${P}head` },
        h("span", { text: t("Apps") }),
        h("small", { text: tn("{count} switch", "{count} switches", u.switches ?? 0) })),
      ...(rows.length
        ? rows.map((r) =>
            h("div", { class: `${P}app ${P}c-${r.cat}` },
              h("i"),
              h("span", { text: r.name, title: r.name }),
              meter(r.secs / most),
              h("small", { text: span(r.secs) })))
        : [h("p", { text: t("Nothing counted yet today.") })]),
    );

    // Right: the last seven days as bars, today last.
    const narrow = new Intl.DateTimeFormat(language(), { weekday: "narrow" });
    const days = Array.from({ length: 7 }, (_, i) => {
      const d = new Date();
      d.setDate(d.getDate() - (6 - i));
      return { d, secs: u.days?.[`${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`] ?? 0 };
    });
    const tallest = Math.max(1, ...days.map((x) => x.secs));
    const week = h("div", { class: `${P}history` },
      h("div", { class: `${P}head` }, h("span", { text: t("Last 7 days") })),
      h("div", { class: `${P}week` },
        ...days.map((x, i) =>
          h("div", { class: `${P}day${i === 6 ? " today" : ""}`, title: `${weekdayShort(x.d.getDay())} · ${span(x.secs)}` },
            h("i", { style: `height:${Math.max(2, Math.round((x.secs / tallest) * 100))}%` }),
            h("small", { text: narrow.format(x.d) })))),
      clearBtn(),
    );

    el.replaceChildren(overview, ranked, week);
  }

  return {
    el,
    show() {
      loadUsage = load;
      load();
      timer = window.setInterval(load, 10_000);
    },
    hide() {
      loadUsage = undefined;
      clearInterval(timer);
      clearTimeout(disarm);
      timer = undefined;
      armed = false;
    },
    sync() {},
  };
}

// ── Settings ─────────────────────────────────────────────────────────────────

function settingsPage(cx: FeatureCtx): HTMLElement {
  const box = h("div", { class: `${P}settings` });
  void cx.settings.get().then((s) => {
    const sw = (key: string) => toggle(s[key] === true, (on) => void cx.settings.patch({ [key]: on }));
    /** A number as a slider with its reading: saved when the slider is let go. */
    const num = (key: string, label: string, min: number, max: number, step: number, unit: (n: number) => string) => {
      let v = Math.min(max, Math.max(min, Number(s[key]) || min));
      const out = h("b", { text: unit(v) });
      const set = (f: number) => {
        v = Math.round((min + f * (max - min)) / step) * step;
        out.textContent = unit(v);
      };
      const bar = slider((v - min) / (max - min), {
        label,
        onInput: set,
        onChange: (f) => {
          set(f);
          void cx.settings.patch({ [key]: v });
        },
      });
      return h("span", { class: `${P}num` }, bar, out);
    };
    let armed = false;
    const clear = btn(t("Clear"), "danger", () => {
      if (!armed) {
        armed = true;
        clear.firstElementChild!.textContent = t("Clear everything?");
        return;
      }
      armed = false;
      clear.firstElementChild!.textContent = t("Clear");
      void cx.call("clear");
    });
    box.append(
      h("div", { class: "section-title", text: t("Eye breaks") }),
      card([
        row(t("Eye breaks"), sw("eye"), t("The 20-20-20 habit: every 20 minutes, look at something far away for 20 seconds.")),
        row(t("Break every"), num("eyeEvery", t("Eye breaks"), 5, 60, 5, minutes), t("Only time at the PC counts. Being away for as long as a break counts as having taken it.")),
        row(t("Break lasts"), num("eyeSecs", t("Eye breaks"), 10, 60, 5, seconds)),
      ]),
      h("div", { class: "section-title", text: t("Movement breaks") }),
      card([
        row(t("Movement breaks"), sw("move"), t("A reminder to stand up, on its own interval.")),
        row(t("Break every"), num("moveEvery", t("Movement breaks"), 15, 120, 5, minutes)),
        row(t("Break lasts"), num("moveMins", t("Movement breaks"), 1, 10, 1, minutes)),
      ]),
      h("div", { class: "section-title", text: t("During a break") }),
      card([
        row(t("Fade the screens"), sw("fade"), t("Every screen fades to black with the countdown, after a 10-second notice on the island. Esc or a click ends it at once.")),
        row(t("Sound"), sw("sound"), t("A soft tone when a break starts and when it ends.")),
        row(t("Snooze for"), num("snooze", t("Snooze for"), 1, 15, 1, minutes)),
        row(t("Wait while on a call"), sw("holdCall"), t("A break waits while an app has the microphone open.")),
        row(t("Wait while something is full screen"), sw("holdFullscreen"), t("A film, a game or a presentation is not interrupted.")),
        row(t("Try it"),
          h("span", { class: `${P}try` },
            btn(t("Eye break"), "secondary", () => void cx.call("take", { kind: "eye" })),
            btn(t("Movement break"), "secondary", () => void cx.call("take", { kind: "move" })))),
      ]),
      h("div", { class: "section-title", text: t("Screen time") }),
      card([
        row(t("Screen time"), sw("screenTime"), t("Counts how long each app is in front and shows it on the island. Only app names and times are kept, on this PC. Window titles are never saved.")),
        row(t("History"), clear, t("Today and the six days before it.")),
      ]),
    );
  });
  return box;
}
