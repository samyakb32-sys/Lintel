// What a voice turn shows (src/features/voice/ui/, settings.ts; docs/VOICE.md §7.5, §7.7, §9): the
// strip row in each state, the face and the sound that go with it, what the capsule holds, where it
// sits and when it stays away, which key may be the talk key, what a download's row shows. The rules
// are plain functions (ui/rules.ts); the last tests run the real wiring (ui/index.ts) against the real
// turn loop with a made-up Rust side, a made-up island and a made-up DOM.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installFakeDom } from "./fakedom.mjs";
import {
  CAPSULE, THINK_MS, capsuleFollows, capsuleRect, capsuleShown, captionFor, downloadLook, emoteFor, ending, faceFor, levelFor, megabytes,
  micOpen, rowActive, rowFor, soundFor, talkKeyName, talkPicker,
} from "../src/features/voice/ui/rules.ts";

const view = (patch = {}) => ({ state: "idle", heard: "", final: false, answer: [], spoken: -1, result: "", confirm: null, undo: null, ...patch });

// ── The strip row ─────────────────────────────────────────────────────────────

test("the row has the strip during a turn, and while it closes only with a result to read", () => {
  for (const state of ["listening", "thinking", "speaking", "asking"]) assert.ok(rowActive(view({ state })), state);
  for (const state of ["off", "idle", "waiting", "hearing", "paused", "closing"]) assert.ok(!rowActive(view({ state })), state);
  assert.ok(rowActive(view({ state: "closing", result: "Timer · 12 min" })));
});

test("the row says Listening, Thinking or the result; its right end is the meter, the bars or Yes", () => {
  assert.deepEqual(rowFor(view({ state: "listening" })), { title: "Listening", detail: "", right: "meter" });
  assert.deepEqual(rowFor(view({ state: "listening", heard: "set a timer" })), { title: "Listening", detail: "set a timer", right: "meter" });
  assert.deepEqual(rowFor(view({ state: "thinking", heard: "set a timer for 12 minutes", final: true })), { title: "Thinking", detail: "set a timer for 12 minutes", right: null });
  // An answer with no result of its own: the sentence being said.
  const answer = ["It's 4 degrees.", "Cloudy until noon."];
  assert.deepEqual(rowFor(view({ state: "speaking", answer, spoken: 1 })), { title: "Speaking", detail: "Cloudy until noon.", right: "bars" });
  assert.deepEqual(rowFor(view({ state: "speaking", answer, spoken: -1 })), { title: "Speaking", detail: "", right: "bars" });
  // The result is the title, while it is said and while the turn closes.
  assert.deepEqual(rowFor(view({ state: "speaking", result: "Timer · 12 min", answer: ["Timer set."], spoken: 0 })), { title: "Timer · 12 min", detail: "", right: "bars" });
  assert.deepEqual(rowFor(view({ state: "closing", result: "Timer · 12 min" })), { title: "Timer · 12 min", detail: "", right: null });
  // The fixed titles go through `t`; what the user said and what was answered do not.
  assert.equal(rowFor(view({ state: "thinking", heard: "bonjour" }), (s) => `<${s}>`).title, "<Thinking>");
  assert.equal(rowFor(view({ state: "thinking", heard: "bonjour" }), (s) => `<${s}>`).detail, "bonjour");
});

test("a reply that is waited for shows the question until its first words come", () => {
  const asked = view({ state: "asking", heard: "set a timer", final: true, answer: ["For how long?"], spoken: 0 });
  assert.deepEqual(rowFor(asked), { title: "Listening", detail: "For how long?", right: "meter" });
  assert.deepEqual(rowFor({ ...asked, heard: "twelve", final: false }), { title: "Listening", detail: "twelve", right: "meter" });
});

test("a yes or no that waits is the row, with Yes, while it is said and while the microphone waits", () => {
  const confirm = { text: "Paste the last copy?", yes() {}, no() {} };
  for (const state of ["speaking", "asking"]) assert.deepEqual(rowFor(view({ state, confirm, answer: [confirm.text], spoken: 0 })), { title: confirm.text, detail: "", right: "yes" });
  // Not once it is thought about again.
  assert.equal(rowFor(view({ state: "thinking", confirm })).right, null);
});

test("the Visor's face and the level it follows", () => {
  assert.deepEqual(["listening", "thinking", "speaking", "asking", "closing", "idle", "waiting", "paused"].map(faceFor), ["listening", "thinking", "talking", "asking", null, null, null, null]);
  const level = { mic: 0.4, out: 0.7 };
  assert.equal(levelFor("listening", level), 0.4);
  assert.equal(levelFor("asking", level), 0.4);
  assert.equal(levelFor("speaking", level), 0.7);
  assert.equal(levelFor("thinking", level), 0);
  assert.ok(micOpen("listening") && micOpen("asking") && !micOpen("speaking") && !micOpen("waiting"));
});

// ── How a turn ended, and its sounds ──────────────────────────────────────────

test("how a turn ended: spoken, shown without a word, a miss, or nothing at all", () => {
  const none = [];
  const at = { answer: none, spoken: -1, result: "" };
  // Nothing was heard (a tap by mistake, Esc before a word): no sound, no face.
  assert.equal(ending(view({ state: "closing" }), null), "nothing");
  assert.equal(ending(view({ state: "closing", heard: "set a", final: false }), null), "nothing");
  // A sentence was taken and nothing came of it.
  assert.equal(ending(view({ state: "closing", heard: "flibber the jabber", final: true, answer: none }), at), "miss");
  // It said something since.
  assert.equal(ending(view({ state: "closing", heard: "what time is it", final: true, answer: ["It's noon."], spoken: 0 }), at), "spoken");
  // It answered without a word (short replies as a sound, or speech off).
  assert.equal(ending(view({ state: "closing", heard: "pause", final: true, answer: ["Paused."], spoken: -1 }), at), "shown");
  assert.equal(ending(view({ state: "closing", heard: "set a timer", final: true, answer: none, result: "Timer · 5 min" }), at), "shown");
  // The second miss of a conversation: "Sorry, say that again?" is still there from before, and nothing is new.
  const sorry = ["Sorry, say that again?"];
  assert.equal(ending(view({ state: "closing", heard: "mumble", final: true, answer: sorry, spoken: 0 }), { answer: sorry, spoken: 0, result: "" }), "miss");
  // "Cancel" and "that's all" are answers in themselves.
  assert.equal(ending(view({ state: "closing", heard: "never mind", final: true }), at, true), "nothing");
});

test("the sounds: blip when it listens, pop when the turn is taken, annoyed on a miss, wink at a clean silent end", () => {
  assert.equal(soundFor("idle", "listening"), "blip");
  assert.equal(soundFor("waiting", "listening"), "blip");
  assert.equal(soundFor("speaking", "asking"), "blip");
  assert.equal(soundFor("listening", "listening"), null);
  assert.equal(soundFor("listening", "thinking"), "pop");
  assert.equal(soundFor("asking", "thinking"), "pop");
  assert.equal(soundFor("thinking", "speaking"), null);
  assert.equal(soundFor("thinking", "closing", "miss"), "annoyed");
  assert.equal(soundFor("thinking", "closing", "shown"), "wink");
  assert.equal(soundFor("speaking", "closing", "spoken"), null);
  assert.equal(soundFor("listening", "closing", "nothing"), null);
  assert.equal(soundFor("closing", "idle"), null);
  assert.equal(soundFor("idle", "paused"), null);
  assert.equal(THINK_MS, 1200);
  assert.deepEqual(["spoken", "shown", "miss", "nothing"].map(emoteFor), ["wink", "wink", "surprised", null]);
});

// ── The capsule ───────────────────────────────────────────────────────────────

test("the capsule is there through the turn and while its captions stay, and leaves a turn that heard nothing at once", () => {
  for (const state of ["listening", "thinking", "speaking", "asking"]) assert.ok(capsuleShown(view({ state })), state);
  for (const state of ["off", "idle", "waiting", "hearing", "paused"]) assert.ok(!capsuleShown(view({ state, heard: "x", answer: ["y"] })), state);
  assert.ok(capsuleShown(view({ state: "closing", heard: "what time is it" })));
  assert.ok(capsuleShown(view({ state: "closing", answer: ["It's noon."] })));
  assert.ok(!capsuleShown(view({ state: "closing" })));
});

test("the capsule holds what was heard and the answer from the sentence being said, on one to three lines", () => {
  const on = { captions: true, open: false };
  const listening = captionFor(view({ state: "listening" }), on);
  assert.deepEqual([listening.mic, listening.lines, listening.w, listening.h], [true, 1, 380, 32]);
  const answer = ["It's 4 degrees in Oslo.", "Cloudy until noon, then some sun.", "Tomorrow is colder."];
  const speaking = captionFor(view({ state: "speaking", heard: "what's the weather", final: true, answer, spoken: 1 }), on);
  assert.equal(speaking.mic, true, "open while it speaks, for the word stop");
  assert.equal(captionFor(view({ state: "thinking" }), on).mic, false);
  assert.equal(speaking.heard, "what's the weather");
  // Folded, it begins at the sentence being said; that one is bright, the next still waits.
  assert.deepEqual(speaking.answer, [{ text: answer[1], at: "now" }, { text: answer[2], at: "next" }]);
  assert.deepEqual([speaking.lines, speaking.h], [3, 64]);
  // Before the first sentence starts, everything waits; once the mouth is done (or was never asked), it is simply the answer.
  assert.deepEqual(captionFor(view({ state: "speaking", answer, spoken: -1 }), on).answer.map((a) => a.at), ["next", "next", "next"]);
  assert.deepEqual(captionFor(view({ state: "closing", answer: ["Paused."], spoken: -1 }), on).answer, [{ text: "Paused.", at: "now" }]);
  // A short answer under what was heard: two lines.
  assert.deepEqual([captionFor(view({ state: "speaking", heard: "pause", answer: ["Paused."], spoken: 0 }), on).lines, captionFor(view({ state: "speaking", heard: "pause", answer: ["Paused."], spoken: 0 }), on).h], [2, 48]);
  // Undo is offered while the runner offers it.
  assert.equal(captionFor(view({ state: "speaking", answer: ["Done."], spoken: 0, undo() {} }), on).undo, true);
  assert.equal(speaking.undo, false);
  // The online model that wrote the answer is named after it, dim; never where there is no answer.
  assert.deepEqual(captionFor(view({ state: "speaking", answer: ["Blue light scatters."], spoken: 0, by: "Claude" }), on).answer, [{ text: "Blue light scatters.", at: "now" }, { text: "· Claude", at: "next" }]);
  assert.deepEqual(captionFor(view({ state: "listening", by: "Claude" }), on).answer, []);
});

test("opened by a click the capsule is 480 × 240 with the whole answer; with captions off it is the dot and the stop button", () => {
  const answer = ["One.", "Two.", "Three."];
  const open = captionFor(view({ state: "speaking", heard: "count", answer, spoken: 2 }), { captions: true, open: true });
  assert.deepEqual([open.open, open.w, open.h], [true, CAPSULE.openW, CAPSULE.openH]);
  assert.deepEqual(open.answer.map((a) => a.at), ["said", "said", "now"]);
  const bare = captionFor(view({ state: "listening", heard: "secret words", answer }), { captions: false, open: true });
  assert.deepEqual(bare, { mic: true, heard: "", answer: [], undo: false, bare: true, open: false, lines: 1, w: 64, h: 32 });
});

const display = { x: 960, y: 0, scale: 1, left: 0, top: 0, right: 1920, bottom: 1080 };
const strip = { mode: "strip", width: 340, height: 32, top: 0 };
const open = { mode: "open", width: 640, height: 320, top: 0 };
const size = { w: 380, h: 48 };
/** The island's box on the screen, physical pixels. */
const box = (shape, o) => ({ top: o.y + shape.top * o.scale, bottom: o.y + (shape.top + shape.height) * o.scale });
const apart = (rect, shape, o) => rect.y >= box(shape, o).bottom || rect.y + rect.h <= box(shape, o).top;

test("the capsule sits centred, 6 px under the island's lower edge: strip, open island, full-screen app", () => {
  assert.deepEqual(capsuleRect(strip, display, size), { x: 770, y: 38, w: 380, h: 48 });
  assert.deepEqual(capsuleRect(open, display, size), { x: 770, y: 326, w: 380, h: 48 });
  // Hidden under a full-screen app: 6 px from the top.
  assert.deepEqual(capsuleRect({ mode: "hidden", width: 240, height: 0, top: 0 }, display, size), { x: 770, y: 6, w: 380, h: 48 });
  // A display at 150 %: every length in its pixels.
  assert.deepEqual(capsuleRect(strip, { ...display, x: 1440, scale: 1.5, right: 2880, bottom: 1620 }, size), { x: 1155, y: 57, w: 570, h: 72 });
  // A second display to the left of the first.
  assert.deepEqual(capsuleRect(strip, { ...display, x: -960, left: -1920, right: 0 }, size), { x: -1150, y: 38, w: 380, h: 48 });
  // A floating island hangs 12 px lower in its window.
  assert.deepEqual(capsuleRect({ ...strip, top: 12 }, { ...display, y: 300 }, size), { x: 770, y: 350, w: 380, h: 48 });
});

test("the capsule never lies on the island, so never on a card that waits", () => {
  const sizes = [size, { w: 380, h: 32 }, { w: 380, h: 64 }, { w: 480, h: 240 }, { w: 64, h: 32 }];
  for (const shape of [strip, open, { ...open, height: 300, top: 12 }, { mode: "hidden", width: 240, height: 0, top: 0 }]) {
    for (const s of sizes) {
      for (const y of [0, 200, 500, 700, 760, 1000]) {
        for (const scale of [1, 1.25, 1.5, 2]) {
          const o = { ...display, y, scale };
          const rect = capsuleRect(shape, o, s);
          if (!rect) continue;
          assert.ok(apart(rect, shape, o), `on the island: ${JSON.stringify({ shape, s, y, scale, rect })}`);
          assert.ok(rect.y >= o.top && rect.y + rect.h <= o.bottom && rect.x >= o.left && rect.x + rect.w <= o.right, `off the display: ${JSON.stringify({ shape, s, y, scale, rect })}`);
        }
      }
    }
  }
  // An island dragged to the bottom of the display: no room under it, so 6 px above it.
  assert.deepEqual(capsuleRect(open, { ...display, y: 740 }, size), { x: 770, y: 686, w: 380, h: 48 });
  // No room either way (a display barely taller than the island): it stays away.
  assert.equal(capsuleRect(open, { ...display, y: 0, bottom: 340 }, size), null);
  // Near a side edge it is kept on the display.
  assert.equal(capsuleRect(strip, { ...display, x: 100 }, size).x, 0);
  assert.equal(capsuleRect(strip, { ...display, x: 1900 }, size).x, 1540);
});

test("the capsule follows the island, and leaves with a strip that hides while the captions only stay", () => {
  const hidden = { mode: "hidden", width: 240, height: 0, top: 0 };
  assert.ok(capsuleFollows("speaking", hidden));
  assert.ok(capsuleFollows("closing", strip));
  assert.ok(capsuleFollows("closing", open));
  assert.ok(!capsuleFollows("closing", hidden));
});

// ── Settings: the talk key ────────────────────────────────────────────────────

test("the talk key is one key that types nothing, taken when it is let go", () => {
  const p = talkPicker();
  assert.deepEqual(p.down({ code: "ControlRight", key: "Control" }), { kind: "held", key: "RCtrl" });
  assert.deepEqual(p.up(), { kind: "save", key: "RCtrl" });
  for (const [code, key] of [["ShiftRight", "RShift"], ["AltRight", "RAlt"], ["ScrollLock", "ScrollLock"], ["Pause", "Pause"], ["Insert", "Insert"], ["F1", "F1"], ["F13", "F13"], ["F24", "F24"]]) {
    const q = talkPicker();
    assert.deepEqual(q.down({ code, key: code }), { kind: "held", key }, code);
    assert.deepEqual(q.up(), { kind: "save", key }, code);
  }
  assert.deepEqual(["RCtrl", "RShift", "RAlt", "ScrollLock", "F13"].map(talkKeyName), ["Right Ctrl", "Right Shift", "Right Alt", "Scroll Lock", "F13"]);
});

test("Esc cancels, Backspace removes, and a key that cannot be the talk key says why and saves nothing", () => {
  assert.deepEqual(talkPicker().down({ code: "Escape", key: "Escape" }), { kind: "cancel" });
  assert.deepEqual(talkPicker().down({ code: "Backspace", key: "Backspace" }), { kind: "clear" });
  // A letter, a left modifier, the keys that open something on their own, a function key that does not exist.
  for (const code of ["KeyA", "Digit1", "Space", "Enter", "ControlLeft", "ShiftLeft", "AltLeft", "MetaLeft", "MetaRight", "ContextMenu", "F25", "F0"]) {
    const p = talkPicker();
    assert.equal(p.down({ code, key: code }).kind, "refused", code);
    assert.deepEqual(p.up(), { kind: "held", key: "" }, code);
  }
  // A good key, then a bad one before letting go: nothing is saved.
  const p = talkPicker();
  p.down({ code: "ControlRight", key: "Control" });
  p.down({ code: "KeyA", key: "a" });
  assert.equal(p.up().kind, "held");
  // AltGr types characters on the layouts that have it.
  assert.match(talkPicker().down({ code: "AltRight", key: "AltGraph" }).why, /Right Alt types characters/);
});

test("the picker stores only names the Rust side knows", () => {
  const rust = readFileSync(new URL("../crates/voice/src/ears.rs", import.meta.url), "utf8");
  const known = rust.slice(rust.indexOf("fn vk_of"), rust.indexOf("fn alone"));
  for (const code of ["ControlRight", "ShiftRight", "AltRight", "ScrollLock", "Pause", "Insert"]) {
    const { key } = talkPicker().down({ code, key: code });
    assert.ok(known.includes(`"${key}"`), `${key} is not in vk_of`);
  }
  assert.ok(known.includes("starts_with('F')"), "function keys");
});

// ── Settings: a download ──────────────────────────────────────────────────────

test("a download's row: Download, a count, the check of the files, Ready, or why it failed", () => {
  const off = { installed: false, downloading: false };
  assert.deepEqual(downloadLook(null, null), { kind: "get" });
  assert.deepEqual(downloadLook(off, null), { kind: "get" });
  assert.deepEqual(downloadLook({ installed: true, downloading: false }, null), { kind: "ready" });
  // Running when the page was opened: busy until the first count arrives.
  assert.deepEqual(downloadLook({ installed: false, downloading: true }, null), { kind: "busy", fraction: 0, text: "" });
  // Just clicked: no count yet.
  assert.deepEqual(downloadLook(off, { what: "speech", received: 0, total: 0, done: false }), { kind: "busy", fraction: 0, text: "" });
  const half = downloadLook(off, { what: "speech", received: 185_560_645, total: 371_121_290, done: false });
  assert.deepEqual(half, { kind: "busy", fraction: 0.5, text: "185 MB of 371 MB" });
  assert.equal(downloadLook(off, { what: "generator", received: 1_250_000_000, total: 2_760_466_294, done: false }).text, "1.2 GB of 2.7 GB");
  // Every byte is here and it is not done: the files are being checked.
  assert.deepEqual(downloadLook(off, { what: "generator", received: 2_760_466_294, total: 2_760_466_294, done: false }), { kind: "check" });
  assert.deepEqual(downloadLook(off, { what: "speech", received: 371_121_290, total: 371_121_290, done: true }), { kind: "ready" });
  assert.deepEqual(downloadLook(off, { what: "speech", received: 0, total: 371_121_290, done: true, error: "download: checksum mismatch, file deleted" }), { kind: "error", text: "download: checksum mismatch, file deleted" });
  // Stopped by switching voice off: back to the button, with nothing to explain.
  assert.deepEqual(downloadLook(off, { what: "generator", received: 0, total: 1, done: true, error: "cancelled" }), { kind: "get" });
});

test("sizes are cut, never rounded up", () => {
  assert.equal(megabytes(18_488_687), "18 MB");
  assert.equal(megabytes(371_121_290), "371 MB");
  assert.equal(megabytes(2_740_937_888), "2.7 GB");
  assert.equal(megabytes(2_760_466_294), "2.7 GB");
  assert.equal(megabytes(0), "0 MB");
});

// ── The strings ───────────────────────────────────────────────────────────────

test("every string these files show is in strings.json, in English and nothing else", () => {
  const dir = new URL("../src/features/voice/", import.meta.url);
  const { strings } = JSON.parse(readFileSync(new URL("strings.json", dir), "utf8"));
  for (const [key, entry] of Object.entries(strings)) assert.deepEqual(entry, { en: key }, key);
  const used = new Set();
  for (const file of ["settings.ts", "ui/index.ts", "ui/rules.ts"]) {
    const code = readFileSync(new URL(file, dir), "utf8");
    for (const m of code.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) used.add(JSON.parse(`"${m[1]}"`));
    // No string is chosen inside the call, where this scan would not see it.
    assert.doesNotMatch(code, /\bt\(\s*[a-zA-Z_.]+\s*\?/, `${file}: a string picked inside t()`);
  }
  used.add(talkPicker().down({ code: "KeyA", key: "a" }).why);
  used.add(talkPicker().down({ code: "AltRight", key: "AltGraph" }).why);
  assert.ok(used.size > 80, `only ${used.size} strings found: has the scan broken?`);
  assert.deepEqual([...used].filter((key) => !(key in strings)), []);
  // The page says what the owner decided, in so many words.
  for (const key of ["How listening starts", "Talk key", "Try the voice", "1. Built-in model", "3. A free online model", "4. Claude, with my own key", "Weather by voice", "Look up facts", "What voice never does", "Stopping it"]) {
    assert.ok(key in strings, key);
  }
  const all = Object.keys(strings).join("\n");
  // No upstream name; the first two are spelled in halves so this file passes scripts/check-brand.mjs itself.
  assert.doesNotMatch(all, new RegExp(["cou" + "cou", "mo" + "chi", "vorssaint", "boring notch"].join("|"), "i"));
});

test("the online switches of the page start off, and the talk key starts as Right Ctrl", () => {
  const rust = readFileSync(new URL("../crates/voice/src/lib.rs", import.meta.url), "utf8");
  const defaults = JSON.parse(/defaults: r#"(\{.*\})"#/.exec(rust)[1]);
  for (const key of ["freeModel", "claude", "eleven", "weather", "lookup", "localServer", "chatAloud"]) assert.equal(defaults[key], false, key);
  assert.equal(defaults.wake, "off");
  assert.equal(defaults.key, "RCtrl");
  assert.equal(defaults.city, "");
  // Every setting the page writes is one the manifest has.
  const page = readFileSync(new URL("../src/features/voice/settings.ts", import.meta.url), "utf8");
  for (const m of page.matchAll(/\bsw\("(\w+)"/g)) assert.ok(m[1] in defaults, m[1]);
  for (const m of page.matchAll(/\bset\(\{ (\w+):/g)) assert.ok(m[1] in defaults, m[1]);
});

// ── The wiring, against the real turn loop ────────────────────────────────────

installFakeDom();
globalThis.cancelAnimationFrame = () => {};
// What Tauri's `unlisten` reaches for in a webview (tests/tauri.mjs stands in for the rest).
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
const { startTurns } = await import("../src/features/voice/turn.ts");
const { capsulePage, startUi, turnActivity } = await import("../src/features/voice/ui/index.ts");

const tick = async (rounds = 8) => {
  for (let i = 0; i < rounds; i++) await new Promise((done) => setImmediate(done));
};

function world({ shape = strip, captions = true } = {}) {
  const calls = [];
  const island = [];
  const listeners = new Map();
  const w = { shape, ask: null };
  const cx = {
    id: "voice",
    call: async (method, args) => {
      calls.push([method, args]);
      if (method === "caption.origin") return display;
      if (method === "state") return { state: "idle", gen: 1 };
      if (method === "mouth.speaking") return { speaking: false };
      return {};
    },
    on: (event, fn) => {
      const set = listeners.get(event) ?? new Set();
      listeners.set(event, set);
      set.add(fn);
      return () => set.delete(fn);
    },
    settings: { get: async () => ({ captions }), patch: async () => {}, onChange: () => () => {} },
    island: {
      activityChanged: (id) => island.push(["activityChanged", id]),
      redraw: (id) => island.push(["redraw", id]),
      face: (face, level) => island.push(["face", face, level]),
      shape: (ask) => {
        if (ask !== undefined) w.ask = ask;
        return w.shape;
      },
    },
    t: (s) => s,
  };
  const stops = [startTurns(cx, { entries: () => [], cxOf: () => ({}), choose: async () => ({ kind: "none" }), answer: async () => null }), startUi(cx)];
  return {
    calls, island, w,
    emit: (event, payload) => {
      for (const fn of [...(listeners.get(event) ?? [])]) fn(payload);
    },
    sent: (method) => calls.filter(([m]) => m === method).map(([, args]) => args),
    stop: () => stops.reverse().forEach((stop) => stop()),
  };
}

test("a turn brings the row and the capsule, and they leave with it", async () => {
  const v = world();
  await tick();
  assert.equal(turnActivity.isActive(), false);
  assert.deepEqual(v.sent("caption.place"), []);

  v.emit("state", { state: "listening", gen: 2 });
  await tick();
  assert.equal(turnActivity.isActive(), true);
  assert.deepEqual(v.island.filter(([what]) => what === "activityChanged"), [["activityChanged", "turn"]]);
  const row = turnActivity.render();
  assert.equal(row.find("B")[0].textContent, "Listening");
  assert.equal(row.find(".meter").length, 1, "the microphone's meter");
  assert.equal(row.find("BUTTON").length, 0);
  // The island's mascot listens too.
  assert.deepEqual(v.island.find(([what]) => what === "face"), ["face", "listening", 0]);
  // The capsule: told what to show, then put 6 px under the strip, the microphone's dot on.
  assert.equal(v.sent("caption.show")[0].mic, true);
  assert.deepEqual(v.sent("caption.place"), [{ x: 770, y: 38, w: 380, h: 32 }]);
  assert.equal(typeof v.w.ask?.changed, "function", "it asked to be told when the island's box changes");

  // A word more is drawn where the row stands; the capsule stays where it is.
  v.emit("heard", { text: "what is the", final: false });
  await tick();
  assert.equal(v.island.at(-1)[0], "redraw");
  assert.equal(turnActivity.render().find(".d")[0].textContent, "what is the");
  assert.equal(v.sent("caption.show").at(-1).heard, "what is the");
  assert.equal(v.sent("caption.place").length, 1);

  // The level moves the meter and the face; nothing is drawn again.
  const before = v.island.length;
  v.emit("level", { mic: 0.6 });
  assert.deepEqual(v.island.slice(before), [["face", "listening", 0.6]]);

  // The island opens (a card arrived, the user hovered): the capsule goes under it, never on it.
  v.w.shape = open;
  v.w.ask.changed(open);
  await tick();
  assert.deepEqual(v.sent("caption.place").at(-1), { x: 770, y: 326, w: 380, h: 32 });

  // The capsule's stop button ends the turn.
  v.emit("caption-act", { what: "stop" });
  await tick();
  assert.equal(v.sent("ears.stop").length, 1);

  // Closing: the row goes (there is no result to read), the captions stay.
  v.emit("state", { state: "closing", gen: 2 });
  await tick();
  assert.equal(turnActivity.isActive(), false);
  assert.equal(v.sent("caption.hide").length, 0);
  // Rest: out in 220 ms, then the window is put away, and the island is no longer asked about its box.
  v.emit("state", { state: "idle", gen: 2 });
  await tick();
  assert.deepEqual(v.sent("caption.show").at(-1), { out: true });
  assert.equal(v.w.ask, null);
  await new Promise((done) => setTimeout(done, CAPSULE.outMs + 40));
  await tick();
  assert.equal(v.sent("caption.hide").length, 1);
  assert.deepEqual(v.island.filter(([what]) => what === "face").at(-1), ["face", null, 0], "the mascot is its own again");
  v.stop();
});

test("over a full-screen app the capsule is 6 px from the top; with captions off it is the dot and stop", async () => {
  const v = world({ shape: { mode: "hidden", width: 240, height: 0, top: 0 }, captions: false });
  await tick();
  v.emit("state", { state: "listening", gen: 2 });
  await tick();
  assert.deepEqual(v.sent("caption.place"), [{ x: 928, y: 6, w: 64, h: 32 }]);
  assert.equal(v.sent("caption.show").at(-1).bare, true);
  // Paused (the tray, a lock): everything goes at once.
  v.emit("state", { state: "paused", reason: "lock", gen: 3 });
  await tick();
  assert.equal(v.sent("caption.hide").length, 1);
  v.stop();
});

test("switched off, nothing of it is left", async () => {
  const v = world();
  await tick();
  v.emit("state", { state: "listening", gen: 2 });
  await tick();
  v.stop();
  await tick();
  assert.equal(turnActivity.isActive(), false);
  assert.equal(v.w.ask, null);
  assert.ok(v.sent("caption.hide").length >= 1);
  const calls = v.calls.length;
  v.emit("state", { state: "listening", gen: 3 });
  v.emit("level", { mic: 1 });
  await tick();
  assert.equal(v.calls.length, calls, "no call after stop");
});

test("the capsule's page draws what it is sent and reports its clicks; it listens to no key", async () => {
  const root = document.createElement("div");
  const acts = [];
  let draw = null;
  const events = [];
  capsulePage(root, {
    call: async (method, args) => void acts.push([method, args]),
    on: (event, fn) => (events.push(event), (draw = fn), () => {}),
    t: (s) => s,
  });
  assert.deepEqual(events, ["caption"]);
  // Just made: it asks for what it missed, and shows nothing until it is told.
  assert.deepEqual(acts, [["caption.act", { what: "ready" }]]);
  assert.equal(root.children.length, 0);

  draw(captionFor(view({ state: "speaking", heard: "what's the weather", final: true, answer: ["It's 4 degrees.", "Cloudy."], spoken: 0 }), { captions: true, open: false }));
  const cap = root.children[0];
  assert.ok(cap.classList.contains("f-voice-cap") && cap.classList.contains("fx-drop") && cap.classList.contains("mic"));
  assert.equal(cap.find(".f-voice-cap-heard")[0].textContent, "what's the weather");
  assert.deepEqual(cap.find(".f-voice-cap-answer")[0].children.map((s) => [s.className, s.textContent]), [["now", "It's 4 degrees. "], ["next", "Cloudy. "]]);

  // Listening with nothing heard yet: the dot is on and the line says so.
  draw(captionFor(view({ state: "listening" }), { captions: true, open: false }));
  assert.ok(cap.classList.contains("mic"));
  assert.equal(cap.find(".f-voice-cap-heard")[0].textContent, "Listening…");

  const [undo, stop] = cap.find("BUTTON");
  stop.fire("click");
  undo.fire("click");
  cap.fire("click");
  assert.deepEqual(acts.slice(1), [["caption.act", { what: "stop" }], ["caption.act", { what: "undo" }], ["caption.act", { what: "open" }]]);
  for (const el of [cap, ...cap.walk()]) for (const type of el.listeners.keys()) assert.equal(type, "click", "only clicks are heard");

  draw({ out: true });
  assert.ok(cap.classList.contains("fx-out") && !cap.classList.contains("mic") && !cap.classList.contains("fx-drop"));
});

// ── The settings page ─────────────────────────────────────────────────────────

test("the settings page: the owner's choices as controls, the downloads, and what leaves the computer said beside each switch", async () => {
  const { settingsPage } = await import("../src/features/voice/settings.ts");
  const rust = readFileSync(new URL("../crates/voice/src/lib.rs", import.meta.url), "utf8");
  const defaults = JSON.parse(/defaults: r#"(\{.*\})"#/.exec(rust)[1]);
  const patches = [];
  const calls = [];
  const listeners = new Map();
  const cx = {
    id: "voice",
    call: async (method, args) => {
      calls.push([method, args]);
      if (method === "mouth.voices") return [{ id: "windows:Microsoft David", name: "Microsoft David", engine: "windows", lang: "en-US" }];
      if (method === "ears.status") return { mic: false, model: true };
      if (method.endsWith(".status")) return { installed: false, downloading: false };
      return {};
    },
    on: (event, fn) => {
      const set = listeners.get(event) ?? new Set();
      listeners.set(event, set);
      set.add(fn);
      return () => set.delete(fn);
    },
    settings: { get: async () => defaults, patch: async (patch) => void patches.push(patch), onChange: () => () => {} },
    t: (s, vars) => s.replace(/\{(\w+)\}/g, (_, k) => vars?.[k] ?? `{${k}}`),
  };
  const page = settingsPage(cx);
  await tick();
  const text = page.textContent;
  const rows = new Map(page.find(".row").map((r) => [r.find(".row-label")[0].textContent, r]));
  assert.deepEqual(page.find(".section-title").map((e) => e.textContent), ["Listening", "Voice", "Answers", "From the internet", "Stopping it", "What voice never does"]);

  // How listening starts: the three choices, the talk key alone first.
  const wake = rows.get("How listening starts");
  assert.deepEqual(wake.find("option").map((o) => [o.getAttribute("value"), o.textContent]), [["off", "Talk key only"], ["on", "Talk key and “OK Lintel”"], ["plugged", "“OK Lintel” when plugged in"]]);
  assert.equal(wake.find("select")[0].value, "off");
  // Each choice says in a line what it does to the microphone, with the phrase spelled out.
  for (const [value, line] of [["plugged", /Listens for “OK Lintel” on mains power only/], ["on", /stays open to hear “OK Lintel”.*bar of light/], ["off", /opens only when you press the talk key/]]) {
    wake.find("select")[0].value = value;
    wake.find("select")[0].fire("change");
    assert.deepEqual(patches.at(-1), { wake: value });
    assert.match(wake.find(".row-hint")[0].textContent, line);
  }
  // The talk key: Right Ctrl, and what a tap and a hold do.
  assert.equal(rows.get("Talk key").find(".kbd")[0].textContent, "Right Ctrl");
  assert.match(rows.get("Talk key").find(".row-hint")[0].textContent, /Tap it to start talking.*hold it and talk.*until you let go/);

  // The brains in the owner's order, each online one off, each with what leaves.
  const labels = [...rows.keys()];
  const order = ["1. Built-in model", "2. My local server from chat", "3. A free online model", "4. Claude, with my own key"].map((l) => labels.indexOf(l));
  assert.ok(order.every((at, i) => at >= 0 && (i === 0 || at > order[i - 1])), `the order: ${order}`);
  const on = (label) => rows.get(label).find(".switch")[0].getAttribute("aria-checked");
  for (const label of ["3. A free online model", "4. Claude, with my own key", "2. My local server from chat", "ElevenLabs voice", "Weather by voice", "Look up facts"]) assert.equal(on(label), "false", label);
  assert.equal(on("Esc"), "true");
  assert.match(rows.get("3. A free online model").textContent, /Off until you turn it on.*the words of your question.*are sent to OpenRouter's free models or Google's free tier/);
  assert.match(rows.get("4. Claude, with my own key").textContent, /Off until you turn it on, even if a key is saved for chat/);
  assert.match(rows.get("1. Built-in model").textContent, /About 2\.7 GB to download, and about 4\.3 GB of memory/);
  assert.match(rows.get("Look up facts").textContent, /sent to Wikipedia/);
  assert.match(rows.get("Weather by voice").textContent, /Open-Meteo/);
  // The city waits for its switch.
  assert.equal(rows.get("City").find("input")[0].disabled, true);
  rows.get("Weather by voice").find(".switch")[0].fire("click");
  assert.deepEqual(patches.at(-1), { weather: true });
  await tick();
  assert.equal(rows.get("City").find("input")[0].disabled, false);
  // A switch writes its own setting and nothing else.
  rows.get("3. A free online model").find(".switch")[0].fire("click");
  assert.deepEqual(patches.at(-1), { freeModel: true });

  // What voice never does, and how to stop it.
  assert.deepEqual(page.find("li").map((li) => li.textContent.split(".")[0]), ["Approve or deny a permission", "Send an email", "Delete files, or clear or empty anything", "Spend your plan"]);
  for (const label of ["Esc", "The talk key", "Say “stop”"]) assert.ok(rows.has(label), label);
  // Not built, so not promised (the runner's note).
  assert.doesNotMatch(text, /\bpills?\b|app sounds|ask the chat/i);
  assert.doesNotMatch(page.textContent, /\{\w+\}/, "a placeholder nobody filled in");

  // Downloads: what is there says Ready, the rest offers Download; progress and the check of the files show in the row.
  assert.equal(rows.get("Speech recognition").find(".f-voice-ready").length, 1);
  const natural = rows.get("Natural voice");
  assert.equal(natural.find("button")[0].textContent, "Download");
  natural.find("button")[0].fire("click");
  assert.ok(calls.some(([m]) => m === "speech.download"));
  const tell = (e) => [...listeners.get("download")].forEach((fn) => fn(e));
  tell({ what: "speech", received: 185_560_645, total: 371_121_290, done: false });
  assert.equal(natural.find(".f-voice-count")[0].textContent, "185 MB of 371 MB");
  assert.equal(natural.find(".meter").length, 1);
  assert.equal(rows.get("1. Built-in model").find("button")[0].textContent, "Download", "another download's row is not moved by it");
  tell({ what: "speech", received: 371_121_290, total: 371_121_290, done: false });
  assert.equal(natural.find(".f-voice-count")[0].textContent, "Checking the files…");
  tell({ what: "speech", received: 371_121_290, total: 371_121_290, done: true });
  assert.equal(natural.find(".f-voice-ready").length, 1);
  // Removing it takes a second click.
  const remove = natural.find("button")[0];
  remove.fire("click");
  assert.equal(remove.textContent, "Remove it?");
  assert.ok(!calls.some(([m]) => m === "speech.remove"));
  remove.fire("click");
  assert.ok(calls.some(([m]) => m === "speech.remove"));

  // Try the voice goes through the one mouth of the island's page.
  rows.get("Hear it").find("button")[0].fire("click");
  assert.deepEqual(calls.at(-1), ["caption.act", { what: "try" }]);
});
