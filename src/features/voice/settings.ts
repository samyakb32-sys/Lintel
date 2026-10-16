// The settings page (docs/VOICE.md §9 with the owner's decisions; the first run of §8): how listening
// starts and the talk key, the voice and its download, the brains in the owner's order (on this
// computer first, a free service second, a paid one last, each online one off until switched on, with
// what would leave the computer), weather and lookup, how to stop it, and what voice never does.
// Package voice-ui (VOICE §10; ARCHITECTURE §7). The rules of the key picker and of a download's row
// are in ui/rules.ts (tests/voice-ui.test.mjs).

import { Bridge } from "../../core/bridge";
import { BRAND } from "../../core/brand";
import type { FeatureCtx } from "../../core/feature";
import { btn, card, field, kbd, meter, row, setMeter, toggle } from "../../ui/components";
import { icon } from "../../ui/icons";
import { h } from "../../views/dom";
import type { VoiceDesc, VoiceSettings } from "./index";
import { downloadLook, megabytes, talkKeyName, talkPicker, type DownloadEvent } from "./ui/rules";

/** `crates/voice/src/eleven.rs` reads it from Windows Credential Manager. */
const ELEVEN_KEY = "voice/elevenlabs-key";

/** What the page listens to, as last built: the settings window builds it again on every visit. */
let offs: (() => void)[] = [];

const small = (b: HTMLButtonElement): HTMLButtonElement => (b.classList.add("sm"), b);

/**
 * The talk key as a button, as the Dictation page's "Hold to dictate" (src/features/dictation): a
 * click on it (or on the row's name) listens, the key shows while it is down, letting go saves, Esc
 * cancels, Backspace removes, and a key that cannot be the talk key says why. The rules are `talkPicker`.
 */
function talkKey(cx: FeatureCtx, start: string): { el: HTMLElement; listen: () => void } {
  let key = start;
  let stop: (() => void) | null = null;
  const b = h("button", { class: "btn secondary sm", title: cx.t("Click, then press a key") });
  const clear = small(btn(cx.t("Clear"), "ghost", () => { stop?.(); save(""); }));
  const note = h("span", { class: "f-voice-note-under" });
  const keys = (k: string, empty: string) => b.replaceChildren(k ? kbd(talkKeyName(k)) : h("span", { text: empty }));
  const say = (text: string, warn = false) => { note.textContent = text; note.classList.toggle("warn", warn); };
  const show = () => {
    b.classList.remove("recording");
    keys(key, cx.t("Not set"));
    say("");
    // Not `hidden`: the display of .btn wins over it.
    clear.style.display = key ? "" : "none";
  };
  const save = (k: string) => { key = k; show(); void cx.settings.patch({ key: k }); };
  const listen = () => {
    if (stop) return;
    const pick = talkPicker();
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat) return;
      const r = e.type === "keydown" ? pick.down(e) : pick.up();
      if (r.kind === "held") return keys(r.key, cx.t("Press a key…"));
      if (r.kind === "refused") return say(cx.t(r.why), true);
      end();
      if (r.kind === "cancel") show();
      else save(r.kind === "save" ? r.key : "");
    };
    const outside = (e: Event) => { if (!b.contains(e.target as Node)) cancel(); };
    const end = () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKey, true);
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("blur", cancel);
      stop = null;
      void Bridge.shortcutsSuspend(false);
    };
    const cancel = () => { end(); show(); };
    stop = cancel;
    b.classList.add("recording");
    keys("", cx.t("Press a key…"));
    say(cx.t("Press the key, then let go. Esc cancels, Backspace removes it."));
    // As on the Shortcuts page: the app's own shortcuts and key hooks let go meanwhile, the talk key's too.
    void Bridge.shortcutsSuspend(true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKey, true);
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("blur", cancel);
  };
  b.addEventListener("click", () => (stop ? stop() : listen()));
  show();
  return { el: h("span", { class: "f-voice-side" }, b, clear, note), listen };
}

/** What is fetched by a click, by its name in `voice:download`: the method that tells its state, the one that fetches, the one that deletes. */
const MODELS = {
  whistle: { status: "ears.status", get: "ears.download", remove: "" },
  needle: { status: "needle.status", get: "needle.download", remove: "" },
  speech: { status: "speech.status", get: "speech.download", remove: "speech.remove" },
  generator: { status: "generator.status", get: "generator.download", remove: "generator.remove" },
};

/** A download's control: Download, then a meter with its count, the check of the files, Ready (and Remove), or why it failed. `changed` runs when it arrived or was removed. */
function download(cx: FeatureCtx, what: keyof typeof MODELS, changed?: () => void): HTMLElement {
  const m = MODELS[what];
  const el = h("span", { class: "f-voice-side" });
  let status: { installed: boolean; downloading: boolean } | null = null;
  let last: DownloadEvent | null = null;
  let kind = "";
  let bar: HTMLElement | null = null;
  let count: HTMLElement | null = null;
  const text = (words: string, warn = false) => h("span", { class: warn ? "f-voice-count warn" : "f-voice-count", text: words });
  const read = async () => {
    const r = await cx.call<{ installed?: boolean; model?: boolean; downloading?: boolean }>(m.status);
    // `ears.status` says `model` for what the others call `installed`.
    status = r && { installed: !!(r.installed ?? r.model), downloading: !!r.downloading };
    draw();
  };
  const get = () => {
    last = { what, received: 0, total: 0, done: false };
    draw();
    void cx.call(m.get).then((r) => {
      if (r !== null) return;
      last = { what, received: 0, total: 0, done: true, error: cx.t("The download did not start. Try again in a moment.") };
      draw();
    });
  };
  // Deleting what took minutes to fetch takes a second click.
  const remove = small(btn(cx.t("Remove"), "ghost", () => {
    if (!remove.classList.contains("danger")) {
      remove.className = "btn danger sm";
      remove.children[0].textContent = cx.t("Remove it?");
      return;
    }
    void cx.call(m.remove).then(() => { last = null; return read(); }).then(() => changed?.());
  }));
  const draw = () => {
    const look = downloadLook(status, last);
    if (look.kind === "busy" && kind === "busy" && bar && count) {
      setMeter(bar, look.fraction);
      count.textContent = look.text || cx.t("Starting…");
      return;
    }
    kind = look.kind;
    bar = count = null;
    switch (look.kind) {
      case "get":
        return el.replaceChildren(small(btn(cx.t("Download"), "secondary", get)));
      case "busy":
        bar = meter(look.fraction);
        count = text(look.text || cx.t("Starting…"));
        return el.replaceChildren(bar, count);
      case "check":
        return el.replaceChildren(text(cx.t("Checking the files…")));
      case "ready": {
        remove.className = "btn ghost sm";
        remove.children[0].textContent = cx.t("Remove");
        const ok = h("span", { class: "f-voice-ready" }, icon("check", 14), cx.t("Ready"));
        return m.remove ? el.replaceChildren(ok, remove) : el.replaceChildren(ok);
      }
      case "error":
        return el.replaceChildren(small(btn(cx.t("Try again"), "secondary", get)), text(look.text, true));
    }
  };
  offs.push(cx.on<DownloadEvent>("download", (e) => {
    if (e.what !== what) return;
    last = e;
    draw();
    if (e.done) void read().then(() => changed?.());
  }));
  void read();
  draw();
  return el;
}

function build(cx: FeatureCtx, s: Partial<VoiceSettings>): Node[] {
  const t = cx.t;
  const phrase = `OK ${BRAND.name}`;
  const set = (patch: Partial<VoiceSettings>) => cx.settings.patch(patch);
  /** A switch of one setting; `then` runs once Rust has the new value. */
  const sw = (key: keyof VoiceSettings, then?: (on: boolean) => void) =>
    toggle(s[key] === true, (on) => void set({ [key]: on }).then(() => then?.(on)));
  const select = (value: string, options: [string, string][], onPick: (value: string) => void) => {
    const el = h("select", {}, ...options.map(([v, name]) => h("option", { value: v, text: name })));
    el.value = options.some(([v]) => v === value) ? value : options[0][0];
    el.addEventListener("change", () => onPick(el.value));
    return el;
  };
  const picker = (el: HTMLSelectElement, wide = false) => h("label", { class: wide ? "field f-voice-wide" : "field" }, el, icon("chevd", 14));
  const title = (text: string) => h("div", { class: "section-title", text });
  const always = () => h("span", { class: "f-voice-always", text: t("Always on") });
  /** A line in the warning colour after a row's hint, shown only while `on`. */
  const warning = (line: HTMLElement, text: string) => {
    const el = h("span", { class: "f-voice-warn", text: ` ${text}` });
    line.querySelector(".row-hint")?.append(el);
    return (on: boolean) => void (el.style.display = on ? "" : "none");
  };

  // ── Listening ───────────────────────────────────────────────────────────────
  const wakeLine: Record<string, string> = {
    off: t("The microphone opens only when you press the talk key."),
    on: t("The microphone stays open to hear “{phrase}”. A bar of light at the top of the screen says so.", { phrase }),
    plugged: t("Listens for “{phrase}” on mains power only. On battery, only the talk key.", { phrase }),
  };
  const wake = select(
    s.wake ?? "off",
    [["off", t("Talk key only")], ["on", t("Talk key and “{phrase}”", { phrase })], ["plugged", t("“{phrase}” when plugged in", { phrase })]],
    (v) => { void set({ wake: v as VoiceSettings["wake"] }); wakeHint.textContent = wakeLine[v]; },
  );
  const wakeRow = row(t("How listening starts"), picker(wake, true), wakeLine[wake.value]);
  const wakeHint = wakeRow.querySelector(".row-hint") as HTMLElement;
  // The tray's "Listen for the wake phrase" switches it too.
  offs.push(cx.settings.onChange((v) => {
    const now = String(v.wake ?? "off");
    if (now === wake.value || !(now in wakeLine)) return;
    wake.value = now;
    wakeHint.textContent = wakeLine[now];
  }));
  const key = talkKey(cx, typeof s.key === "string" ? s.key : "");
  const keyRow = row(t("Talk key"), key.el,
    t("Tap it to start talking. Or hold it and talk: it listens until you let go. The key still does its normal job too, so pick one that does nothing on its own."));
  // The name of the row starts it too, as a label does for its field.
  keyRow.querySelector(".row-label")?.addEventListener("click", key.listen);

  // ── Voice ───────────────────────────────────────────────────────────────────
  const voice = h("select", {});
  let chosen = typeof s.voice === "string" ? s.voice : "";
  voice.addEventListener("change", () => void set({ voice: (chosen = voice.value) }));
  const engines: Record<VoiceDesc["engine"], string> = { kokoro: t("natural"), windows: "Windows", eleven: "ElevenLabs" };
  const loadVoices = async () => {
    const list = (await cx.call<VoiceDesc[]>("mouth.voices")) ?? [];
    voice.replaceChildren(
      h("option", { value: "", text: t("Automatic") }),
      ...list.map((v) => h("option", { value: v.id, text: `${v.name} · ${engines[v.engine] ?? v.engine}` })),
    );
    // A voice that is gone (its download removed, its switch off) reads Automatic; the setting is left as it is.
    voice.value = list.some((v) => v.id === chosen) ? chosen : "";
  };
  void loadVoices();
  const hear = btn(t("Try the voice"), "secondary", () => void cx.call("caption.act", { what: "try" }));
  hear.disabled = s.speak === false;

  const secret = field({ type: "password" });
  const secretNote = h("span", { class: "f-voice-note-under" });
  /** Asks the account for its voices (only while its switch is on), then lists them. */
  const elevenVoices = () => cx.call("eleven.voices").then(loadVoices);
  const secretState = async () => {
    const has = (await Bridge.secretPresent(ELEVEN_KEY)) ?? false;
    secret.input.placeholder = has ? t("A key is saved") : t("Paste your key");
    forget.style.display = has ? "" : "none";
  };
  const keep = async (work: () => Promise<void>, failed: (error: string) => string) => {
    secretNote.textContent = "";
    secretNote.classList.remove("warn");
    try {
      await work();
      secret.input.value = "";
      await secretState();
      void elevenVoices();
    } catch (err) {
      secretNote.classList.add("warn");
      secretNote.textContent = failed(String(err));
    }
  };
  const store = small(btn(t("Save"), "secondary", () => {
    const value = secret.input.value.trim();
    if (value) void keep(() => Bridge.secretSet(ELEVEN_KEY, value), (error) => t("Could not save: {error}", { error }));
  }));
  const forget = small(btn(t("Remove"), "ghost", () => void keep(() => Bridge.secretClear(ELEVEN_KEY), (error) => t("Could not remove: {error}", { error }))));
  void secretState();

  // ── Answers ─────────────────────────────────────────────────────────────────
  const loaded = select(
    String(s.keepModel ?? 10),
    [["3", t("3 minutes")], ["10", t("10 minutes")], ["30", t("30 minutes")], ["60", t("1 hour")]],
    (v) => void set({ keepModel: Number(v) }),
  );
  const online = { free: s.freeModel === true, claude: s.claude === true };
  const freeRow = row(t("3. A free online model"), sw("freeModel", (on) => { online.free = on; void keys(); }),
    t("Off until you turn it on. Then the words of your question, and the few before it, are sent to OpenRouter's free models or Google's free tier, using the key you saved under Chat. Your voice and your files are never sent. A free service may keep what it receives."));
  const claudeRow = row(t("4. Claude, with my own key"), sw("claude", (on) => { online.claude = on; void keys(); }),
    t("Off until you turn it on, even if a key is saved for chat. Then the words of your question, and the few before it, are sent to Anthropic and charged to your key."));
  const freeNeeds = warning(freeRow, t("No OpenRouter or Google key is saved yet: add one under Chat."));
  const claudeNeeds = warning(claudeRow, t("No Claude key is saved yet: add one under Chat."));
  /** A switch that is on without its key answers nothing: the row says so. */
  const keys = async () => {
    const has = (name: string) => Bridge.secretPresent(name).then((yes) => yes === true);
    const [claude, openrouter, google] = await Promise.all(["anthropic-api-key", "openrouter-api-key", "google-api-key"].map(has));
    freeNeeds(online.free && !openrouter && !google);
    claudeNeeds(online.claude && !claude);
  };
  freeNeeds(false);
  claudeNeeds(false);
  void keys();

  // ── From the internet ───────────────────────────────────────────────────────
  let saved = String(s.city ?? "");
  const city = field({ value: saved, placeholder: t("Your city") });
  const cityNote = h("span", { class: "f-voice-note-under" });
  // The search is refused while the switch is off (crates/voice/src/weather.rs): the field waits for it.
  city.input.disabled = s.weather !== true;
  city.input.addEventListener("change", async () => {
    const name = city.input.value.trim();
    if (name === saved) return;
    saved = name;
    cityNote.textContent = "";
    await set({ city: name });
    if (!name) return;
    const found = await cx.call<{ name: string; country: string }[]>("weather.find", { city: name });
    if (saved !== name) return;
    cityNote.classList.toggle("warn", !found?.length);
    cityNote.textContent = found?.length
      ? [found[0].name, found[0].country].filter(Boolean).join(", ")
      : found ? t("No city by that name was found.") : t("The city could not be looked up just now.");
  });

  return [
    h("p", { class: "f-voice-lead", text: t("Talk to {name} and it answers out loud. The sound of your voice stays in this computer's memory: it is never saved and never sent anywhere.", { name: BRAND.name }) }),

    title(t("Listening")),
    card([
      wakeRow,
      keyRow,
      row(t("Speech recognition"), download(cx, "whistle"), t("Turns what you say into text, on this computer. About 19 MB. Voice needs it before anything else.")),
      row(t("Command model"), download(cx, "needle"), t("Understands a command said in your own words, on this computer. About 35 MB.")),
    ]),

    title(t("Voice")),
    card([
      row(t("Speak answers"), sw("speak", (on) => (hear.disabled = !on)), t("Off: answers are only shown, in the caption.")),
      row(t("Voice"), picker(voice), t("Automatic is the natural voice once it is downloaded, and a Windows voice until then.")),
      row(t("Natural voice"), download(cx, "speech", () => void loadVoices()),
        t("Kokoro, on this computer. About {size}; every file is checked before it is used.", { size: megabytes(371_121_290) })),
      row(t("ElevenLabs voice"), sw("eleven", (on) => void (on ? elevenVoices() : loadVoices())),
        t("Off until you turn it on, and it needs your own key. Then every sentence it speaks is sent to ElevenLabs.")),
      row(t("ElevenLabs key"), h("span", { class: "f-voice-side" }, secret, store, forget, secretNote), t("Kept in Windows Credential Manager, never in a file.")),
      row(t("Hear it"), hear, t("Says one sentence with the voice chosen above.")),
      row(t("Captions"), sw("captions"), t("What was heard and the answer, in a small capsule under the island. Off: only its microphone dot and stop button.")),
      row(t("Lower the music while it talks"), sw("duck")),
      row(t("Read chat answers aloud"), sw("chatAloud")),
    ]),

    title(t("Answers")),
    h("p", { class: "f-voice-lead", text: t("A command never leaves this computer. An open question goes to the first of these that is switched on and ready, in this order.") }),
    card([
      row(t("1. Built-in model"), download(cx, "generator"),
        t("Qwen3.5-4B, on this computer. About 2.7 GB to download, and about 4.3 GB of memory while it is loaded.")),
      row(t("Keep it loaded"), picker(loaded), t("It loads with the first question and leaves memory this long after the last one.")),
      row(t("2. My local server from chat"), sw("localServer"), t("The Ollama, LM Studio or other server you set up under Chat. Nothing leaves this computer.")),
      freeRow,
      claudeRow,
    ]),

    title(t("From the internet")),
    card([
      row(t("Weather by voice"), sw("weather", (on) => (city.input.disabled = !on)),
        t("Off until you turn it on. Then the name of your city, and nothing else, goes to Open-Meteo, a free weather service.")),
      row(t("City"), h("span", { class: "f-voice-side" }, city, cityNote), t("Can be set once weather by voice is on. Type it and press Enter.")),
      row(t("Look up facts"), sw("lookup"),
        t("Off until you turn it on. Then the words of a question such as “who was Ada Lovelace” are sent to Wikipedia, and the answer begins “According to Wikipedia”.")),
    ]),

    title(t("Stopping it")),
    card([
      row(t("Esc"), sw("esc"), t("Ends the turn at once, whatever it is doing. The key still reaches the window you are in.")),
      row(t("The talk key"), always(), t("Stops what it is saying and listens again.")),
      row(t("Say “stop”"), always(), t("Works while it is talking.")),
    ]),

    title(t("What voice never does")),
    card(
      h("ul", { class: "f-voice-never" },
        ...[
          t("Approve or deny a permission. It reads the request aloud and says to click."),
          t("Send an email. It gets the message ready; Send is your click."),
          t("Delete files, or clear or empty anything."),
          t("Spend your plan. It never prompts or starts a coding agent."),
        ].map((line) => h("li", {}, icon("lock", 14), h("span", { text: line }))),
      ),
    ),
  ];
}

export function settingsPage(cx: FeatureCtx): HTMLElement {
  for (const off of offs.splice(0)) off();
  const el = h("div", { class: "f-voice-settings" });
  void cx.settings.get().then((s) => el.replaceChildren(...build(cx, s as Partial<VoiceSettings>)));
  return el;
}
