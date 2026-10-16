// The mascot outside the island: the icon in the notification area, the taskbar button of METEOR V1's
// own windows and the icon of METEOR V1's shortcuts. The island's page is always loaded, so it draws
// them with the engine the island's mascot is drawn with (mascot/engine.ts) and hands the pixels
// to the host (src-tauri/src/tray.rs, brand_icon.rs).
//
// Idle means idle. While nothing happens the tray holds one still picture and nothing of this
// file runs but a blink every several seconds; it moves, a few frames a second, only while the
// island's mascot moves for a reason, and a frame is sent only when it differs from the one shown.
// The windows' and the shortcuts' icon are drawn at the start and when the colour changes.
//
// The rules are plain functions: tests/tray-mascot.test.mjs.

import { Bridge } from "../core/bridge";
import { dibEntry, encodeIco, type IcoEntry } from "../core/ico";
import type { BotStateName } from "../core/layout";
import { pillColor } from "../core/pill-colors";
import { pillDefinition } from "../core/pills";
import { State } from "../core/state";
import { BotEngine, hexToRGB, wipe, type RGB } from "../mascot/engine";

// ── The rules ─────────────────────────────────────────────────────────────────

/** The pill whose colour is the product's own outside the island: VS Code. */
export const BRAND_PILL = "integration_claude";
/** The product's own colour until the user gives that pill one: the accent (--accent), which the icon in the program carries. */
export const BRAND_DEFAULT = "#8EA2FF";

/**
 * The colour of the mascot wherever it stands for the product (the tray, a window's taskbar
 * button, a shortcut): the one the user gave the VS Code pill, else the accent. No state
 * changes it: as in the island, a state colours the bar of light, never the body.
 */
export function brandColor(pillColors: unknown): string {
  const def = pillDefinition(BRAND_PILL)!;
  return pillColor(def.id, BRAND_DEFAULT, pillColors);
}

/** It is the default colour: the icon the program itself carries is that one. */
export const brandIsDefault = (pillColors: unknown): boolean => brandColor(pillColors) === brandColor(null);

/** The side of a notification icon, by the display's scale: 16 px at 100 %, 20, 24, and 32 at 200 %. */
export function traySize(scale: number): number {
  const want = 16 * (Number.isFinite(scale) && scale > 0 ? scale : 1);
  return [16, 20, 24, 32].reduce((best, s) => (Math.abs(s - want) <= Math.abs(best - want) ? s : best));
}

export interface TrayNow {
  /** Settings › Mascot "Animate the tray icon". */
  animate: boolean;
  /** Settings › Mascot "Show the mascot". */
  mascot: boolean;
  /** Tray › Pause. */
  paused: boolean;
  /** The computer is locked or was asleep a moment ago. */
  away: boolean;
  /** What the island's mascot is doing. */
  state: BotStateName;
  /** A notice is up on the island. */
  notice: boolean;
}

/** The states in which the island's mascot moves for a reason: an agent works, or waits for the user. */
const BUSY: ReadonlySet<BotStateName> = new Set(["working", "thinking", "searching", "approval", "question"]);

/** off: nothing is sent. still: one picture and no frames. moving: a few frames a second. */
export function trayMode(o: TrayNow): "off" | "still" | "moving" {
  if (o.paused) return "off";
  if (o.away || !o.animate || !o.mascot) return "still";
  return o.notice || BUSY.has(o.state) ? "moving" : "still";
}

/** The face of the tray's mascot: the island's, or the resting one when the tray is not to follow it. */
export const trayFace = (o: TrayNow): BotStateName => (o.animate && o.mascot ? o.state : "idle");

/** At rest and alive, with someone there to see it, it blinks now and then. */
export const trayBlinks = (o: TrayNow): boolean =>
  o.animate && o.mascot && !o.away && o.state === "idle" && trayMode(o) === "still";

/** A frame is sent only when it is not the one the tray shows. */
export function frameChanged(shown: ArrayLike<number> | null, next: ArrayLike<number>): boolean {
  if (!shown || shown.length !== next.length) return true;
  for (let i = 0; i < next.length; i++) if (shown[i] !== next[i]) return true;
  return false;
}

/** Below this size the engine's drawing does not hold (a bar of 1.5 px with a glow, a bob of a fraction of a pixel). */
export const ENGINE_MIN = 20;

/** The visor on whole pixels, as scripts/gen-icons.mjs measures it for the small sizes of the program's icon. */
export function smallVisor(size: number) {
  const c = size / 2;
  const barH = Math.max(2, Math.round(size * 0.074));
  const bandH = barH + 2 * Math.round((size * 0.28 - barH) / 2);
  const bandW = 2 * Math.round(size * 0.35);
  const bandY = Math.round(c - size * 0.1 - bandH / 2);
  return { bandX: c - bandW / 2, bandY, bandW, bandH, barY: bandY + (bandH - barH) / 2, barW: 2 * Math.round(size * 0.18), barH };
}

/**
 * The bar of light in that visor at `t` seconds, on whole pixels: it scans while an agent works,
 * pulses at full width while one waits, is dim asleep, and is not there for the frame of a blink.
 */
export function smallBar(state: BotStateName, t: number, size: number, shut: boolean): { x: number; w: number; alpha: number } | null {
  const v = smallVisor(size);
  const [left, room] = [v.bandX + 1, v.bandW - 2];
  if (state === "working" || state === "thinking" || state === "searching") {
    const w = v.barW - 2;
    return { x: left + Math.round((room - w) * (0.5 + 0.5 * Math.sin((t * Math.PI * 2) / 1.1))), w, alpha: 1 };
  }
  if (state === "approval" || state === "question") {
    return { x: left + 1, w: room - 2, alpha: 0.45 + 0.55 * (0.5 + 0.5 * Math.sin((t * Math.PI * 2) / 1.2)) };
  }
  if (shut) return null;
  return { x: size / 2 - v.barW / 2, w: v.barW, alpha: state === "sleeping" || state === "ratelimit" ? 0.5 : 1 };
}

// ── The drawing ───────────────────────────────────────────────────────────────

const WHITE: RGB = [1, 1, 1];
const mix = (a: RGB, b: RGB, t: number): RGB => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
const css = (c: RGB, a = 1) => `rgba(${Math.round(c[0] * 255)},${Math.round(c[1] * 255)},${Math.round(c[2] * 255)},${a})`;

/** What the bar turns when a state wants the user, by the design tokens, as in the engine. */
const TONE: Partial<Record<BotStateName, string>> = { approval: "--warn", question: "--info", error: "--bad", finished: "--ok" };

/**
 * 16 px: the body and the visor only, on whole pixels. The engine is not asked at this size; the
 * bar says the state by its colour and by `smallBar`.
 */
function drawSmall(ctx: CanvasRenderingContext2D, size: number, body: RGB, state: BotStateName, t: number, shut: boolean) {
  const tokens = getComputedStyle(document.documentElement);
  const token = (name: string, fallback: string) => hexToRGB(tokens.getPropertyValue(name).trim() || fallback);
  const black = token("--bg", "#000");
  const [c, r, v] = [size / 2, size * 0.46, smallVisor(size)];
  const shade = ctx.createLinearGradient(0, c - r, 0, c + r);
  shade.addColorStop(0, css(mix(body, WHITE, 0.3)));
  shade.addColorStop(0.55, css(body));
  shade.addColorStop(1, css(mix(body, black, 0.32)));
  ctx.fillStyle = shade;
  ctx.beginPath();
  ctx.arc(c, c, r, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = css(mix(black, token("--panel", "#0b0c0e"), 0.6), 0.94);
  ctx.beginPath();
  ctx.roundRect(v.bandX, v.bandY, v.bandW, v.bandH, 1);
  ctx.fill();
  const bar = smallBar(state, t, size, shut);
  if (!bar) return;
  ctx.globalAlpha = bar.alpha;
  ctx.fillStyle = css(TONE[state] ? token(TONE[state]!, "#fff") : WHITE);
  ctx.fillRect(bar.x, v.barY, bar.w, v.barH);
  ctx.globalAlpha = 1;
}

/** One picture of the mascot, `size` pixels square, as RGBA. */
function paint(canvas: HTMLCanvasElement, engine: BotEngine, size: number, state: BotStateName, t: number, shut: boolean): Uint8ClampedArray {
  if (canvas.width !== size) canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  wipe(ctx);
  if (size < ENGINE_MIN) drawSmall(ctx, size, engine.bodyColor ?? WHITE, state, t, shut);
  else engine.draw(ctx, size / 2, size / 2, size);
  return ctx.getImageData(0, 0, size, size).data;
}

const bytes = (rgba: Uint8ClampedArray) => new Uint8Array(rgba.buffer, rgba.byteOffset, rgba.byteLength);

/** Between two frames while it moves: six a second. */
const FRAME_MS = 160;
/** At rest it blinks every six to ten seconds, shut for this long. */
const BLINK_EVERY_MS = 6000;
const BLINK_SHUT_MS = 140;
/** How long after the last word of a lock or a sleep the pointer coming back counts as someone being there. */
const AWAY_MS = 1500;
/** The sizes the icon of a shortcut holds. */
const ICON_SIZES = [16, 20, 24, 32, 40, 48, 64, 256];
/** The size handed over for a window's taskbar button. */
const WINDOW_ICON = 32;

export class TrayMascot {
  private engine = new BotEngine();
  private canvas: HTMLCanvasElement | null = null;
  /** The frame the tray shows, and what it was drawn for. */
  private shown: Uint8ClampedArray | null = null;
  private key = "";
  /** The colour the windows and the shortcuts were last given. */
  private brand = "";
  private frames: number | undefined;
  private blink: number | undefined;
  private lastFrame = 0;
  private awayAt = -Infinity;
  private noticed = false;
  private notice: () => boolean;

  /** `notice`: a notice is up on the island. */
  constructor(notice: () => boolean) {
    this.notice = notice;
    this.engine.isMini = true;
    // It blinks when this file says so, never by itself.
    this.engine.blinkIn = Infinity;
  }

  /** The computer was locked or slept (the island's cursor poll, or the host refusing a frame): it stops moving. */
  away() {
    this.awayAt = performance.now();
    this.sync();
  }

  /** The pointer moved: someone is there again. Called for every move, so it is two comparisons when nobody was away. */
  back() {
    if (this.awayAt === -Infinity || performance.now() - this.awayAt < AWAY_MS) return;
    this.awayAt = -Infinity;
    this.sync();
  }

  /** Something may have changed (a state, a colour, a setting, a notice): called from the island's own sync. */
  sync() {
    const s = State.settings;
    const now: TrayNow = {
      animate: s.trayAnimate !== false, mascot: s.showMascot, paused: State.paused,
      away: this.awayAt !== -Infinity, state: State.effectiveState, notice: this.notice(),
    };
    this.syncBrand();
    const [mode, face, color] = [trayMode(now), trayFace(now), brandColor(s.pillColors)];
    const size = traySize(window.devicePixelRatio || 1);
    // A notice came up: one wink for it.
    if (now.notice && !this.noticed && mode === "moving") this.engine.triggerEmote("wink");
    this.noticed = now.notice;
    const key = [mode, face, color, size, trayBlinks(now)].join();
    if (key === this.key) return;
    this.key = key;
    this.stop();
    if (mode === "off") return;
    this.engine.bodyColor = hexToRGB(color);
    this.engine.setState(face);
    if (mode === "moving") {
      this.lastFrame = performance.now();
      this.frames = window.setInterval(() => this.frame(size, face), FRAME_MS);
      this.frame(size, face);
    } else {
      this.still(size, face);
      if (trayBlinks(now)) this.blinkLater(size, face);
    }
  }

  private stop() {
    window.clearInterval(this.frames);
    window.clearTimeout(this.blink);
    this.frames = this.blink = undefined;
  }

  private frame(size: number, face: BotStateName) {
    const t = performance.now();
    this.engine.update(Math.min(0.25, (t - this.lastFrame) / 1000));
    this.lastFrame = t;
    this.send(this.draw(size, face, t / 1000, false));
  }

  /** The pose once every fade has ended, at the middle of its bob: the same pixels each time, so nothing is sent twice. */
  private still(size: number, face: BotStateName) {
    for (let i = 0; i < 24; i++) this.engine.update(0.05);
    this.engine.t = 0;
    this.send(this.draw(size, face, 0, false));
  }

  /** One timer every several seconds while it rests: shut, then open again. */
  private blinkLater(size: number, face: BotStateName) {
    this.blink = window.setTimeout(() => {
      this.engine.blink();
      this.engine.update(0.05);
      this.engine.t = 0;
      this.send(this.draw(size, face, 0, true));
      this.blink = window.setTimeout(() => {
        this.still(size, face);
        this.blinkLater(size, face);
      }, BLINK_SHUT_MS);
    }, BLINK_EVERY_MS + Math.random() * 4000);
  }

  private draw(size: number, face: BotStateName, t: number, shut: boolean): Uint8ClampedArray {
    return paint((this.canvas ??= document.createElement("canvas")), this.engine, size, face, t, shut);
  }

  private send(frame: Uint8ClampedArray) {
    if (!frameChanged(this.shown, frame)) return;
    this.shown = frame;
    void Bridge.trayIcon(bytes(frame)).then((there) => {
      // The host could not see the desktop (locked, a prompt of Windows): it stops moving. A frame
      // that got through after that means someone is back.
      if (there === false) this.away();
      else if (there) this.back();
    });
  }

  /**
   * The windows' taskbar button and the shortcuts wear the product's colour: told at the start
   * and when the colour or the switch changes, never on a timer. With the catalog's colour the
   * icon the program carries is the right one already, and the host is told to use it.
   */
  private syncBrand() {
    const s = State.settings;
    const color = brandColor(s.pillColors);
    const shortcuts = s.iconShortcuts !== false;
    const key = `${color}|${shortcuts}`;
    if (key === this.brand) return;
    this.brand = key;
    const own = !brandIsDefault(s.pillColors);
    // An engine of its own, at rest: the tray's keeps the face it has.
    const idle = new BotEngine();
    idle.bodyColor = hexToRGB(color);
    idle.t = 0;
    const canvas = document.createElement("canvas");
    const still = (size: number) => paint(canvas, idle, size, "idle", 0, false);
    void Bridge.windowIcon(own ? bytes(still(WINDOW_ICON)) : null);
    if (own && shortcuts) void shortcutIco(canvas, still).then((ico) => Bridge.shortcutIcon(ico));
    else void Bridge.shortcutIcon(null);
  }
}

/** The .ico of a shortcut: a bitmap of each size, and a PNG at 256 as Windows wants it. */
async function shortcutIco(canvas: HTMLCanvasElement, still: (size: number) => Uint8ClampedArray): Promise<Uint8Array> {
  const entries: IcoEntry[] = [];
  for (const size of ICON_SIZES) {
    const rgba = still(size);
    if (size < 256) entries.push({ size, data: dibEntry(rgba, size) });
    else {
      const png = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/png"));
      entries.push({ size, data: png ? new Uint8Array(await png.arrayBuffer()) : dibEntry(rgba, size) });
    }
  }
  return encodeIco(entries);
}
