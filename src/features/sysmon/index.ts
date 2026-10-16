// System monitor — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// Nothing runs at import: surfaces are declared here, work begins in `start`.
//
// Rust (crates/sysmon over crates/svc-sysmon) samples only while the System
// section is on screen (`watch`) and tells every reading (`tick`). This draws
// the section: a card per metric with its history, and behind a card its page
// (graph, figures, the heaviest programs with a way to end one). Processes
// and open ports have a window of their own (tools.ts). The alerts are judged
// in Rust; this shows them as notices.
//
// The card grid and its attention rules follow the upstream file
// Sources/Vorssaint/UI/Notch/NotchSystemView.swift.

import "./style.css";
import { defineFeature, type FeatureCtx, type SectionHost } from "../../core/feature";
import { tn } from "../../i18n/i18n";
import { btn, card, iconBtn, meter, row, setMeter, slider, toggle } from "../../ui/components";
import { icon } from "../../ui/icons";
import { h } from "../../views/dom";
import { bytes, graphCeiling, percent, rate, span } from "./format";
import { drawGraph, HISTORY, type Line } from "./graph";
import { confirmBar, cpuText, killSummary, mountTools, type KillResult, type Target } from "./tools";

// ── What Rust sends ───────────────────────────────────────────────────────────

interface Volume {
  letter: string;
  label: string;
  fs: string;
  total: number;
  free: number;
  system: boolean;
  ejectable: boolean;
}

interface Snap {
  /** 0 = nothing has been read yet. */
  at: number;
  cpu: number | null;
  cores: number[];
  /** Null: this machine's GPU cannot be read (the card hides). `usage` null: not yet. */
  gpu: { name: string; usage: number | null; dedicated: number; shared: number } | null;
  mem: { total: number; used: number; available: number; committed: number; commitLimit: number; cached: number; pressure: number } | null;
  net: { down: number | null; up: number | null; totalDown: number; totalUp: number } | null;
  disk: { read: number | null; write: number | null; volumes: Volume[] } | null;
  power: {
    battery: boolean;
    ac: boolean;
    charging: boolean;
    percent: number | null;
    secondsLeft: number | null;
    rateMw: number | null;
    fullMwh: number | null;
    designMwh: number | null;
    saver: boolean;
  } | null;
  temps: { name: string; celsius: number }[];
}

/** What the hardware helper reads (the feature `hw-helper`), when it runs: rows, no card of its own. */
interface Hw {
  /** The processor, °C, and its package, W. Null when not read: never a zero. */
  cpu: number | null;
  power: number | null;
  /** `tone`: near the limit its hardware names, "warn" or "bad"; else "". */
  temps: { id: string; name: string; hardware: string; value: number; tone: string }[];
  /** RPM. */
  fans: { name: string; value: number }[];
  /** SMART: `status` is good, caution, bad or unknown; `life` in percent; `letters` like "CD". */
  drives: { name: string; letters: string; status: string; life: number | null; celsius: number | null }[];
  /** Why a reading may be missing: "driver" (not read through PawnIO), "reading" (just opened), or "" (this PC does not report it). */
  why: string;
}

type Series = "cpu" | "gpu" | "mem" | "down" | "up" | "read" | "write" | "battery";
type Hist = Record<Series, number[]>;

/** The processes of one program. */
interface ProcRow {
  name: string;
  count: number;
  cpu: number | null;
  mem: number;
  gpu: number;
  protected: boolean;
  targets: Target[];
}

interface State {
  /** Per logical processor: higher = a performance core. */
  classes: number[];
  /** The service the speed test talks to. */
  host: string;
  procs: ProcRow[] | null;
}

interface Speed {
  phase: "idle" | "latency" | "download" | "upload" | "done" | "failed";
  ms: number | null;
  down: number | null;
  up: number | null;
}

type Alert = { kind: "cpu"; percent: number } | { kind: "memory" } | { kind: "disk"; letter: string; free: number };

/** The cards that open a page. */
type Kind = "cpu" | "gpu" | "mem" | "net" | "disk" | "battery";

const DEFAULTS = {
  alertCpu: false,
  alertCpuPercent: 90,
  alertMemory: false,
  alertDisk: false,
  alertDiskFreePercent: 10,
};
type Options = typeof DEFAULTS;

// Icons only this feature draws (src/ui/icons.ts has the shared set).
const GPU = "M3 5v14M3 8h16a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2H3M12.5 9.5a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM7 17v2.5M10.5 17v2.5";
const MEMORY = "M3 8h18v8H3zM7 11v2M11 11v2M15 11v2M6.5 16v2.5M10.5 16v2.5M14.5 16v2.5M18.5 16v2.5";
const DISK = "M5 5h14a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM4 14h16M16.5 16.5h.01M13.5 16.5h.01";
const HEAT = "M12 3.5a2 2 0 0 0-2 2v8.2a4 4 0 1 0 4 0V5.5a2 2 0 0 0-2-2zM12 9v7.5";
const PLUG = "M9 3.5V8M15 3.5V8M6.5 8h11v3.5a5.5 5.5 0 0 1-11 0zM12 17v3.5";
const BACK = "M14.5 6.5L9 12l5.5 5.5";

/** At or above this share a reading is the news (the card turns amber). */
const BUSY = 0.85;
const emptyHist = (): Hist => ({ cpu: [], gpu: [], mem: [], down: [], up: [], read: [], write: [], battery: [] });

// ── The System section ────────────────────────────────────────────────────────

function buildSection(cx: FeatureCtx): SectionHost {
  const t = cx.t;
  let shown = false;
  let snap: Snap | null = null;
  let hw: Hw | null = null;
  /** The temperatures of the helper the user chose to see here. */
  let pins: string[] = [];
  let hist = emptyHist();
  let classes: number[] = [];
  let host = "";
  let detail: Kind | null = null;
  let procs: ProcRow[] | null = null;
  let devices: string[] = [];
  let addresses: { ip: string; name: string }[] = [];
  let speed: Speed = { phase: "idle", ms: null, down: null, up: null };
  /** The program the user is being asked about. */
  let pending: ProcRow | null = null;
  /** A button of the list is held: the rows are not rebuilt under the pointer. */
  let pressed = false;
  let offs: (() => void)[] = [];
  /** `watch` on and off reach Rust in the order they were asked. */
  let chain: Promise<unknown> = Promise.resolve();
  let statusTimer = 0;

  const watts = (milliwatts: number) => t("{n} W", { n: (Math.abs(milliwatts) / 1000).toFixed(1) });
  const wattHours = (mwh: number) => t("{n} Wh", { n: (mwh / 1000).toFixed(1) });
  const share = (part: number, whole: number) => (whole > 0 ? part / whole : 0);
  const ofTotal = (part: number, whole: number) => t("{used} of {total}", { used: bytes(part), total: bytes(whole) });
  const mainVolume = () => snap?.disk?.volumes.find((v) => v.system) ?? snap?.disk?.volumes[0] ?? null;

  // ── The cards ──

  interface CardView {
    el: HTMLElement;
    value: HTMLElement;
    sub: HTMLElement;
    canvas: HTMLCanvasElement | null;
    bar: HTMLElement | null;
  }

  const makeCard = (title: string, glyph: string, look: "graph" | "meter" | "plain", kind?: Kind): CardView => {
    const value = h("b", { text: "…" });
    const sub = h("small");
    const canvas = look === "graph" ? h("canvas") : null;
    const bar = look === "meter" ? meter(0) : null;
    const el = h(kind ? "button" : "div", { class: "card f-sysmon-card" }, h("span", { class: "f-sysmon-label" }, icon(glyph, 13), h("span", { text: title })), value, sub, canvas, bar);
    if (kind) el.addEventListener("click", () => open(kind));
    return { el, value, sub, canvas, bar };
  };

  const cards = {
    cpu: makeCard(t("CPU"), "cpu", "graph", "cpu"),
    gpu: makeCard(t("GPU"), GPU, "graph", "gpu"),
    mem: makeCard(t("Memory"), MEMORY, "graph", "mem"),
    net: makeCard(t("Network"), "wifi", "graph", "net"),
    disk: makeCard(t("Disk"), DISK, "meter", "disk"),
    battery: makeCard(t("Battery"), "battery", "meter", "battery"),
    temp: makeCard(t("Temperature"), HEAT, "plain"),
    devices: makeCard(t("USB devices"), PLUG, "plain"),
  };
  cards.net.el.classList.add("two");
  // The helper's rows: fans, package power, pinned temperatures.
  const hwRows = h("div", { class: "f-sysmon-hw" });
  cards.temp.el.append(hwRows);

  /** A card's figures. A reading that is not in yet leaves the last one standing. */
  const put = (c: CardView, value: string | null, sub: string, attention = false) => {
    if (value != null) c.value.textContent = value;
    c.sub.textContent = sub;
    c.el.classList.toggle("warn", attention);
  };
  const spark = (c: CardView, lines: Line[], max: number) => c.canvas && drawGraph(c.canvas, lines, max);
  const tone = (attention: boolean) => (attention ? "--warn" : "--accent");
  /** The top of a graph of rates, and never flat on the floor. */
  const ceiling = (...series: number[][]) => graphCeiling(Math.max(1024, ...series.flatMap((s) => s.slice(-HISTORY))));

  function paintCards() {
    const s = snap;
    const busy = (v: number | null | undefined) => v != null && v >= BUSY;

    put(cards.cpu, s?.cpu != null ? percent(s.cpu) : null, "", busy(s?.cpu));
    spark(cards.cpu, [{ values: hist.cpu, color: tone(busy(s?.cpu)) }], 1);

    // Known not to be readable here: the card goes. Not yet known: it waits.
    cards.gpu.el.hidden = !!s && !s.gpu;
    const gpu = s?.gpu;
    put(cards.gpu, gpu?.usage != null ? percent(gpu.usage) : null, gpu && gpu.dedicated + gpu.shared > 0 ? bytes(gpu.dedicated + gpu.shared) : "", busy(gpu?.usage));
    spark(cards.gpu, [{ values: hist.gpu, color: tone(busy(gpu?.usage)) }], 1);

    const mem = s?.mem;
    const memory = mem ? share(mem.used, mem.total) : null;
    put(cards.mem, memory != null ? percent(memory) : null, mem ? ofTotal(mem.used, mem.total) : "", busy(memory) || (mem?.pressure ?? 0) >= 2);
    spark(cards.mem, [{ values: hist.mem, color: tone(busy(memory)) }], 1);

    const net = s?.net;
    put(cards.net, net?.down != null ? t("↓ {rate}", { rate: rate(net.down) }) : null, net?.up != null ? t("↑ {rate}", { rate: rate(net.up) }) : cards.net.sub.textContent ?? "");
    spark(cards.net, [{ values: hist.down, color: "--accent" }, { values: hist.up, color: "--info" }], ceiling(hist.down, hist.up));

    const volume = mainVolume();
    const full = !!volume && share(volume.free, volume.total) < 0.1;
    put(cards.disk, volume ? bytes(volume.free) : null, volume ? t("free on {drive}", { drive: volume.letter }) : "", full);
    if (volume && cards.disk.bar) setMeter(cards.disk.bar, 1 - share(volume.free, volume.total));

    const power = s?.power;
    cards.battery.el.hidden = !!power && !power.battery;
    const level = power?.percent != null ? power.percent / 100 : null;
    const state = !power ? "" : power.charging ? t("Charging") : power.ac ? t("Plugged in") : power.secondsLeft ? t("{time} left", { time: span(power.secondsLeft) }) : t("On battery");
    put(cards.battery, level != null ? percent(level) : null, state, !!power && !power.ac && level != null && level <= 0.2);
    if (level != null && cards.battery.bar) setMeter(cards.battery.bar, level);

    // With the hardware helper: the processor's own temperature, and a row for each thing it reads.
    const degrees = (n: number) => t("{n} °C", { n: Math.round(n) });
    const rows: [string, string, string?][] = [];
    if (hw?.fans.length) rows.push([tn("Fan", "Fans", hw.fans.length), t("{n} rpm", { n: hw.fans.map((f) => Math.round(f.value)).join(" / ") })]);
    // A power that was not read is no row: never a zero in its place.
    if (hw?.power) rows.push([t("Package"), watts(hw.power * 1000)]);
    for (const x of hw?.temps ?? []) if (pins.includes(x.id)) rows.push([x.name, degrees(x.value), x.tone]);
    hwRows.replaceChildren(
      ...rows.slice(0, 3).map(([name, value, tone]) => {
        const el = fact([name, value]);
        if (tone === "warn" || tone === "bad") el.classList.add(tone);
        return el;
      }),
    );

    // The processor's own temperature when the helper reads it; without it, what Windows reads
    // unelevated; with neither, why, in a few words. No dash, and no card when nothing reads at all.
    const temps = s?.temps ?? [];
    const why = !hw || hw.cpu != null ? "" : hw.why === "driver" ? t("Needs the PawnIO driver") : hw.why === "reading" ? t("Reading…") : t("Not reported by this PC");
    const listed = rows.map(([name, value]) => t("{name}: {result}", { name, result: value })).join("\n");
    cards.temp.el.hidden = !hw && temps.length === 0;
    cards.temp.el.classList.toggle("f-sysmon-words", !!why && temps.length === 0);
    if (hw?.cpu != null) {
      put(cards.temp, degrees(hw.cpu), rows.length ? "" : t("Processor"), hw.cpu >= 90);
      cards.temp.el.title = listed;
    } else if (temps.length) {
      const hottest = temps.reduce((a, b) => (b.celsius > a.celsius ? b : a));
      const sensors = temps.length > 1 ? tn("{count} sensor", "{count} sensors", temps.length) : t("System sensor");
      // With a helper that will still read the processor, that is said under the figure Windows gives.
      put(cards.temp, t("{n} °C", { n: Math.round(hottest.celsius) }), hw?.why ? why : sensors, hottest.celsius >= 90);
      cards.temp.el.title = temps.map((x) => t("{name}: {n} °C", { name: x.name, n: Math.round(x.celsius) })).join("\n");
    } else if (hw) {
      put(cards.temp, why, "");
      cards.temp.el.title = listed;
    }

    cards.devices.el.hidden = devices.length === 0;
    put(cards.devices, String(devices.length), devices[0] ?? "");
    cards.devices.el.title = devices.join("\n");
  }

  // ── A card's page ──

  const say = (text: string, ok = true) => {
    status.textContent = text;
    status.classList.toggle("bad", !ok);
    window.clearTimeout(statusTimer);
    statusTimer = window.setTimeout(() => (status.textContent = ""), 6000);
  };

  const back = iconBtn(BACK, t("Back"), () => open(null));
  const mark = h("span", { class: "f-sysmon-mark" });
  const title = h("b");
  const figure = h("span", { class: "f-sysmon-figure" });
  const status = h("span", { class: "f-sysmon-status" });
  const legend = h("span", { class: "f-sysmon-legend" });
  const scale = h("span");
  const graph = h("canvas");
  const coreBars = h("div", { class: "f-sysmon-cores" });
  const facts = h("div", { class: "f-sysmon-kv" });
  const side = h("div", { class: "f-sysmon-side" });

  const fact = ([label, value]: [string, string]) => h("div", {}, h("span", { text: label }), h("b", { text: value, title: value }));
  const dot = (colour: string, label: string) => h("span", {}, h("i", { style: `background: var(${colour})` }), label);

  // The heaviest programs (CPU, GPU, memory).
  const procList = h("div", { class: "f-sysmon-procs" });
  procList.addEventListener("pointerdown", () => (pressed = true));
  const asking = h("div");

  const answer = async (how: "ask" | "end" | null) => {
    const what = pending;
    pending = null;
    asking.replaceChildren();
    if (!what || !how) return;
    const results = await cx.call<KillResult[]>("kill", { targets: what.targets, ask: how === "ask" });
    const said = killSummary(results, how === "ask");
    say(t("{name}: {result}", { name: what.name, result: said.text }), said.ok);
  };

  function paintProcs() {
    if (pressed || !detail) return;
    const key = detail;
    const list = procs;
    const note = (text: string) => procList.replaceChildren(h("div", { class: "f-sysmon-empty", text }));
    if (!list) return note(t("Reading…"));
    // A share needs two readings: the first only says who is there.
    if (key === "cpu" && list.every((p) => p.cpu == null)) return note(t("Measuring…"));
    if (list.length === 0) return note(key === "gpu" ? t("Nothing is using the GPU.") : t("Nothing to list."));
    procList.replaceChildren(
      ...list.map((p) => {
        const end = p.protected
          ? h("span")
          : iconBtn("close", t("End"), () => {
              pending = p;
              asking.replaceChildren(confirmBar(p.name, p.count, (how) => void answer(how)));
            });
        end.classList.add("sm");
        const value = key === "mem" ? bytes(p.mem) : key === "gpu" ? percent(p.gpu) : cpuText(p.cpu);
        return h(
          "div",
          { class: "f-sysmon-proc" },
          h("span", { class: "f-sysmon-name", text: p.name, title: p.name }),
          h("small", { text: p.count > 1 ? t("×{count}", { count: p.count }) : "" }),
          h("span", { class: "f-sysmon-num", text: value }),
          end,
        );
      }),
    );
  }

  // The network page: this machine's addresses, and the speed test.
  const addressList = h("div", { class: "f-sysmon-kv" });
  const speedText = h("span", { class: "f-sysmon-speed" });
  const speedNote = h("small");
  const speedButton = btn("", "secondary", () => {
    const running = speed.phase === "latency" || speed.phase === "download" || speed.phase === "upload";
    void cx.call(running ? "speedCancel" : "speedStart");
  });
  speedButton.classList.add("sm");
  const netSide = [
    card([h("div", { class: "f-sysmon-ctitle", text: t("This computer's addresses") }), addressList]),
    card([h("div", { class: "f-sysmon-ctitle" }, h("span", { text: t("Speed test") }), speedButton), speedText, speedNote]),
  ];

  function paintSpeed() {
    const mbps = (v: number | null) => (v == null ? "…" : v >= 100 ? String(Math.round(v)) : v.toFixed(1));
    const p = speed.phase;
    const running = p === "latency" || p === "download" || p === "upload";
    speedButton.firstElementChild!.textContent = running ? t("Cancel") : t("Run test");
    // Before it runs, and whenever it is not running: who it talks to and what it costs.
    speedNote.textContent = t("Sends and receives test data with {host} for about ten seconds, up to a few hundred megabytes on a fast connection. None of your own data is sent.", { host });
    speedNote.hidden = running || p === "done";
    speedText.textContent =
      p === "latency" ? t("Contacting {host}…", { host })
      : p === "download" ? t("Downloading: {n} Mbps", { n: mbps(speed.down) })
      : p === "upload" ? t("Uploading: {n} Mbps", { n: mbps(speed.up) })
      : p === "done" ? t("↓ {down} Mbps   ↑ {up} Mbps   {ms} ms", { down: mbps(speed.down), up: mbps(speed.up), ms: Math.round(speed.ms ?? 0) })
      : p === "failed" ? t("The test could not finish. Check the connection and try again.")
      : "";
    speedText.hidden = !speedText.textContent;
  }

  // The disk page: every volume, and eject for the ones that come out.
  let volumesDrawn = "";
  const eject = async (drive: string) => {
    const done = await cx.call<{ ok: boolean; why: string | null }>("eject", { letter: drive });
    if (done?.ok) say(t("{drive} can be unplugged.", { drive }));
    else if (done?.why === "busy") say(t("{drive} is in use. Close what is open on it and try again.", { drive }), false);
    else say(t("{drive} could not be ejected.", { drive }), false);
  };
  function paintVolumes() {
    const volumes = snap?.disk?.volumes ?? [];
    // What the drive says of its own health (SMART), when the hardware helper runs.
    const health = (v: Volume) => {
      const d = hw?.drives.find((x) => x.letters.includes(v.letter.charAt(0)));
      const said = d ? [d.life != null && t("Health {percent}", { percent: percent(d.life / 100) }), d.celsius != null && t("{n} °C", { n: Math.round(d.celsius) })] : [];
      // Poor health, or a temperature near the limit the drive names for itself.
      const hot = !!d && !!hw?.temps.some((x) => x.hardware === d.name && x.tone);
      return { text: said.filter(Boolean).join(" · "), poor: d?.status === "caution" || d?.status === "bad" || hot };
    };
    const signature = volumes.map((v) => `${v.letter}|${bytes(v.free)}|${v.ejectable}|${health(v).text}|${health(v).poor}`).join(",");
    // Rebuilt only when something shows differently: the Eject button stays put under the pointer.
    if (signature === volumesDrawn) return;
    volumesDrawn = signature;
    side.replaceChildren(
      ...volumes.map((v) => {
        const out = v.ejectable ? btn(t("Eject"), "secondary", () => void eject(v.letter)) : null;
        out?.classList.add("sm");
        const name = v.label ? t("{label} ({drive})", { label: v.label, drive: v.letter }) : v.letter;
        const bar = meter(1 - share(v.free, v.total), share(v.free, v.total) < 0.1 ? "warn" : undefined);
        const smart = health(v);
        const el = card([
          h("div", { class: "f-sysmon-ctitle" }, h("span", { text: name, title: name }), h("small", { text: v.fs }), out),
          bar,
          h("small", { text: t("{free} free of {total}", { free: bytes(v.free), total: bytes(v.total) }) }),
          smart.text ? h("small", { class: smart.poor ? "warn" : "", text: smart.text }) : null,
        ]);
        el.classList.add("f-sysmon-vol");
        return el;
      }),
    );
  }

  const batterySide = h("div", { class: "f-sysmon-kv" });

  function paintDetail() {
    const s = snap;
    if (!detail) return;
    let lines: Line[] = [];
    let max = 1;
    let top = percent(1);
    let value: string | null = null;
    let pairs: [string, string][] = [];
    let keys: HTMLElement[] = [];
    const rates = (a: Series, b: Series, first: string, second: string) => {
      lines = [{ values: hist[a], color: "--accent" }, { values: hist[b], color: "--info" }];
      max = ceiling(hist[a], hist[b]);
      top = rate(max);
      keys = [dot("--accent", first), dot("--info", second)];
    };
    const rateOf = (v: number | null | undefined) => (v == null ? "…" : rate(v));

    if (detail === "cpu") {
      lines = [{ values: hist.cpu, color: "--accent" }];
      value = s?.cpu != null ? percent(s.cpu) : null;
      const cores = s?.cores ?? [];
      const fast = Math.max(0, ...classes);
      const mixed = classes.length === cores.length && classes.some((c) => c !== fast);
      const mean = (pick: (c: number) => boolean) => {
        const of = cores.filter((_, i) => pick(classes[i]));
        return percent(of.reduce((a, b) => a + b, 0) / Math.max(1, of.length));
      };
      pairs = [[t("Threads"), String(cores.length || classes.length)]];
      if (mixed && cores.length) pairs.push([t("Performance cores"), mean((c) => c === fast)], [t("Efficiency cores"), mean((c) => c !== fast)]);
      if (mixed) keys = [dot("--accent", t("Performance")), dot("--info", t("Efficiency"))];
      // One bar per thread; the efficiency cores stand apart, in the second colour.
      if (coreBars.childElementCount !== cores.length) {
        coreBars.replaceChildren(...cores.map((_, i) => h("i", { class: mixed && classes[i] !== fast ? "e" : "" })));
        const firstSlow = mixed ? classes.findIndex((c) => c !== fast) : -1;
        if (firstSlow > 0) coreBars.children[firstSlow].classList.add("gap");
      }
      cores.forEach((v, i) => (coreBars.children[i] as HTMLElement).style.setProperty("--v", String(v)));
    } else if (detail === "gpu") {
      const gpu = s?.gpu;
      lines = [{ values: hist.gpu, color: "--accent" }];
      value = gpu?.usage != null ? percent(gpu.usage) : null;
      if (gpu) {
        pairs = [[t("Adapter"), gpu.name], [t("Memory in use"), bytes(gpu.dedicated + gpu.shared)]];
        if (gpu.dedicated > 0) pairs.push([t("Dedicated"), bytes(gpu.dedicated)]);
      }
    } else if (detail === "mem") {
      const mem = s?.mem;
      lines = [{ values: hist.mem, color: "--accent" }];
      value = mem ? percent(share(mem.used, mem.total)) : null;
      if (mem) {
        pairs = [
          [t("In use"), ofTotal(mem.used, mem.total)],
          [t("Available"), bytes(mem.available)],
          [t("Committed"), ofTotal(mem.committed, mem.commitLimit)],
          [t("Cached"), bytes(mem.cached)],
        ];
      }
    } else if (detail === "net") {
      const net = s?.net;
      rates("down", "up", t("Download"), t("Upload"));
      value = net?.down != null ? t("↓ {rate}", { rate: rate(net.down) }) : null;
      pairs = [[t("Download"), rateOf(net?.down)], [t("Upload"), rateOf(net?.up)]];
      if (net) pairs.push([t("Received"), bytes(net.totalDown)], [t("Sent"), bytes(net.totalUp)]);
      addressList.replaceChildren(...(addresses.length ? addresses.map((a) => fact([a.name, a.ip])) : [h("div", {}, h("span", { text: t("Not connected") }))]));
      paintSpeed();
    } else if (detail === "disk") {
      const volume = mainVolume();
      rates("read", "write", t("Read"), t("Write"));
      value = volume ? t("{size} free", { size: bytes(volume.free) }) : null;
      pairs = [[t("Read"), rateOf(s?.disk?.read)], [t("Write"), rateOf(s?.disk?.write)]];
      paintVolumes();
    } else {
      const power = s?.power;
      lines = [{ values: hist.battery, color: "--accent" }];
      value = power?.percent != null ? percent(power.percent / 100) : null;
      if (power) {
        pairs = [[t("Status"), power.charging ? t("Charging") : power.ac ? t("Plugged in") : t("On battery")]];
        if (power.secondsLeft) pairs.push([t("Time left"), span(power.secondsLeft)]);
        if (power.rateMw) pairs.push([power.rateMw > 0 ? t("Charging at") : t("Power draw"), watts(power.rateMw)]);
        pairs.push([t("Energy saver"), power.saver ? t("On") : t("Off")]);
        const health: [string, string][] = [];
        if (power.fullMwh && power.designMwh) health.push([t("Health"), percent(power.fullMwh / power.designMwh)]);
        if (power.fullMwh) health.push([t("Full charge"), wattHours(power.fullMwh)]);
        if (power.designMwh) health.push([t("When new"), wattHours(power.designMwh)]);
        batterySide.replaceChildren(...health.map(fact));
      }
    }

    if (value != null) figure.textContent = value;
    scale.textContent = top;
    legend.replaceChildren(...keys);
    facts.replaceChildren(...pairs.map(fact));
    drawGraph(graph, lines, max);
    if (detail === "cpu" || detail === "gpu" || detail === "mem") paintProcs();
  }

  const TITLES: Record<Kind, [string, string]> = {
    cpu: [t("CPU"), "cpu"],
    gpu: [t("GPU"), GPU],
    mem: [t("Memory"), MEMORY],
    net: [t("Network"), "wifi"],
    disk: [t("Disk"), DISK],
    battery: [t("Battery"), "battery"],
  };

  /** Opens a card's page, or goes back to the cards. */
  function open(next: Kind | null) {
    detail = next;
    procs = null;
    pending = null;
    volumesDrawn = "";
    asking.replaceChildren();
    status.textContent = "";
    figure.textContent = "";
    el.classList.toggle("detail", !!next);
    if (next) {
      const lists = next === "cpu" || next === "gpu" || next === "mem";
      title.textContent = TITLES[next][0];
      mark.replaceChildren(icon(TITLES[next][1], 16));
      coreBars.hidden = next !== "cpu";
      coreBars.replaceChildren();
      side.replaceChildren(...(lists ? [procList, asking] : next === "net" ? netSide : next === "battery" ? [card(batterySide)] : []));
      page.classList.remove("fx-in");
      void page.offsetWidth;
      page.classList.add("fx-in");
      if (next === "net") void cx.call<{ ip: string; name: string }[]>("addresses").then((list) => ((addresses = list ?? []), paint()));
    }
    paint();
    // Which programs to list, if any, travels with the watch.
    if (shown) watch();
  }

  function paint() {
    if (!shown) return;
    if (detail) paintDetail();
    else paintCards();
  }

  const watch = () => {
    chain = chain
      .then(() => cx.call<State>("watch", { on: true, detail: detail ?? "" }))
      .then((state) => {
        if (!state || !shown) return;
        classes = state.classes;
        host = state.host;
        if (state.procs) procs = state.procs;
        paint();
      });
  };

  const record = (s: Snap) => {
    const add = (key: Series, value: number | null | undefined) => {
      if (value == null) return;
      hist[key].push(value);
      if (hist[key].length > HISTORY) hist[key].shift();
    };
    add("cpu", s.cpu);
    add("gpu", s.gpu?.usage);
    add("mem", s.mem ? share(s.mem.used, s.mem.total) : null);
    add("down", s.net?.down);
    add("up", s.net?.up);
    add("read", s.disk?.read);
    add("write", s.disk?.write);
    add("battery", s.power?.percent != null ? s.power.percent / 100 : null);
  };

  const tools = (name: string, tabName: string) => {
    const b = btn(name, "secondary", () => void cx.call("window", { tab: tabName }));
    b.classList.add("sm");
    return b;
  };
  const home = h(
    "div",
    { class: "f-sysmon-home" },
    h("div", { class: "f-sysmon-grid" }, ...Object.values(cards).map((c) => c.el)),
    h("div", { class: "f-sysmon-foot" }, tools(t("Processes"), "processes"), tools(t("Open ports"), "ports")),
  );
  const chart = card([h("div", { class: "f-sysmon-ghead" }, legend, scale), graph]);
  chart.classList.add("f-sysmon-graph");
  const page = h(
    "div",
    { class: "f-sysmon-detail" },
    h("div", { class: "f-sysmon-dhead" }, back, mark, title, figure, status),
    h("div", { class: "f-sysmon-dbody" }, h("div", { class: "f-sysmon-left" }, chart, coreBars, facts), side),
  );
  back.classList.add("sm");
  const el = h("div", { class: "f-sysmon-page" }, home, page);
  const release = () => (pressed = false);

  return {
    el,
    show() {
      shown = true;
      // What was drawn last time is minutes old: the graphs start over with the sampler.
      hist = emptyHist();
      window.addEventListener("pointerup", release);
      offs = [
        () => window.removeEventListener("pointerup", release),
        cx.on<{ snap: Snap; procs?: ProcRow[]; hw?: Hw; pins?: string[] }>("tick", (said) => {
          snap = said.snap;
          hw = said.hw ?? null;
          pins = said.pins ?? [];
          record(said.snap);
          if (said.procs) procs = said.procs;
          paint();
        }),
        cx.on<string[]>("devices", (list) => {
          devices = list;
          paint();
        }),
        cx.on<Speed>("speed", (now) => {
          speed = now;
          paint();
        }),
      ];
      watch();
      void cx.call<string[]>("devices").then((list) => {
        devices = list ?? [];
        paint();
      });
      paint();
    },
    hide() {
      shown = false;
      for (const off of offs) off();
      offs = [];
      pressed = false;
      pending = null;
      asking.replaceChildren();
      // A test nobody is looking at stops moving data.
      if (speed.phase !== "idle" && speed.phase !== "done" && speed.phase !== "failed") void cx.call("speedCancel");
      speed = { phase: "idle", ms: null, down: null, up: null };
      chain = chain.then(() => cx.call("watch", { on: false }));
    },
    sync() {},
    command(c) {
      if (c !== "back" || !detail) return false;
      if (pending) void answer(null);
      else open(null);
      return true;
    },
  };
}

// ── Settings ──────────────────────────────────────────────────────────────────

function settingsPage(cx: FeatureCtx): HTMLElement {
  const t = cx.t;
  const el = h("div", { class: "f-sysmon-settings" });
  void cx.settings.get().then((stored) => {
    const now = { ...DEFAULTS, ...stored } as Options;
    const save = (patch: Partial<Options>) => void cx.settings.patch(patch);
    const item = (key: "alertCpu" | "alertMemory" | "alertDisk", label: string, hint: string) =>
      row(t(label), toggle(now[key], (on) => save({ [key]: on })), t(hint));
    /** A limit in whole steps of five between `low` and `high` percent. */
    const limit = (key: "alertCpuPercent" | "alertDiskFreePercent", label: string, low: number, high: number) => {
      const snapTo = (fraction: number) => Math.round((low + fraction * (high - low)) / 5) * 5;
      const text = h("span", { class: "f-sysmon-pct", text: percent(now[key] / 100) });
      const range = slider((now[key] - low) / (high - low), {
        label: t(label),
        onInput: (fraction) => (text.textContent = percent(snapTo(fraction) / 100)),
        onChange: (fraction) => {
          const value = snapTo(fraction);
          setMeter(range, (value - low) / (high - low));
          text.textContent = percent(value / 100);
          save({ [key]: value });
        },
      });
      return row(t(label), h("span", { class: "f-sysmon-range" }, range, text));
    };
    el.append(
      h("div", { class: "section-title", text: t("Alerts") }),
      card([
        item("alertCpu", "High CPU load", "A notice when the load stays over the limit for twelve seconds."),
        limit("alertCpuPercent", "CPU limit", 50, 100),
        item("alertMemory", "Memory nearly full", "A notice when Windows has promised programs nearly all the memory it has."),
        item("alertDisk", "Low disk space", "A notice when a drive has less free space than the limit."),
        limit("alertDiskFreePercent", "Free space limit", 5, 30),
      ]),
      h("p", {
        class: "f-sysmon-hint",
        text: t("An alert that is turned on keeps checking in the background, every two seconds. Each alert appears at most once every fifteen minutes. Low battery is announced by Battery and power."),
      }),
    );
  });
  return el;
}

// ── The feature ───────────────────────────────────────────────────────────────

export default defineFeature({
  id: "sysmon",

  sections: [{ id: "system", title: "System", icon: "cpu", letter: "u", height: 320, build: buildSection }],

  settings: settingsPage,

  windows: { tools: mountTools },

  // The command bar filters these by what is typed.
  commands: (_query, cx) => [
    {
      id: "sysmon:processes",
      title: cx.t("Processes"),
      subtitle: cx.t("See what runs, end what hangs"),
      icon: "cpu",
      keywords: ["kill", "end", "task", "process"],
      run: () => void cx.call("window", { tab: "processes" }),
    },
    {
      id: "sysmon:ports",
      title: cx.t("Open ports"),
      subtitle: cx.t("What listens on this computer"),
      icon: "link",
      keywords: ["port", "listen", "tcp", "localhost"],
      run: () => void cx.call("window", { tab: "ports" }),
    },
  ],

  start: (cx) =>
    // An alert Rust judged due: the island says it, and a click opens the section.
    cx.on<Alert>("alert", (a) => {
      const said =
        a.kind === "cpu" ? { title: cx.t("CPU load is high"), detail: cx.t("{percent}% for a while", { percent: a.percent }), icon: "cpu" }
        : a.kind === "memory" ? { title: cx.t("Memory is nearly full"), icon: MEMORY }
        : { title: cx.t("{drive} is nearly full", { drive: a.letter }), detail: cx.t("{size} free", { size: bytes(a.free) }), icon: DISK };
      cx.island.notice({ kind: "accessory", tone: "warn", section: "system", ...said });
    }),
});
