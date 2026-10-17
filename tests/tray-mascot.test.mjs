// The mascot outside the island (src/island/tray-mascot.ts): which colour it wears, when the
// tray's moves, what a frame is, and the 16 px visor on whole pixels. And the .ico the shortcuts
// are given (src/core/ico.ts). No canvas here: the drawing itself is looked at in the running app.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  BRAND_DEFAULT, BRAND_PILL, ENGINE_MIN, brandColor, brandIsDefault, frameChanged, smallBar, smallVisor, trayBlinks, trayFace,
  trayMode, traySize,
} from "../src/island/tray-mascot.ts";
import { dibEntry, encodeIco } from "../src/core/ico.ts";
import { pillDefinition } from "../src/core/pills.ts";
import { DEFAULT_SETTINGS, State } from "../src/core/state.ts";

const STATES = ["idle", "working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit", "sleeping", "dizzy"];

// ── Which colour ──────────────────────────────────────────────────────────────

test("the product's colour is the user's colour for the VS Code pill, else the accent", () => {
  const catalog = BRAND_DEFAULT;
  assert.equal(pillDefinition(BRAND_PILL).name, "Claude Code");
  assert.equal(brandColor({}), catalog);
  assert.equal(brandColor(null), catalog);
  assert.equal(brandColor({ integration_claude: "#2dd4bf" }), "#2DD4BF");
  // Another pill's colour is not the product's, and what is not a colour is no choice.
  assert.equal(brandColor({ agent_codex: "#FACC15" }), catalog);
  assert.equal(brandColor({ integration_claude: "teal" }), catalog);
  assert.equal(brandIsDefault({}), true);
  assert.equal(brandIsDefault({ agent_codex: "#FACC15" }), true);
  assert.equal(brandIsDefault({ integration_claude: "#2DD4BF" }), false);
});

test("a state never changes the body's colour, in the island or outside it", () => {
  State.settings = { ...DEFAULT_SETTINGS, pillColors: { integration_claude: "#2DD4BF" } };
  State.tasks = [];
  State.loadIntegrationTasks();
  State.setFocus(BRAND_PILL);
  for (const state of STATES) {
    State.updateTask(BRAND_PILL, state);
    assert.equal(State.mascotColor, "#2DD4BF", state);
    assert.equal(brandColor(State.settings.pillColors), "#2DD4BF", state);
  }
  // The tray keeps the product's colour while another pill is in front in the island.
  const other = State.tasks.find((t) => t.id !== BRAND_PILL);
  State.setFocus(other.id);
  assert.equal(State.mascotColor, other.color);
  assert.equal(brandColor(State.settings.pillColors), "#2DD4BF");
});

// ── When it moves ─────────────────────────────────────────────────────────────

const now = (o = {}) => ({ animate: true, mascot: true, paused: false, away: false, state: "idle", notice: false, ...o });

test("the icon's side follows the display's scale", () => {
  assert.equal(traySize(1), 16);
  assert.equal(traySize(1.25), 20);
  assert.equal(traySize(1.5), 24);
  assert.equal(traySize(1.75), 32);
  assert.equal(traySize(2), 32);
  assert.equal(traySize(3), 32);
  assert.equal(traySize(NaN), 16);
  assert.equal(traySize(0), 16);
});

test("it moves only while the island's mascot has a reason to", () => {
  for (const state of ["working", "thinking", "searching", "approval", "question"]) assert.equal(trayMode(now({ state })), "moving", state);
  for (const state of ["idle", "finished", "error", "ratelimit", "sleeping", "dizzy"]) assert.equal(trayMode(now({ state })), "still", state);
  assert.equal(trayMode(now({ notice: true })), "moving");
});

test("the switch off, or the mascot hidden: one still picture of the resting face", () => {
  for (const off of [{ animate: false }, { mascot: false }]) {
    const o = now({ state: "working", notice: true, ...off });
    assert.equal(trayMode(o), "still");
    assert.equal(trayFace(o), "idle");
    assert.equal(trayBlinks(o), false);
  }
  assert.equal(trayFace(now({ state: "approval" })), "approval");
});

test("paused, nothing is sent; locked or asleep, it does not move but keeps the face of the moment", () => {
  assert.equal(trayMode(now({ state: "working", paused: true })), "off");
  assert.equal(trayMode(now({ state: "working", paused: true, away: true })), "off");
  const locked = now({ state: "working", away: true });
  assert.equal(trayMode(locked), "still");
  assert.equal(trayFace(locked), "working");
  assert.equal(trayBlinks(now({ away: true })), false);
});

test("it blinks only at rest, alive and seen: the one timer of an idle tray", () => {
  assert.equal(trayBlinks(now()), true);
  for (const o of [{ state: "finished" }, { state: "working" }, { notice: true }, { paused: true }, { animate: false }, { mascot: false }, { away: true }]) {
    assert.equal(trayBlinks(now(o)), false, JSON.stringify(o));
  }
});

test("a frame is sent only when it differs from the one shown", () => {
  const a = new Uint8ClampedArray([1, 2, 3, 255, 0, 0, 0, 0]);
  assert.equal(frameChanged(null, a), true);
  assert.equal(frameChanged(a, new Uint8ClampedArray(a)), false);
  assert.equal(frameChanged(a, new Uint8ClampedArray([1, 2, 3, 255, 0, 0, 0, 1])), true);
  // Another size is another frame.
  assert.equal(frameChanged(a, new Uint8ClampedArray(4)), true);
});

// ── 16 px ─────────────────────────────────────────────────────────────────────

test("only 16 px is drawn without the engine", () => {
  assert.equal(traySize(1) < ENGINE_MIN, true);
  for (const scale of [1.25, 1.5, 2]) assert.equal(traySize(scale) >= ENGINE_MIN, true);
});

test("at 16 px the visor and its bar sit on whole pixels, inside the band", () => {
  const v = smallVisor(16);
  assert.deepEqual(v, { bandX: 2, bandY: 4, bandW: 12, bandH: 4, barY: 5, barW: 6, barH: 2 });
  for (const state of STATES) {
    for (let i = 0; i < 40; i++) {
      const bar = smallBar(state, i * 0.16, 16, false);
      assert.ok(bar, state);
      assert.equal(Number.isInteger(bar.x) && Number.isInteger(bar.w), true, `${state} at frame ${i}`);
      assert.ok(bar.x >= v.bandX + 1 && bar.x + bar.w <= v.bandX + v.bandW - 1, `${state} at frame ${i}`);
      assert.ok(bar.alpha > 0.4 && bar.alpha <= 1);
    }
  }
});

test("at 16 px the bar scans while an agent works, pulses while one waits, and is gone for a blink", () => {
  const places = new Set(Array.from({ length: 14 }, (_, i) => smallBar("working", i * 0.16, 16, false).x));
  assert.ok(places.size >= 4, "the scan takes several places");
  const pulse = Array.from({ length: 8 }, (_, i) => smallBar("approval", i * 0.16, 16, false));
  assert.equal(new Set(pulse.map((b) => b.x)).size, 1);
  assert.ok(Math.max(...pulse.map((b) => b.alpha)) - Math.min(...pulse.map((b) => b.alpha)) > 0.3);
  assert.equal(smallBar("idle", 0, 16, true), null);
  assert.deepEqual(smallBar("idle", 0, 16, false), { x: 5, w: 6, alpha: 1 });
  assert.equal(smallBar("sleeping", 0, 16, false).alpha, 0.5);
});

// ── The shortcuts' .ico ───────────────────────────────────────────────────────

test("a bitmap entry: its header, the rows bottom-up as BGRA, an empty mask", () => {
  // 2 x 2: red, green / blue, half-clear white.
  const rgba = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128];
  const dib = dibEntry(rgba, 2);
  const view = new DataView(dib.buffer);
  assert.equal(dib.length, 40 + 16 + 4 * 2);
  assert.deepEqual([view.getUint32(0, true), view.getInt32(4, true), view.getInt32(8, true)], [40, 2, 4]);
  assert.deepEqual([view.getUint16(12, true), view.getUint16(14, true)], [1, 32]);
  // The bottom row first: blue, then white.
  assert.deepEqual([...dib.slice(40, 48)], [255, 0, 0, 255, 255, 255, 255, 128]);
  assert.deepEqual([...dib.slice(48, 56)], [0, 0, 255, 255, 0, 255, 0, 255]);
  assert.deepEqual([...dib.slice(56)], [0, 0, 0, 0, 0, 0, 0, 0]);
});

test("the file: a header, a directory with each picture's place, the pictures", () => {
  const small = dibEntry(new Uint8Array(16 * 16 * 4), 16);
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 1, 2, 3]);
  const ico = encodeIco([{ size: 16, data: small }, { size: 256, data: png }]);
  const view = new DataView(ico.buffer);
  assert.deepEqual([...ico.slice(0, 6)], [0, 0, 1, 0, 2, 0]);
  assert.equal(ico.length, 6 + 32 + small.length + png.length);
  // 16 px, then 256 written as 0.
  assert.deepEqual([ico[6], ico[7], view.getUint16(12, true), view.getUint32(14, true), view.getUint32(18, true)], [16, 16, 32, small.length, 38]);
  assert.deepEqual([ico[22], ico[23], view.getUint32(30, true), view.getUint32(34, true)], [0, 0, png.length, 38 + small.length]);
  assert.deepEqual([...ico.slice(38 + small.length)], [...png]);
});
