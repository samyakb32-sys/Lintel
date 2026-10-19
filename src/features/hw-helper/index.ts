// Hardware sensors — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// Nothing runs at import: surfaces are declared here.
//
// The feature has one surface of its own, its Settings page: what the helper
// is and what it uses (the consent), the switch that starts it (one Windows
// prompt), the driver offer, which temperatures the System section shows, and
// fan control where this PC allows it. The readings
// themselves are rows of the System section (src/features/sysmon).
//
// A driver installed while the helper runs needs nothing from the user: the
// helper opens the hardware again by itself and the page shows what came.
//
// Rust (crates/hw-helper over crates/svc-hw) owns the helper; this page asks
// for its `status` every two seconds while it is on screen, and never else.

import "./style.css";
import { BRAND } from "../../core/brand";
import { defineFeature, type FeatureCtx } from "../../core/feature";
import { btn, card, setMeter, slider, toggle } from "../../ui/components";
import { h } from "../../views/dom";

// ── What Rust sends ───────────────────────────────────────────────────────────

interface Named {
  id: string;
  name: string;
  hardware: string;
  value: number;
  /** A fan whose speed can be set from here. */
  ctl: boolean;
  /** A temperature near the limit its hardware names: "warn", "bad", or "". */
  tone: string;
}

interface FanState {
  mode: "auto" | "manual" | "curve";
  level: number;
  /** What the fans are told now, in percent; null while the motherboard has them. */
  applied: number | null;
  reason: string | null;
  controllable: number;
}

interface Status {
  running: boolean;
  error: string | null;
  /** PawnIO is installed; and the running helper reads through it. */
  driver: boolean;
  inUse: boolean;
  helperFound: boolean;
  winget: boolean;
  installing: boolean;
  installNote: string | null;
  /** `why` a reading may be missing: "driver" (not read through PawnIO), "reading" (just opened), or "" (this PC does not report it). */
  snap: { cpu: number | null; power: number | null; temps: Named[]; fans: Named[]; fan: FanState; why: string } | null;
}

const DEFAULTS = { consent: false, pins: [] as string[], fanLevel: 60, fanCurve: [20, 30, 50, 75, 100] };

/** Where the five speeds of the curve stand, in °C (crates/hw-helper has the same). */
const CURVE_AT = [40, 50, 60, 70, 80];
/** The helper never goes under this, so neither do the sliders. */
const FLOOR = 20;
const fraction = (percent: number) => (percent - FLOOR) / (100 - FLOOR);
const stepped = (f: number) => FLOOR + Math.round((f * (100 - FLOOR)) / 5) * 5;

// ── Settings › Hardware sensors ───────────────────────────────────────────────

function settingsPage(cx: FeatureCtx): HTMLElement {
  const t = cx.t;
  const app = BRAND.name;
  let status: Status | null = null;
  let opts = { ...DEFAULTS };
  /** Windows is asking, or the helper is starting. */
  let busy = false;
  /** The switch was pressed for the first time: the question is on screen. */
  let asking = false;
  /** A control of the lists is held: they are not rebuilt under the pointer. */
  let pressed = false;
  let fanNote = "";

  /** A settings row whose hint changes. */
  const liveRow = (label: string, control: HTMLElement) => {
    const hint = h("span", { class: "row-hint" });
    return { el: h("div", { class: "row" }, h("span", { class: "row-label", text: label }), hint, control), hint };
  };
  const flip = (sw: HTMLElement, on: boolean) => sw.setAttribute("aria-checked", String(on));
  const degrees = (n: number) => t("{n} °C", { n: Math.round(n) });

  // ── The helper: consent, switch, driver ──

  const about = h("p", {
    class: "f-hw-helper-about",
    text: t(
      "Processor temperatures, fan speeds, processor power and drive health can only be read by a program with administrator rights. {app} keeps that part separate: a small helper starts when you turn this on, after Windows asks your permission, and closes by itself when {app} no longer needs it. It reads the hardware through the LibreHardwareMonitor library and the PawnIO driver. It talks only to {app} and sends nothing over the network.",
      { app },
    ),
  });

  const launch = async () => {
    asking = false;
    busy = true;
    paint();
    status = (await cx.call<Status>("launch")) ?? status;
    busy = false;
    if (status?.running) opts.consent = true;
    paint();
  };
  const power = toggle(false, (on) => {
    if (!on) void cx.call<Status>("quit").then(got);
    // The first time, the question; after that the switch is the answer.
    else if (opts.consent) void launch();
    else asking = true;
    paint();
  });
  const powerRow = liveRow(t("Read the hardware sensors"), power);
  const question = card(
    [
      h("span", { text: t("Start the helper as administrator? Windows asks for your permission next.") }),
      btn(t("Not now"), "ghost", () => {
        asking = false;
        paint();
      }),
      btn(t("Allow and start"), "primary", () => void launch()),
    ],
    true,
  );
  question.classList.add("f-hw-helper-ask");

  // One of the three, never two: the button while it is missing, the progress while winget runs, the word once it is there.
  const install = btn(t("Install with winget"), "secondary", () => void cx.call<Status>("installDriver").then(got));
  const installing = h("span", { class: "f-hw-helper-busy", text: t("Installing…") });
  const installed = h("span", { class: "f-hw-helper-ok", text: t("Installed") });
  const driverRow = liveRow(t("PawnIO driver"), h("span", { class: "f-hw-helper-slot" }, installed, installing, install));

  // ── Which temperatures the System section shows ──

  const tempTitle = h("div", { class: "section-title", text: t("Temperatures in the System section") });
  const tempList = card([]);
  tempList.classList.add("f-hw-helper-list");
  /** Why the processor is not in the list. */
  const tempGap = h("p", { class: "hint" });

  const pin = (id: string, on: boolean) => {
    opts.pins = on ? [...opts.pins.filter((p) => p !== id), id] : opts.pins.filter((p) => p !== id);
    void cx.settings.patch({ pins: opts.pins });
  };

  // ── Fans ──

  const fanTitle = h("div", { class: "section-title", text: t("Fan speeds") });
  const fanList = card([]);
  fanList.classList.add("f-hw-helper-list");
  const fanGap = h("p", { class: "hint", text: t("This PC does not report its fan.") });

  const applyFan = async (mode: string) => {
    const done = await cx.call<{ fan?: FanState; error?: string }>("fan", { mode, level: opts.fanLevel, curve: opts.fanCurve });
    fanNote = done?.error ?? "";
    if (done?.fan && status?.snap) status.snap.fan = done.fan;
    paint();
  };

  const mode = h("select");
  for (const [value, label] of [["auto", t("Automatic")], ["manual", t("Fixed speed")], ["curve", t("By temperature")]]) mode.append(h("option", { value, text: label }));
  mode.addEventListener("change", () => void applyFan(mode.value));

  /** A slider from the floor to 100 % in steps of five, with its figure. */
  const speed = (label: string, percent: number, onChange: (percent: number) => void) => {
    const text = h("span", { class: "f-hw-helper-pct" });
    const set = (value: number) => {
      setMeter(bar, fraction(value));
      text.textContent = t("{percent}%", { percent: value });
    };
    const bar: HTMLElement = slider(fraction(percent), {
      label,
      onInput: (f) => (text.textContent = t("{percent}%", { percent: stepped(f) })),
      onChange: (f) => {
        set(stepped(f));
        onChange(stepped(f));
      },
    });
    set(percent);
    return { el: h("div", { class: "row" }, h("span", { class: "row-label", text: label }), h("span", { class: "f-hw-helper-range" }, bar, text)), set };
  };

  const manual = speed(t("Speed"), opts.fanLevel, (percent) => {
    opts.fanLevel = percent;
    void applyFan("manual");
  });
  // Warmer never means slower: moving one point takes the others along.
  const curve = CURVE_AT.map((at, i) =>
    speed(t("At {n} °C", { n: at }), opts.fanCurve[i], (percent) => {
      opts.fanCurve = opts.fanCurve.map((level, j) => (j < i ? Math.min(level, percent) : j > i ? Math.max(level, percent) : percent));
      curve.forEach((c, j) => c.set(opts.fanCurve[j]));
      void applyFan("curve");
    }),
  );
  const note = h("p", { class: "f-hw-helper-note" });
  const modeRow = h("div", { class: "row" }, h("span", { class: "row-label", text: t("Fan control") }), h("label", { class: "field" }, mode));
  const control = card([modeRow, manual.el, ...curve.map((c) => c.el)]);
  const limits = h("p", {
    class: "hint",
    text: t("The helper always has the final say: fans never go below 20 %, they run at full speed from 85 °C, and they return to automatic if {app} closes or stops answering for ten seconds.", { app }),
  });
  const uncontrolled = h("p", { class: "hint", text: t("This PC does not let its fans be set from here. Their speeds are still shown.") });

  const el = h("div", { class: "f-hw-helper-page" }, card(about), card([powerRow.el, driverRow.el]), question, tempTitle, tempList, tempGap, fanTitle, fanList, fanGap, uncontrolled, control, note, limits);

  // ── Drawing ──

  // `tone`: a temperature near its hardware's own limit is amber ("warn") or red ("bad").
  const reading = (name: string, hardware: string, value: string, control?: HTMLElement, hint?: string, tone = "") =>
    h("div", { class: "row" }, h("span", { class: "row-label", text: name, title: hardware }), h("span", { class: "row-hint", text: hint ?? hardware }), h("span", { class: "f-hw-helper-slot" }, h("b", { class: tone === "warn" || tone === "bad" ? tone : "", text: value }), control));

  function paint() {
    const s = status;
    const running = !!s?.running;
    const snap = running ? s!.snap : null;

    flip(power, running || busy);
    power.disabled = busy || !s?.helperFound;
    powerRow.hint.textContent = !s ? ""
      : !s.helperFound ? t("The helper ({file}) is not installed beside {app}.", { app, file: `${BRAND.slug}-hw.exe` })
      : busy ? t("Waiting for Windows…")
      : s.error ? s.error
      : running ? t("The helper is running.")
      : t("Off. Nothing runs as administrator.");
    powerRow.hint.classList.toggle("f-hw-helper-bad", !!s && (!s.helperFound || (!busy && !running && !!s.error)));
    question.hidden = !asking;

    installing.hidden = !s?.installing;
    installed.hidden = !s || s.installing || !s.driver;
    install.hidden = !s || s.installing || s.driver;
    install.disabled = !s?.winget;
    driverRow.hint.textContent = !s ? ""
      : s.installing ? t("winget is installing PawnIO. Windows asks for your permission.")
      // The helper takes it up by itself at its next reading, a moment later. Longer than that, and Windows has not started the driver.
      : s.driver && running && !s.inUse ? t("Installed, but it does not answer yet. If this stays, restart Windows.")
      : s.driver ? t("The helper reads processor temperatures and motherboard fans through it.")
      : s.installNote ? s.installNote
      : s.winget ? t("Without it the helper reads graphics cards and drives only. The button runs: winget install namazso.PawnIO")
      : t("Without the PawnIO driver the helper can only read graphics cards and drives. Get it from pawnio.eu, or install winget (App Installer) first.");

    // The lists hold switches: not rebuilt while one is under the pointer.
    if (!pressed) {
      const temps = snap?.temps ?? [];
      tempTitle.hidden = !snap;
      tempList.hidden = temps.length === 0;
      tempList.replaceChildren(
        ...temps.map((x) => reading(x.name, x.hardware, degrees(x.value), toggle(opts.pins.includes(x.id), (on) => pin(x.id, on)), undefined, x.tone)),
      );
      const fans = snap?.fans ?? [];
      // Nothing to show is said, not left blank; but not before the driver is in use and a few readings are in.
      fanGap.hidden = !snap || fans.length > 0 || snap.why !== "";
      fanTitle.hidden = !snap || (fans.length === 0 && !snap.fan.controllable && fanGap.hidden);
      fanList.hidden = fans.length === 0;
      fanList.replaceChildren(
        ...fans.map((x) => reading(x.name, x.hardware, t("{n} rpm", { n: Math.round(x.value) }), undefined, x.ctl || !snap?.fan.controllable ? undefined : t("Cannot be set on this PC."))),
      );
    }

    tempGap.textContent = !snap ? ""
      : snap.why === "driver" ? t("Processor temperature needs the PawnIO driver.")
      : snap.cpu != null ? ""
      : snap.why === "reading" ? t("Reading the sensors…")
      : t("This PC does not report its processor temperature.");
    tempGap.hidden = !tempGap.textContent;

    const fan = snap?.fan;
    const can = !!fan && fan.controllable > 0;
    uncontrolled.hidden = !fan || can || (snap?.fans.length ?? 0) === 0;
    control.hidden = limits.hidden = !can;
    if (fan && can) {
      if (document.activeElement !== mode) mode.value = fan.mode;
      manual.el.hidden = mode.value !== "manual";
      for (const c of curve) c.el.hidden = mode.value !== "curve";
    }
    note.textContent = !fan || !can ? ""
      : fanNote ? fanNote
      : fan.reason === "thermal" ? t("A sensor reached 85 °C: the fans run at full speed until it cools down.")
      : fan.mode !== "auto" && fan.applied != null ? t("The fans are set to {percent}%.", { percent: Math.round(fan.applied) })
      : fan.reason === "heartbeat" ? t("The fans went back to automatic: {app} stopped answering the helper.", { app })
      : fan.reason === "temperature" ? t("The fans went back to automatic: no temperature could be read.")
      : fan.reason === "control" ? t("The fans went back to automatic: this PC refused the speed.")
      : "";
    note.hidden = !note.textContent;
    note.classList.toggle("f-hw-helper-bad", !!fanNote);
  }

  function got(s: Status | null) {
    if (s) status = s;
    paint();
  }
  const refresh = () => cx.call<Status>("status").then(got);

  // ── While the page is on screen ──

  el.addEventListener("pointerdown", () => (pressed = true));
  const release = () => {
    pressed = false;
    paint();
  };
  window.addEventListener("pointerup", release);
  const offChanged = cx.on("changed", () => void refresh());
  let shown = false;
  const timer = window.setInterval(() => {
    if (el.isConnected) {
      shown = true;
      if (!busy) void refresh();
    } else if (shown) {
      // Another page took its place: nothing is asked any more.
      window.clearInterval(timer);
      window.removeEventListener("pointerup", release);
      offChanged();
    }
  }, 2000);

  void cx.settings.get().then((stored) => {
    opts = { ...DEFAULTS, ...(stored as Partial<typeof DEFAULTS>) };
    manual.set(opts.fanLevel);
    curve.forEach((c, i) => c.set(opts.fanCurve[i] ?? DEFAULTS.fanCurve[i]));
    void refresh();
  });
  paint();
  return el;
}

// ── The feature ───────────────────────────────────────────────────────────────

export default defineFeature({
  id: "hw-helper",
  settings: settingsPage,
});
