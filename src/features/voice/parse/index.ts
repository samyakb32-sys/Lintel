// The parser (docs/VOICE.md §4.1): the heard text is prepared, split into commands, and each is
// scored against the forms of every action; slots are read, choices resolved against their live
// lists. Actions are data: nothing here knows a feature. Pure, so `node --test` runs it against a
// fixture registry (never import ../registry.ts or src/core/registry.ts from here).
// Ported in idea from Voice/{IntentParser,EntityResolver,ConversationContext}.swift (normalise,
// fillers, multi-action split, 90 s follow-ups, yes/no, entity resolution); the matcher is new.

import type { VoiceArg, VoiceArgs, VoiceChoice, VoiceCx, VoiceEntry } from "../../../core/feature";

/** A command that was understood. */
export interface Parsed {
  entry: VoiceEntry;
  args: VoiceArgs;
  /** Required arguments nobody said, in the action's order: the runner asks for them, at most twice each. */
  missing: VoiceArg[];
  /** Literal words of the form that matched. */
  score: number;
}

/** What the turns before left for a follow-up ("and 5 more minutes"); `at` is `Date.now()` of the last one, stale after 90 s. */
export interface Conversation {
  last: Parsed | null;
  at: number;
}

/** The `VoiceCx` of an entry's own feature (its `choices` are asked through it). */
export type CxOf = (entry: VoiceEntry) => VoiceCx;

const FOLLOW_MS = 90_000;

// ── Preparing the text ────────────────────────────────────────────────────────

/** Lowercase, punctuation to spaces, fillers and politeness dropped, the speech model's homophones snapped ("said a timer" → "set a timer"). */
export function normalise(text: string): string {
  let s = text.toLowerCase().replace(/[’`]/g, "'");
  s = s.replace(/\bper ?cent\b|\bpercent\b|%/g, " % ").replace(/'/g, "");
  // ":" and "." between digits belong to times and decimals; everything else that is not a word goes.
  s = s.replace(/(\d)[:](?=\d)/g, "$1\u0001").replace(/(\d)[.](?=\d)/g, "$1\u0002");
  s = s.replace(/[^a-z0-9% \u0001\u0002]+/g, " ");
  s = s.replace(/(\d)([a-z])/g, "$1 $2").replace(/([a-z])(\d)/g, "$1 $2");
  s = s.replace(/\u0001/g, ":").replace(/\u0002/g, ".");
  s = s.replace(/\b([ap]) m\b/g, "$1m");
  s = s.replace(/\b(?:please|um+|uh+|uhm|erm?|hmm+|hey|ok meteor|okay meteor|ok lintel|okay lintel)\b/g, " ");
  s = s.replace(/\b(?:can|could|would|will) you\b/g, " ");
  s = s.replace(/\bto (morrow|day|night)\b/g, "to$1");
  // One word the speech model wrote as two (seen in the app: "change the wall paper", "on my desk top").
  s = s.replace(/\b(?:wall paper|desk top|clip board|stop watch)\b/g, (m) => m.replace(" ", ""));
  // Two words it misspelt in the app, on both test voices ("Delite the files", "Delied my files", "cancel the timeer").
  s = s.replace(/\b(?:delite|delied)\b/g, "delete").replace(/\btimeer\b/g, "timer");
  s = s.replace(/\s+/g, " ").trim();
  s = s.replace(/(?<=\S) (?:for me|thanks|thank you)$/, "");
  // Whistle's homophones at the start of a command.
  s = s.replace(/^(?:said|sent|senator|sad|sat) (?=(?:(?:a|an|the|my|timer|volume|alarm|reminder|brightness)\b|\d))/, "set ");
  s = s.replace(/^(?:both|paws|pores) (?=(?:the|it|this|that|my|music|media|song|video|playback)\b)/, "pause ");
  s = s.replace(/^(?:both|paws)$/, "pause");
  return s;
}

const words = (s: string): string[] => (s ? s.split(" ") : []);

// A word that opens a command: a second one after "and"/"then"/a pause starts a new command.
const VERBS = new Set(
  ("set start open pause play stop mute unmute turn volume remind add show skip next previous resume keep close switch " +
    "take read lower raise increase decrease toggle enable disable launch hide lock minimise minimize silence snooze").split(" "),
);
const JOIN = new Set(["and", "then", "also", "plus"]);

/** Several commands in one sentence, in the order they were said. One command comes back alone. */
export function splitCommands(text: string): string[] {
  const toks = [...text.matchAll(/\S+/g)];
  const bare = (t: string) => t.toLowerCase().replace(/[^a-z0-9]/g, "");
  const cuts: [number, number][] = []; // [start of the cut, start of the next command]
  for (let k = 1; k < toks.length; k++) {
    let j = k;
    const pause = /[,.;!?]$/.test(toks[k - 1][0]);
    const joined = JOIN.has(bare(toks[j][0]));
    if (joined) while (j + 1 < toks.length && JOIN.has(bare(toks[j + 1][0]))) j++;
    const next = joined ? j + 1 : k;
    if (!(joined || pause) || next >= toks.length || !VERBS.has(bare(toks[next][0]))) continue;
    cuts.push([toks[k].index!, toks[next].index!]);
    k = next;
  }
  const out: string[] = [];
  let from = 0;
  for (const [cut, next] of cuts) {
    out.push(text.slice(from, cut));
    from = next;
  }
  out.push(text.slice(from));
  return out.map((p) => p.trim().replace(/[,.;!?\s]+$/, "")).filter(Boolean);
}

const fullOf = (alts: string) => new RegExp(`^(?:(?:${alts})\\b ?)+$`);
const YES = fullOf("yes|yeah|yep|yup|yea|sure|ok|okay|correct|right|affirmative|absolutely|definitely|confirm|confirmed|proceed|go ahead|do it|thats right|sounds good|why not|of course");
const NO = fullOf("no|nope|nah|negative|never|not now|not really|dont do it|do not do it|dont|do not|i dont think so");
const CANCEL = fullOf("cancel|cancel that|cancel it|never mind|nevermind|forget it|forget that|abort|scrap that|drop it|leave it|no cancel");
const END = fullOf("no|ok|okay|alright|thank you|thanks|thats all|thats it|thats everything|goodbye|good bye|bye|bye bye|see you|see ya|im done|were done|all done|nothing else|nothing more|that will be all|stop listening|go to sleep|dismiss");

export function isYes(text: string): boolean {
  return YES.test(normalise(text));
}

export function isNo(text: string): boolean {
  return NO.test(normalise(text));
}

/** "Cancel", "never mind": the question or the turn is dropped. */
export function isCancel(text: string): boolean {
  return CANCEL.test(normalise(text));
}

/** "That's all", "goodbye": the conversation ends. */
export function isEnd(text: string): boolean {
  const s = normalise(text);
  // "ok", "no" and "thanks" alone are answers, not goodbyes: one real end phrase is needed.
  return END.test(s) && /\b(?:thats|goodbye|good bye|bye|see|im done|were done|all done|nothing|that will|stop listening|go to sleep|dismiss|thank)/.test(s);
}

// ── Numbers, minutes, percent, time ───────────────────────────────────────────

interface Cand {
  v: string | number;
  n: number;
}

const SMALL = new Map(
  ("zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen")
    .split(" ")
    .map((w, i) => [w, i] as const),
);
const TENS = new Map("twenty thirty forty fifty sixty seventy eighty ninety".split(" ").map((w, i) => [w, (i + 2) * 10] as const));
const UNIT = new Map<string, number>([
  ...["hour", "hours", "hr", "hrs", "h"].map((w) => [w, 60] as const),
  ...["minute", "minutes", "min", "mins", "m"].map((w) => [w, 1] as const),
  ...["second", "seconds", "sec", "secs", "s"].map((w) => [w, 1 / 60] as const),
]);

function tailNum(w: string[], j: number): { v: number; n: number } | null {
  const ten = TENS.get(w[j]);
  if (ten !== undefined) {
    const u = SMALL.get(w[j + 1]);
    return u !== undefined && u >= 1 && u <= 9 ? { v: ten + u, n: 2 } : { v: ten, n: 1 };
  }
  const s = SMALL.get(w[j]);
  return s === undefined ? null : { v: s, n: 1 };
}

/** Digits or number words up to 999 ("one hundred and five", "a hundred"). */
function wordsToNumber(w: string[], i: number): { v: number; n: number } | null {
  const t = w[i];
  if (t === undefined) return null;
  if (/^\d+(\.\d+)?$/.test(t)) return { v: Number(t), n: 1 };
  let j = i;
  let v = 0;
  let got = false;
  const h = t === "a" ? 1 : SMALL.get(t);
  if (h !== undefined && h >= 1 && h <= 9 && w[i + 1] === "hundred") {
    v = h * 100;
    j += 2;
    got = true;
    if (w[j] === "and" && tailNum(w, j + 1)) j++;
  }
  const r = tailNum(w, j);
  if (r) {
    v += r.v;
    j += r.n;
    got = true;
  }
  return got ? { v, n: j - i } : null;
}

function readNumber(w: string[], i: number): Cand[] {
  const r = wordsToNumber(w, i);
  if (r) return [{ v: r.v, n: r.n }];
  if (w[i] === "a" && w[i + 1] === "couple") return [{ v: 2, n: w[i + 2] === "of" ? 3 : 2 }];
  if (w[i] === "a" || w[i] === "an") return [{ v: 1, n: 1 }];
  return [];
}

const SPECIAL_HOURS: [RegExp, number][] = [
  [/^(?:a )?half (?:of )?(?:an |a )?hour/, 30],
  [/^(?:a )?quarter (?:of )?(?:an |a )?hour/, 15],
  [/^three quarters (?:of )?(?:an |a )?hour/, 45],
];

/** Minutes, as a number (90 seconds is 1.5): "12", "5 minutes", "an hour and a half", "2 h 15". */
function readMinutes(w: string[], i: number): Cand[] {
  const rest = w.slice(i).join(" ");
  for (const [re, v] of SPECIAL_HOURS) {
    const m = re.exec(rest);
    if (m) return [{ v, n: m[0].split(" ").length }];
  }
  let j = i;
  let total = 0;
  let last = 0;
  let any = false;
  for (;;) {
    const s = any && w[j] === "and" ? j + 1 : j;
    if (any && w[s] === "a" && w[s + 1] === "half" && last) {
      total += 0.5 * last;
      j = s + 2;
      break;
    }
    let num = wordsToNumber(w, s);
    if (!num && (w[s] === "a" || w[s] === "an") && UNIT.has(w[s + 1])) num = { v: 1, n: 1 };
    if (!num && w[s] === "a" && w[s + 1] === "couple") num = { v: 2, n: w[s + 2] === "of" ? 3 : 2 };
    if (!num) break;
    let k = s + num.n;
    let v = num.v;
    if (w[k] === "and" && w[k + 1] === "a" && w[k + 2] === "half" && UNIT.has(w[k + 3])) {
      v += 0.5;
      k += 3;
    }
    const f = UNIT.get(w[k]);
    if (f === undefined) {
      if (!any) {
        total = v;
        j = k;
        any = true;
      } else if (last === 60) {
        total += v; // "2 h 15"
        j = k;
      }
      break;
    }
    total += v * f;
    j = k + 1;
    last = f;
    any = true;
  }
  return any ? [{ v: total, n: j - i }] : [];
}

function readPercent(w: string[], i: number): Cand[] {
  let r = wordsToNumber(w, i);
  if (!r && ["full", "max", "maximum"].includes(w[i])) r = { v: 100, n: 1 };
  if (!r || r.v > 100) return [];
  return [{ v: r.v, n: r.n + (w[i + r.n] === "%" ? 1 : 0) }];
}

const pad = (n: number) => String(n).padStart(2, "0");

/** "HH:MM" on the 24-hour clock: "5 pm", "5:30 pm", "17:30", "five thirty", "half past five", "quarter to 6", "noon". */
function readTime(w: string[], i: number): Cand[] {
  let j = i;
  if (w[j] === "at") j++;
  const t = w[j];
  if (t === "noon") return [{ v: "12:00", n: j + 1 - i }];
  if (t === "midnight") return [{ v: "00:00", n: j + 1 - i }];
  let h: number;
  let m = 0;
  let sub = false;
  let explicit = false; // 24-hour reading, no am/pm guess
  const colon = /^(\d{1,2}):(\d{2})$/.exec(t ?? "");
  if (colon) {
    h = Number(colon[1]);
    m = Number(colon[2]);
    explicit = h === 0 || h >= 13;
    j++;
  } else if (t === "half" || t === "quarter" || (wordsToNumber(w, j) && ["past", "to"].includes(w[j + (wordsToNumber(w, j)?.n ?? 0)]))) {
    let off: number;
    if (t === "half" || t === "quarter") {
      off = t === "half" ? 30 : 15;
      j++;
    } else {
      const x = wordsToNumber(w, j)!;
      off = x.v;
      j += x.n;
    }
    const dir = w[j];
    if (dir !== "past" && dir !== "to") return [];
    const y = wordsToNumber(w, j + 1);
    if (!y || off >= 60) return [];
    h = y.v;
    m = off;
    j += 1 + y.n;
    if (dir === "to") {
      sub = true;
      m = 60 - off;
    }
  } else {
    const x = wordsToNumber(w, j);
    if (!x) return [];
    h = x.v;
    j += x.n;
    if (w[j] === "oclock") j++;
    else if (w[j] === "o" && w[j + 1] === "clock") j += 2;
    else {
      const oh = w[j] === "oh" || w[j] === "o";
      const y = wordsToNumber(w, oh ? j + 1 : j);
      if (y && (oh ? y.v >= 1 && y.v <= 9 : y.v >= 10 && y.v <= 59)) {
        m = y.v;
        j += (oh ? 1 : 0) + y.n;
      }
    }
  }
  let mer: "am" | "pm" | "" = "";
  if (w[j] === "am" || w[j] === "pm") {
    mer = w[j] as "am" | "pm";
    j++;
  } else if (w[j] === "in" && w[j + 1] === "the" && ["morning", "afternoon", "evening"].includes(w[j + 2])) {
    mer = w[j + 2] === "morning" ? "am" : "pm";
    j += 3;
  } else if (w[j] === "at" && w[j + 1] === "night") {
    mer = "pm";
    j += 2;
  }
  if (h > 23 || m > 59 || (mer && h > 12) || (mer && h < 1)) return [];
  if (mer === "pm" && h < 12) h += 12;
  else if (mer === "am" && h === 12) h = 0;
  else if (!mer && !explicit && h >= 1 && h <= 6) h += 12; // ponytail: a bare "5" is 17:00, 7 to 11 are morning; a clock-aware guess if it matters
  if (sub) h = (h + 23) % 24;
  return [{ v: `${pad(h)}:${pad(m)}`, n: j - i }];
}

// ── Choices: a spoken name against a live list ────────────────────────────────

const stripArticle = (s: string) => s.replace(/^(?:(?:the|my|a|an|to|on) )+/, "").trim();

function distance(a: string, b: string): number {
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    const row = [i];
    for (let k = 1; k <= b.length; k++) {
      row[k] = Math.min(prev[k] + 1, row[k - 1] + 1, prev[k - 1] + (a[i - 1] === b[k - 1] ? 0 : 1));
    }
    prev = row;
  }
  return prev[b.length];
}

/** What was said against a live list: alias, exact name, one whole-word match, then a near miss unless two are as near. */
export function resolveChoice(spoken: string, choices: readonly VoiceChoice[]): VoiceChoice | null {
  const s = stripArticle(normalise(spoken));
  if (!s) return null;
  const forms = choices.map((c) => ({ c, say: c.say.map((x) => stripArticle(normalise(x))).filter(Boolean) }));
  const only = (hits: typeof forms) => (hits.length === 1 ? hits[0].c : null);
  const exact = forms.filter((f) => f.say.includes(s));
  if (exact.length) return only(exact);
  const sw = words(s);
  const whole = forms.filter((f) => f.say.some((x) => sw.every((w) => words(x).includes(w))));
  if (whole.length) return only(whole);
  const inside = forms.filter((f) => f.say.some((x) => ` ${s} `.includes(` ${x} `)));
  if (inside.length) return only(inside);
  const tol = s.length <= 4 ? 1 : 2;
  const near = forms
    .map((f) => ({ c: f.c, d: Math.min(...f.say.map((x) => distance(s, x)), Infinity) }))
    .filter((f) => f.d <= tol)
    .sort((a, b) => a.d - b.d);
  return near.length === 1 ? near[0].c : null;
}

// ── Templates ─────────────────────────────────────────────────────────────────

type Node =
  | { t: "w"; w: string }
  | { t: "s"; name: string }
  | { t: "o"; body: Node[] }
  | { t: "a"; alts: Node[][] };

const compiled = new Map<string, Node[]>();

/** `"set a timer for {minutes}"`: words in order, `[x]` optional, `(a|b)` alternatives, `{name}` a slot. */
function compile(form: string): Node[] {
  const hit = compiled.get(form);
  if (hit) return hit;
  const toks = form.match(/[[\]()|]|\{[^}]+\}|[^\s[\]()|{}]+/g) ?? [];
  let p = 0;
  const seq = (end: string[]): Node[] => {
    const out: Node[] = [];
    while (p < toks.length && !end.includes(toks[p])) {
      const t = toks[p++];
      if (t === "[" || t === "(") {
        const close = t === "[" ? "]" : ")";
        const alts = [seq(["|", close])];
        while (toks[p] === "|") {
          p++;
          alts.push(seq(["|", close]));
        }
        p++;
        const one: Node = { t: "a", alts };
        out.push(t === "(" ? one : { t: "o", body: alts.length > 1 ? [one] : alts[0] });
      } else if (t[0] === "{") out.push({ t: "s", name: t.slice(1, -1) });
      else out.push({ t: "w", w: t.toLowerCase().replace(/'/g, "") });
    }
    return out;
  };
  const nodes = seq([]);
  compiled.set(form, nodes);
  return nodes;
}

interface Ctx {
  w: string[];
  args: Map<string, VoiceArg>;
  lists: Map<VoiceArg, readonly VoiceChoice[]>;
}
interface St {
  i: number;
  score: number;
  vals: Record<string, string | number>;
  text: boolean;
}

function cands(arg: VoiceArg, x: Ctx, i: number): Cand[] {
  const w = x.w;
  switch (arg.kind) {
    case "number":
      return readNumber(w, i);
    case "minutes":
      return readMinutes(w, i);
    case "percent":
      return readPercent(w, i);
    case "time":
      return readTime(w, i);
    case "text": {
      const out: Cand[] = [];
      for (let n = w.length - i; n >= 1; n--) out.push({ v: w.slice(i, i + n).join(" "), n });
      return out;
    }
    case "choice": {
      const list = x.lists.get(arg) ?? [];
      const out: Cand[] = [];
      for (let n = Math.min(6, w.length - i); n >= 1; n--) {
        const c = resolveChoice(w.slice(i, i + n).join(" "), list);
        if (c) out.push({ v: c.id, n });
      }
      return out;
    }
  }
}

/** Every way the nodes can match the whole text. */
function* go(nodes: Node[], st: St, x: Ctx): Generator<St> {
  if (!nodes.length) {
    if (st.i === x.w.length) yield st;
    return;
  }
  const [n, ...rest] = nodes;
  if (n.t === "w") {
    if (x.w[st.i] === n.w) yield* go(rest, { ...st, i: st.i + 1, score: st.score + 1 }, x);
  } else if (n.t === "o") {
    yield* go([...n.body, ...rest], st, x);
    yield* go(rest, st, x);
  } else if (n.t === "a") {
    for (const a of n.alts) yield* go([...a, ...rest], st, x);
  } else {
    const arg = x.args.get(n.name);
    if (!arg) return;
    for (const c of cands(arg, x, st.i)) {
      yield* go(rest, { ...st, i: st.i + c.n, vals: { ...st.vals, [n.name]: c.v }, text: st.text || arg.kind === "text" }, x);
    }
  }
}

/** The text slot's words in the case they were said. */
function originalCase(raw: string, value: string): string {
  const esc = (c: string) => c.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const re = value
    .split(" ")
    .map((w) => [...w].map(esc).join("['’]?"))
    .join("[^a-z0-9]+");
  return new RegExp(re, "i").exec(raw)?.[0] ?? value;
}

interface Hit {
  score: number;
  filled: number;
  vals: Record<string, string | number>;
}

function bestOf(entry: VoiceEntry, x: Ctx, raw: string): Hit | null {
  let best: Hit | null = null;
  for (const form of entry.action.forms) {
    for (const st of go(compile(form), { i: 0, score: 0, vals: {}, text: false }, x)) {
      // One literal word is enough when the form swallows no free text ("pause", "volume 30"); a text slot needs two.
      if (st.score < (st.text ? 2 : 1)) continue;
      const filled = Object.keys(st.vals).length;
      if (!best || st.score > best.score || (st.score === best.score && filled > best.filled)) best = { score: st.score, filled, vals: st.vals };
    }
  }
  if (best) {
    const vals = { ...best.vals };
    for (const a of entry.action.args ?? []) if (a.kind === "text" && typeof vals[a.name] === "string") vals[a.name] = originalCase(raw, vals[a.name] as string);
    best.vals = vals;
  }
  return best;
}

const wordSet = (entry: VoiceEntry) =>
  new Set(entry.action.forms.flatMap((f) => (f.match(/[^\s[\]()|{}]+/g) ?? []).map((t) => t.toLowerCase().replace(/'/g, ""))));

/**
 * One command against the registry. Null is a miss: under two literal words matched when the form
 * takes free text, or a tie between two actions. `context` lets a follow-up lean on the command before it.
 */
export async function parse(text: string, entries: readonly VoiceEntry[], cx: CxOf, context?: Conversation): Promise<Parsed | null> {
  const w = words(normalise(text));
  if (!w.length) return null;
  const lists = new Map<VoiceArg, readonly VoiceChoice[]>();
  const heard = new Set(w);
  await Promise.all(
    entries.map(async (e) => {
      const has = [...wordSet(e)].some((t) => heard.has(t));
      if (!has) return;
      for (const a of e.action.args ?? []) {
        if (a.kind !== "choice" || !a.choices) continue;
        try {
          lists.set(a, await a.choices(cx(e)));
        } catch {
          lists.set(a, []);
        }
      }
    }),
  );
  const hits: { entry: VoiceEntry; hit: Hit }[] = [];
  for (const entry of entries) {
    const x: Ctx = { w, args: new Map((entry.action.args ?? []).map((a) => [a.name, a])), lists };
    const hit = bestOf(entry, x, text);
    if (hit) hits.push({ entry, hit });
  }
  hits.sort((a, b) => b.hit.score - a.hit.score || b.hit.filled - a.hit.filled);
  if (hits.length) {
    const [a, b] = hits;
    if (b && a.hit.score === b.hit.score && a.hit.filled === b.hit.filled) return null;
    const missing = (a.entry.action.args ?? []).filter((g) => !g.optional && !(g.name in a.hit.vals));
    return { entry: a.entry, args: a.hit.vals, missing, score: a.hit.score };
  }
  return context?.last && Date.now() - context.at <= FOLLOW_MS ? followUp(text, context.last, cx) : null;
}

const LEAD = /^(?:(?:and|also|then|now|no|make it|make that|set it to|change it to|change that to|what about|how about|and what about) )+/;

/** "and 5 more minutes", "make it 40", "and the speakers": the command before, with one argument changed. */
async function followUp(text: string, last: Parsed, cx: CxOf): Promise<Parsed | null> {
  let s = normalise(text).replace(LEAD, "").replace(/ (?:instead|too|as well)$/, "");
  const more = /\bmore\b/.test(s);
  s = s.replace(/\bmore\b/, "").replace(/\s+/g, " ").trim();
  if (!s) return null;
  for (const arg of last.entry.action.args ?? []) {
    if (arg.kind === "text") continue;
    const v = await readArg(arg, s, cx(last.entry));
    if (v === null) continue;
    const old = last.args[arg.name];
    const next = more && typeof v === "number" && typeof old === "number" ? old + v : v;
    const args = { ...last.args, [arg.name]: next };
    return { ...last, args, missing: (last.entry.action.args ?? []).filter((g) => !g.optional && !(g.name in args)) };
  }
  return null;
}

const ARG_LEAD = /^(?:(?:for|to|at|in|about|around|make it|set it to|its|it is|the) )+/;

/** The reply to an action's question, read as that argument (a number, minutes, a time, a choice…). Null when it is not one. */
export async function readArg(arg: VoiceArg, text: string, v: VoiceCx): Promise<string | number | null> {
  if (arg.kind === "text") return text.trim() || null;
  const s = normalise(text).replace(ARG_LEAD, "");
  if (!s) return null;
  if (arg.kind === "choice") return resolveChoice(s, arg.choices ? await arg.choices(v) : [])?.id ?? null;
  const w = words(s);
  const x: Ctx = { w, args: new Map(), lists: new Map() };
  return cands(arg, x, 0).find((c) => c.n === w.length)?.v ?? null;
}
