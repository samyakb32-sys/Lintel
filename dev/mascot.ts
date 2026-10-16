// Review page for the mascot and the sounds. Not shipped.
//   npx vite --port 5199 --strictPort      then open  http://127.0.0.1:5199/dev/mascot.html
// Everything on it is the real code: the engine, the greeting, the drop canvas (in the
// frame), the recap image, the synth and the generated icons.
// For screenshots the live row also takes ?state= ?emote= ?hatch= ?dance=1 ?color=RRGGBB.

import "../src/ui/tokens.css";
import "../src/ui/components.css";
import { BOT_EMOTES, BOT_STATES, BotEngine, hexToRGB } from "../src/mascot/engine";
import { Greeting } from "../src/mascot/greeting";
import { renderShareImage } from "../src/recap/share";
import { SOUND_NAMES, Sound, type SoundName } from "../src/core/sound";
import { renderSound } from "../src/core/synth";
import type { BotEmoteName, BotStateName } from "../src/core/layout";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const el = <K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = "") => {
  const node = document.createElement(tag);
  node.className = cls;
  node.textContent = text;
  return node;
};
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const query = new URLSearchParams(location.search);
const EMOTES = Object.keys(BOT_EMOTES) as BotEmoteName[];
const SIZES: [number, string][] = [[58, "open island"], [28, "peek strip"], [22, "mini pill"]];
// A sample of the pill catalogue; the first is no pill colour at all.
const PILLS: [string, string][] = [
  ["Neutral", ""], ["Claude Code", "#E07950"], ["Codex", "#2DD4BF"], ["Gemini CLI", "#8AB4F8"],
  ["OpenCode", "#4ADE80"], ["Copilot CLI", "#818CF8"], ["Amp", "#F59E0B"], ["VS Code", "#F5F6F8"],
];
/** What the app plays with each state and emote (the island does this, not the engine). */
const SOUND_OF: Partial<Record<BotStateName | BotEmoteName, SoundName>> = {
  working: "work", thinking: "think", searching: "search", approval: "approval", question: "question",
  error: "error", finished: "finish", ratelimit: "rate", sleeping: "sleep", dizzy: "dizzy",
  love: "love", surprised: "pop", proud: "proud", wink: "wink", yawn: "yawn", annoyed: "annoyed",
};

// ── Mascots ───────────────────────────────────────────────────────────────────

interface View {
  box: HTMLElement;
  canvas: HTMLCanvasElement;
  engine: BotEngine;
  d: number;
  /** An emote this one replays every 2.8 s. */
  loop?: BotEmoteName;
  loopIn: number;
}
const views: View[] = [];
const live: View[] = [];

/** A mascot `d` across in a canvas twice that, so hops, hearts and the sparkle have room. */
function mount(host: HTMLElement, d: number, state: BotStateName = "idle", loop?: BotEmoteName): View {
  const box = el("span", "m");
  box.style.width = box.style.height = `${d}px`;
  const canvas = el("canvas");
  canvas.style.width = canvas.style.height = `${d * 2}px`;
  box.append(canvas);
  host.append(box);
  const engine = new BotEngine(state);
  engine.isMini = d < 26;
  const view = { box, canvas, engine, d, loop, loopIn: 0.4 };
  views.push(view);
  return view;
}

let color = query.has("color") ? `#${query.get("color")}` : PILLS[1][1];
const paint = () => views.forEach((v) => { v.engine.bodyColor = color ? hexToRGB(color) : null; });
const sound = (name?: SoundName) => {
  Sound.resume();
  if (name && $<HTMLInputElement>("with-sound").checked) Sound.play(name);
};
const pick = (group: HTMLElement, on: Element | null) =>
  [...group.children].forEach((c) => c.classList.toggle("on", c === on));

// The live row, with the island's own interactions (the island owns them, not the engine).
let state: BotStateName = (query.get("state") as BotStateName) || "idle";
let slaps: number[] = [];
let dizzy = 0;
/** A click annoys it; the third within 1.7 s makes it dizzy for 3.3 s. */
function slap() {
  if (dizzy) return;
  const t = performance.now();
  slaps = slaps.filter((at) => t - at < 1700);
  slaps.push(t);
  if (slaps.length < 3) {
    live.forEach((v) => v.engine.triggerEmote("annoyed"));
    sound("annoyed");
    return;
  }
  slaps = [];
  live.forEach((v) => v.engine.setState("dizzy"));
  sound("dizzy");
  dizzy = window.setTimeout(() => {
    dizzy = 0;
    live.forEach((v) => { v.engine.setState(state); v.engine.triggerEmote("happy"); });
  }, 3300);
}
for (const [d, where] of SIZES) {
  const cell = el("span");
  const v = mount(cell, d, state);
  cell.append(el("span", "", `${d} px · ${where}`));
  $("trio").append(cell);
  live.push(v);

  let love = 0;
  v.box.addEventListener("pointerenter", () => {
    sound("hover");
    love = window.setTimeout(() => { v.engine.triggerEmote("love"); sound("love"); }, 1900);
  });
  v.box.addEventListener("pointerleave", () => clearTimeout(love));
  v.box.addEventListener("pointerdown", () => { clearTimeout(love); slap(); });
}

for (const s of BOT_STATES) {
  const b = el("button", `tab${s === state ? " on" : ""}`, s);
  b.addEventListener("click", () => {
    state = s;
    pick($("states"), b);
    live.forEach((v) => v.engine.setState(s));
    sound(SOUND_OF[s]);
  });
  $("states").append(b);
}
for (const e of EMOTES) {
  const b = el("button", "btn sm secondary", e);
  b.addEventListener("click", () => { live.forEach((v) => v.engine.triggerEmote(e)); sound(SOUND_OF[e]); });
  $("emotes").append(b);
}
for (const [name, c] of PILLS) {
  const b = el("button", `dot${c === color ? " on" : ""}`);
  b.title = name;
  b.style.setProperty("--c", c || "#2a2d35");
  b.addEventListener("click", () => { color = c; pick($("colors"), b); paint(); });
  $("colors").append(b);
}
const hatch = $<HTMLInputElement>("hatch");
hatch.value = query.get("hatch") ?? "0";
const dance = $<HTMLInputElement>("dance");
dance.checked = query.has("dance");
if (query.has("emote")) live.forEach((v) => v.engine.triggerEmote(query.get("emote") as BotEmoteName));

// The sheets: every state, then every emote, at the three sizes.
function sheet(host: HTMLElement, names: readonly string[], make: (cell: HTMLElement, d: number, name: string) => void) {
  host.style.gridTemplateColumns = `64px repeat(${names.length}, minmax(0, 1fr))`;
  for (const [d] of SIZES) {
    host.append(el("b", "", `${d} px`));
    for (const name of names) {
      const cell = el("span");
      make(cell, d, name);
      if (d === 22) cell.append(el("span", "", name));
      host.append(cell);
    }
  }
}
sheet($("sheet-states"), BOT_STATES, (cell, d, name) => mount(cell, d, name as BotStateName));
sheet($("sheet-emotes"), EMOTES, (cell, d, name) => mount(cell, d, "idle", name as BotEmoteName));
paint();

// ── Greeting ──────────────────────────────────────────────────────────────────

const greeting = new Greeting();
const strip = $("strip");
const gcanvas = $<HTMLCanvasElement>("greeting");
function greet() {
  strip.classList.remove("down");
  void strip.offsetWidth; // restart the slide
  strip.classList.add("down");
  Sound.resume();
  greeting.start();
}
$("greet").addEventListener("click", greet);

// ── Frame loop ────────────────────────────────────────────────────────────────

let pointer: { x: number; y: number } | null = null;
addEventListener("pointermove", (e) => { pointer = { x: e.clientX, y: e.clientY }; }, { passive: true });

let last = performance.now();
function frame(nowMs: number) {
  const dt = Math.min(0.05, (nowMs - last) / 1000);
  last = nowMs;
  const dpr = window.devicePixelRatio || 1;
  for (const v of views) {
    const size = v.d * 2;
    const px = Math.round(size * dpr);
    if (v.canvas.width !== px) v.canvas.width = v.canvas.height = px;
    if (pointer) {
      const r = v.box.getBoundingClientRect();
      v.engine.lookX = clamp((pointer.x - r.left - v.d / 2) / 200, -1, 1);
      v.engine.lookY = -clamp((pointer.y - r.top - v.d / 2) / 200, -1, 1);
    }
    if (v.loop && (v.loopIn -= dt) < 0) {
      v.loopIn = 2.8;
      v.engine.triggerEmote(v.loop);
    }
    if (live.includes(v)) {
      v.engine.hatch = Number(hatch.value);
      v.engine.setDancing(dance.checked);
    }
    v.engine.update(dt);
    const ctx = v.canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size, size);
    v.engine.draw(ctx, size / 2, size / 2, v.d);
  }

  const gpx = Math.round(288 * dpr);
  if (gcanvas.width !== gpx) { gcanvas.width = gpx; gcanvas.height = Math.round(32 * dpr); }
  const gctx = gcanvas.getContext("2d")!;
  gctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  greeting.draw(gctx, 24, 16, 28);
  requestAnimationFrame(frame);
}
greet();
requestAnimationFrame(frame);

// ── Recap image and icons ─────────────────────────────────────────────────────

$<HTMLImageElement>("recap").src = renderShareImage({
  weekStart: new Date(2026, 9, 5), weekEnd: new Date(2026, 9, 11, 23, 59, 59),
  totalMinutes: 1124, sessionCount: 37, filesChanged: 212, linesAdded: 8410, linesRemoved: 3122,
  commandsRun: 486, questions: 12, permissionsAllowed: 64, permissionsDenied: 3,
  topAgent: "Claude Code", topProject: "meteor", busiestDay: "Wednesday", longestSessionMinutes: 143,
}, false).toDataURL("image/png");

const stamp = `?v=${Date.now()}`;
for (const [file, show] of [["icon.png", 128], ["128x128.png", 64], ["32x32.png", 32]] as const) {
  const fig = el("figure");
  const img = el("img");
  img.src = `/src-tauri/icons/${file}${stamp}`;
  img.width = img.height = show;
  fig.append(img, `${file} at ${show} px`);
  $("icons").append(fig);
}

/** The BMP entries of icon.ico (32-bit BGRA, bottom-up), drawn as they are stored. */
async function tray() {
  const bytes = await (await fetch(`/src-tauri/icons/icon.ico${stamp}`)).arrayBuffer();
  const view = new DataView(bytes);
  for (let i = 0; i < view.getUint16(4, true); i++) {
    const at = 6 + i * 16;
    const size = view.getUint8(at);
    if (size === 0 || size > 48) continue;
    const pixels = view.getUint32(at + 12, true) + 40;
    const image = new ImageData(size, size);
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        const src = pixels + ((size - 1 - y) * size + x) * 4;
        image.data.set([view.getUint8(src + 2), view.getUint8(src + 1), view.getUint8(src), view.getUint8(src + 3)], (y * size + x) * 4);
      }
    }
    for (const id of ["tray-dark", "tray-light"]) {
      const c = el("canvas");
      c.width = c.height = size;
      c.title = `${size} px`;
      // Device pixels, whatever the display scale: this is what the taskbar shows.
      c.style.width = c.style.height = `${size / (window.devicePixelRatio || 1)}px`;
      c.getContext("2d")!.putImageData(image, 0, 0);
      $(id).append(c);
    }
  }
}
void tray();

// ── Sounds ────────────────────────────────────────────────────────────────────

const vol = $<HTMLInputElement>("vol");
const setVol = () => { Sound.setVolume(Number(vol.value)); $("vol-read").textContent = Number(vol.value).toFixed(2); };
vol.addEventListener("input", setVol);
setVol();
void Sound.preload();
// The first sound needs a click somewhere: browsers start an audio context suspended.
addEventListener("pointerdown", () => Sound.resume(), { capture: true });

const buttons = new Map<SoundName, HTMLButtonElement>();
for (const name of SOUND_NAMES) {
  const b = el("button", "sound");
  b.append(name, el("canvas"), el("small", "", "…"));
  b.addEventListener("click", () => { Sound.resume(); Sound.play(name); });
  buttons.set(name, b);
  $("sounds").append(b);
}
$("play-all").addEventListener("click", () => {
  Sound.resume();
  SOUND_NAMES.forEach((name, i) => window.setTimeout(() => {
    Sound.play(name);
    buttons.get(name)!.animate([{ background: "var(--card-2)" }, {}], 600);
  }, i * 750));
});

/** Renders every sound again here, for its waveform, length and peak. */
async function inspect() {
  let ok = 0;
  let loudest = 0;
  for (const name of SOUND_NAMES) {
    const b = buttons.get(name)!;
    try {
      const data = (await renderSound(name)).getChannelData(0);
      let peak = 0;
      let end = 0;
      data.forEach((s, i) => {
        peak = Math.max(peak, Math.abs(s));
        if (Math.abs(s) > 0.0005) end = i;
      });
      if (!(peak > 0.001) || peak > 0.77) throw new Error(`peak ${peak}`);
      ok += 1;
      loudest = Math.max(loudest, peak);
      b.querySelector("small")!.textContent = `${Math.round((end / 44100) * 1000)} ms · ${(20 * Math.log10(peak)).toFixed(1)} dB`;

      const c = b.querySelector("canvas")!;
      const w = (c.width = 240);
      const h = (c.height = 56);
      const ctx = c.getContext("2d")!;
      ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue("--accent");
      const per = Math.ceil(data.length / w);
      for (let x = 0; x < w; x++) {
        let top = 0;
        for (let i = x * per; i < (x + 1) * per && i < data.length; i++) top = Math.max(top, Math.abs(data[i]));
        const bar = Math.max(1, (top / 0.25) * h);
        ctx.fillRect(x, (h - bar) / 2, 1, bar);
      }
    } catch (err) {
      b.classList.add("bad");
      b.querySelector("small")!.textContent = String(err);
    }
  }
  $("sound-stats").textContent =
    `${ok} of ${SOUND_NAMES.length} rendered in code, no audio files. Loudest peak ${(20 * Math.log10(loudest)).toFixed(1)} dBFS ` +
    `(master −14 dBFS before the 0–0.2 volume). Click a tile to hear it; the waveform is the rendered buffer.`;
}
void inspect();
