# METEOR V1 — voice assistant

The voice assistant is built and part of 1.0.0, off by default; sections 10 and 11 say what is proven. Evidence: a study of upstream 0.3.0 and a spike with the Needle engine and the Whistle model on the development laptop (the notes are not part of this repository). *est.* = not measured.

**Rules.** Every stage: local first, a free service second if quality holds, a paid service only as an option the user turns on with their own key; nothing paid is required or on by default. English only. No telemetry, no account. Voice never approves a permission, sends an email, deletes anything or spends the user's plan by itself: it prepares, shows and asks; the click finishes. The microphone is used only while voice is on, with a visible indicator the whole time it is open. Windows 10/11, unelevated, unpackaged. Development laptop: i7-1260P, 32 GB, Iris Xe, a battery: everything runs on the CPU.

## 1. What the user gets

- **Two ways in.** The talk shortcut (tap: it listens until you stop; hold: while held). Or, behind its own switch, **"OK METEOR"** and the command, in one breath or after the blip.
- **What you can say.**
  - Time and sound: "set a timer for 12 minutes", "how long is left", "pause", "skip", "volume 30", "mute Spotify".
  - PC and island: "keep awake for two hours", "brightness 60", "dark mode", "open Figma", "open the clipboard".
  - Notes: "remind me to call the dentist", "note: renew the passport", "what's next today".
  - Agents: "what are my agents doing", "what is it asking", "jump to the terminal", "how much of my plan is left".
  - Questions and writing: "what's the weather tomorrow", "what is a mutex", "write an email to Tana saying I'm late".
  - Follow-ups: "and 5 more minutes", "undo", "cancel".
- **What it will not do.** Allow a permission, send a mail, delete or empty anything, start the camera, paste without a yes, unmute the microphones, prompt an agent. It opens the right card and says "it's ready, click to finish".
- **How it looks.** The strip slides out, the Visor's bar of light becomes a small waveform that follows your voice, a capsule under the island shows what was heard and the answer. While the wake phrase is on, a 3 px bar of light rests at the top centre of the screen: the microphone indicator.
- **How it sounds.** A blip when it listens, an answer in a natural on-device voice (Kokoro) or a Windows voice, sentence by sentence. Esc, the shortcut or the wake phrase stops it.
- **What stays on the PC.** Audio, always. Speech to text, the rules, the action model, the voice and the small text model run here. Text leaves only to a service you switched on, only for the turns that need it.

## 2. Parity with upstream 0.3.0

| Upstream | METEOR V1 | How / why |
|---|---|---|
| Wake phrase "OK/hey Coucou" | adapted | "OK METEOR": Whistle on voice segments, keyword biasing, variant matcher (§6); off by default |
| Push-to-talk | same | bound by default; tap and hold |
| Energy voice detector, 0.5 s pre-roll, stall watchdog | same | `EnergyVAD` ported in seconds |
| Pause on lock, sleep, low power, dictation | adapted | plus tray Pause, full-screen app, microphones muted |
| Live partial words | adapted | no stream: the buffer is transcribed again about once a second |
| End of turn 1.6 / 2.6 / 3.2 s | adapted | same values; the last word comes from a provisional transcript |
| Cancel and end phrases, follow-up only when it asked, 90 s context, undo, several actions in a sentence, entity resolver | same | |
| Semi-duplex while speaking | adapted | plus interruption (§7.4) |
| Music ducked to 25 | adapted | the player's session to `min(current, 0.25)` |
| Play, pause, next, previous | same | GSMTC through `media` |
| Player volume | adapted | system and per-app volume |
| Play an artist or a playlist | dropped | no cross-player search on Windows; says so |
| Pills add, remove, main, replace, only | same | |
| 13 question topics from kept state | same | plus timer, battery, calendar, usage |
| Weather (Open-Meteo) | same | opt-in with a city |
| Guided email, attachment by drop | adapted | ends in the chat-plus mail window, filled in; Send is a click |
| Contacts | adapted | a small address book in Settings, or a spoken address |
| Open an app | adapted | Start Menu shortcuts |
| Web search answered by Claude | adapted | opens the browser; aloud only with the Claude option and its web switch |
| Apple on-device model | adapted | Needle chooses actions; a small local text model writes (§4) |
| Claude as the brain, implied by a saved chat key | adapted | optional, its own switch; rules and Needle stay in front |
| English and French | adapted | English only |
| System voice, gender, pitch | adapted | Kokoro main, Windows voices fallback |
| ElevenLabs | same | optional, user's key |
| Character states, sounds | adapted | two new Visor faces (§7.6); synthesised sounds (§7.7) |
| Caption capsule, transcript list (20, memory), logs without words | same | |

**METEOR V1 adds:** timers, stopwatch, Pomodoro; system and per-app volume, outputs, mute all microphones; keep awake; brightness; dark mode, lock, display off; reminders and notes; clipboard paste with a yes; shelf; any island section; usage figures; "what is it asking"; interruption; sentence-by-sentence speech; a registry any feature adds to.

## 3. Architecture

### 3.1 Split

A feature crate cannot call another feature (`Ctx` has no such call); the page can (`feature_call`). So Rust is ears, mouth and engines; the island page is language and actions.

- `crates/voice` (feature `voice`): capture, detection, wake, turn timing, Whistle and Needle, player and voices, caption window, generator process.
- `crates/voice-speak` → `meteor-speak.exe`: the Kokoro speech server.
- `src/features/voice`: registry, parser, runner, brain routing, sentence splitter, strip row, settings.
- Shared: built-in `chat` gains `ask` (a headless streamed completion); the island gains `IslandApi.ear / shape / face`; the mascot two faces.

### 3.2 Pipeline

| Stage | Mechanism |
|---|---|
| Capture | WASAPI shared, default input, 16 kHz mono float (`AUTOCONVERTPCM`, as `dictation/capture.rs`), 30 ms poll, 0.5 s ring; rebuilt on device loss, default change (`svc-audio`) or a 2 s stall |
| Detection | `EnergyVAD` port (§3.4); replaces dictation's fixed `QUIET` |
| Wake | each voice segment, and a sliding window in long speech, to Whistle with `--audio-language en --audio-keywords`; matcher §6 |
| Command | the same buffer goes on, detection bypassed; a partial transcript about once a second |
| End of turn | 0.6 s quiet → provisional transcript → 1.6, 2.6 or 3.2 s → final transcript of the whole buffer |
| Speech to text | `needle.exe --model whistle.cact --audio <temp wav>` per run: 0.12 to 0.20 s measured for 2 to 6 s; `dictation::whistle` with two options added |
| Understanding | parser in the page (§4.1) → `needle.exe --model needle3.cact --tools … --serve` on loopback (§4.2) → text model through `chat.ask` (§4.3) |
| Acting | runner in the page, safety classes, `feature_call` (§5) |
| Speaking, showing | sentence queue → Kokoro, Windows or ElevenLabs → one WASAPI render stream; caption window, strip row, Visor (§7) |

### 3.3 Processes and cost

| Piece | Lives | Memory | Idle CPU |
|---|---|---|---|
| Capture thread | wake on: while voice is on and not paused; else during a turn | under 2 MB | 0.1 to 0.3 % of a core *est.* |
| Whistle | one process per run | none between runs | none |
| Needle server | first word of a command → 3 min after the last request | 63 to 68 MB measured | none measured |
| Kokoro server | start of the first turn (or first chat send when chat is read aloud) → 5 min after the last sentence | 0.67 GB measured | none: blocked on its pipe |
| Generator server | first turn that needs written text → 10 min after the last token | about 4.1 GB while loaded (part of it file pages Windows can reclaim) | none; 2 threads on the efficiency cores at below-normal priority while writing (about 12 % of the machine, 4 tokens a second) |
| Player thread | while a sentence plays | under 1 MB | none |

Every child: `NEEDLE_TELEMETRY=0`, `DO_NOT_TRACK=1`, no console, a kill-on-close job, 127.0.0.1 on a free port whose listener PID is checked (the spike saw two servers share a port). All end at once on voice off, tray Pause, lock, sleep, energy saver, quit. Steady sound near the microphone costs one Whistle run per 2 s, about 7 % of a core *est.*; silence costs none.

### 3.4 States and timings

`off` → `idle` (shortcut only, microphone closed) or `waiting` (wake on, microphone open) → `hearing` → `listening` → `thinking` → `speaking` ⇄ `asking` → `closing`. `paused(reason)` from anywhere.

| Value | Setting | Source |
|---|---|---|
| Calibration | 0.5 s | upstream |
| Segment starts / quiet frame / "voiced" | power > noise × 8 / < × 2 / > × 5 | upstream |
| Noise adapts | 0.194 per second idle, 0.042 in quiet; start 1e-7 | upstream's per-frame 0.005 and 0.001 at 43 frames/s, converted (our poll is 33 per second) |
| Segment ends | 0.8 s quiet; **0.5 s** while waiting for the wake | spike: Whistle runs at segment end, the tail is latency (0.5 + 0.13 s) |
| Forced end | 30 s, then noise := segment mean | upstream |
| Wake window in long speech | first at 1.2 s, then every 2 s on the last 3.5 s | new |
| Worth transcribing | 0.3 s voiced | spike |
| First word after a wake or tap | 5.0 s | upstream ("3 s was too short"); its 3 s for the shortcut not kept |
| First word of a reply | 8.0 s; 5.0 s for yes or no; 20 s awaiting a dropped file | upstream |
| Provisional transcript | after 0.6 s quiet | spike |
| Silence ending a turn | 1.6 s; 2.6 s after an extending word; 3.2 s for a free-text answer | upstream |
| Extra patience | voiced in the last 0.5 s → look again in 0.5 s | upstream |
| Hold to talk | held over 0.4 s → ends 0.2 s after release | new |
| Turn limit | 20 s | upstream |
| Follow-up microphone | 150 ms after speech ends | upstream |
| Conversation memory / failures ending it | 90 s / 2 in a row | upstream |
| Stall / rebuild | 2.0 s / stop, 0.5 s, start; 5 s apart, 5 per minute, then a banner | upstream |
| Deaf engine | 3 s voiced and empty text, twice (6 with media playing) → check hashes, rebuild | upstream |
| Needle timeout | 2 s (0.18 to 0.46 s measured) | spike |
| Text model | first token 12 s local, 6 s online; 96 tokens; 30 s | new |
| Captions stay | 2.0 s | upstream |

Dead upstream, not ported: `maxTurnTime 15`, `silenceTimeout 1.5`.

### 3.5 Files

Every module listed here is a file of its own (`crates/voice/src/lib.rs` dispatches by method prefix). Each feature with actions gets `src/features/<id>/voice.ts` and one line in its `index.ts`.

Events to the page: `voice:state`, `voice:level {mic, out}` (20 per second, during a turn), `voice:heard {text, final}`, `voice:spoke {seq}`. Methods: `ears.listen/stop`, `mouth.say/stop/voices`, `needle.tools/choose`, `speech.*`, `generator.*`, `caption.place`.

## 4. The brain

Per turn: (0) empty or filtered noise → close; (1) cancel or end phrase; (2) a pending question takes the reply; (3) parser; (4) Needle; (5) text model; (6) nothing qualifies. A command understood at 3 or 4 never leaves the PC.

### 4.1 Parser

**Not a port of `IntentParser`** (39 KB of pill and music precedence). METEOR V1 has about sixty actions in fourteen features, so actions are data and one matcher scores them. Ported as they are: `normalise`, the filler and politeness lists, the multi-action split, the 90 s follow-ups, `isYes/isNo`, `EntityResolver`.

- **Preparation.** Lowercase, punctuation to spaces; drop "please", "can you", "hey", "um"; snap Whistle's homophones at the start ("said/sent/senator a timer" → "set a timer", "both" → "pause"); "to morrow" → "tomorrow"; "twenty per cent" → "20 %".
- **Templates.** `"set a timer for {minutes}"`, `"{minutes} timer"`, `"open [the] {section}"`. Literal words in order, `[x]` optional, `(a|b)` alternatives. Score = literal words matched; a tie goes to more slots filled; under two literal words, or a tie between two actions, is a miss.
- **Slots.** `number` (digits, words to 999, "a", "a couple"), `minutes` ("90 seconds", "an hour and a half", "2 h 15"), `percent`, `time` ("at 5 pm"), `text` (the rest, original case), `choice`.
- **Entities.** A `choice` names a live list: sections, pills, audio sessions and outputs, quick-access tiles, Start Menu apps, reminders. Alias table, exact name, one whole-word match, then edit distance 2 or less (1 for four letters or fewer) unless the runner-up is also within tolerance. A required slot left empty → the action's question, at most twice.

### 4.2 Needle

- **Tools** come from the registry at start and on `features-changed`: one per action (name, `describe` plus two spoken examples, parameters from `args`, closed sets as enums) and `no_action` ("small talk, thanks, jokes, open questions, anything the other tools cannot do"). Rewriting three descriptions took the spike from 18 to 21 of 23.
- **Request.** `/reset`, then `POST /complete {"input"}`; system `date: …; locale: en-US; device: pc`; numbers as digits.
- **Confidence does not decide.** Right calls scored 0.36 to 0.92, wrong ones 0.12 to 0.50. Every wrong call carried an argument nobody said ("tell me a joke" → `control_media play`). So:
  1. Reject under 0.30, on `no_action`, or when Needle suppressed the call.
  2. **Grounding**: every argument is in the transcript (a number as digits or words, an enum value or a spoken form of it, free text as a substring).
  3. **Trigger**: a verb of the action's forms is present, or a noun of them plus a grounded argument.
  4. Pass: *run* runs and the capsule shows Undo for 6 s; *confirm* asks; *prepare* prepares. Fail with a question shape → text model; else "Sorry, say that again?" once.
- **Life.** Started at the first word of a command (not at the wake: upstream found that slowed recognition), ready in 1.6 s, 0.3 s per request, gone after 3 idle minutes. It never speaks; every word comes from the action.

### 4.3 Written text

The **generator** is a component with one contract, whatever model fills it.

| Contract | |
|---|---|
| Model, runtime | **Qwen3.5-4B**, Q4_K_M quantisation, Apache-2.0, a 2.74 GB download, chosen by measurement (13 of 14 English tasks right and all 5 tool tasks, the best of nine models tried). Run by llama.cpp `llama-server` (CPU build b11552) with `-t 2`, pinned to the efficiency cores at below-normal priority: 4.0 tokens a second on the development laptop, first sentence after about 7 s. Started on first use, unloaded after 10 minutes idle; the system-and-tools prefix is warmed in the background at start (its first pass costs 35 to 70 s). Fallbacks for weak machines: Granite 4.0 H 1B for choosing a tool and LFM2.5-1.2B for text. Every download address, revision and hash is in section 8 of that file |
| Files | `{name, url at a pinned revision, sha256, size}`; fetched only by the Settings button, streamed to `.part`, hashed, renamed; hashed again before each start |
| Process | §3.3, below-normal priority, **2 threads** (1 to 4), context 2,048, one request at a time, prompt cache on |
| Start, stop | the first turn that needs written text; 3 min after the last token |
| Request | `chat.ask {provider: "builtin", system, messages, max_tokens: 96, stream}`; fixed system prompt under 120 tokens ("spoken answer, one or two short sentences, plain words, say so if you don't know"); three earlier exchanges at most |
| Output | `chat:ask-delta {id, text}`, `chat:ask-done {id, stop}`; cancel by id |
| Floor | 2 to 3 tokens per second is enough |

At that floor the model writes slower than the voice talks (about 3.3 tokens per second): the first chunk is spoken 4 to 7 s after the question *est.*, with pauses between sentences. So: two sentences at most, words shown in the capsule as they are written, `think` after 1.2 s. Commands never reach it.

**Routing.** `brain`: `auto` (default), a fixed choice, or `off`. `auto` takes the first that is switched on and ready:

1. the built-in generator;
2. the user's own local server from chat (`ollama`, `lmstudio`, `custom`), reachable in 300 ms;
3. a free online model, only with "Voice may use a free online model": `openrouter` `is_free`, or `google` free tier;
4. Claude with the user's key, only with "Voice may use my Claude key". Never implied by a key saved for chat (upstream sent every transcript once a chat key existed).

A choice that fails or times out falls to the next that is on. An online choice gets the turn's text, three earlier exchanges, the date and the state asked about; never audio, clipboard, notes or file contents.

**Claude.** Model from the chat's list, default the smallest (`claude-haiku-5-5` today). `chat.ask` adds what `claude.rs` lacks: streaming, `output_config.effort: "low"` (thinking counts toward `max_tokens`), 1024 tokens, client tools, `pause_turn`, a cache marker. Claude also gets the registry's tools; its calls pass the same runner and classes; 5 rounds, 24 messages, 90 s (upstream). Web search (`web_search_20260209`) is a second switch.

**Mail text.** "Word for word" is literal; otherwise the routed brain writes it; with none, a template.

### 4.4 Nothing qualifies

"I can't answer that on this PC yet. You can add a text model in Settings, Voice." After a wake with nothing understood: "Sorry, say that again?" once. In a conversation: silent close, `surprised`.

## 5. The action registry

```ts
// src/core/feature.ts            FeatureModule.voice?: VoiceAction[]
export type VoiceSafety = "run" | "confirm" | "prepare";
export interface VoiceArg { name: string; kind: "number" | "minutes" | "percent" | "time" | "text" | "choice";
  choices?: () => { id: string; say: string[] }[]; optional?: boolean; ask?: string }
export interface VoiceAction {
  id: string; tool: string;        // tool = what the action model sees: "timer_start"
  describe: string;                // one line for the model
  forms: string[];                 // "set a timer for {minutes}"
  args?: VoiceArg[]; safety: VoiceSafety;
  confirm?: (a: Args) => string;   // "Paste the last copy into Word?"
  run(a: Args, v: VoiceCx): Promise<VoiceReply | void>;
  undo?: (a: Args, v: VoiceCx) => Promise<void>;
}
export interface VoiceReply { say?: string; show?: string; ask?: VoiceArg; ok?: boolean }
export interface VoiceCx { call: FeatureCtx["call"]; t: FeatureCtx["t"]; open(section: string): void }
```

- `run` gets `VoiceCx`, not the feature's context: its `call` reaches the action's own feature through a **deny list** in the runner (`mail_send`, `always`, `bin_empty`, `clear`, `remove`, `delete_reminder`, `wallSet`). Safety lives in the runner.
- **run**: executes, speaks `say`. **confirm**: asks, shows Yes and No, runs on a spoken yes or a click; 5 s of nothing is a no. **prepare**: opens the feature's own card, filled in; only that card's button completes it.

| Feature | Actions | Class |
|---|---|---|
| `timers-downloads` | timer `start {minutes}`, stopwatch, Pomodoro, `pause`, `resume`, `cancel`, add minutes, time left | run |
| `media` | play, pause (`playPause` after `state`), `next`, `prev`, shuffle, repeat, open player, what's playing | run |
| `audio-mixer` | `volume`, up, down, `mute`, `appVolume`, `appMute`, `nextOutput`, `output` | run |
| | mute all microphones (`muteMics`): says it can no longer hear; unmute is Ctrl+Alt+U | run |
| `controls-toggles` | brightness, dark mode, hidden files, desktop icons, `lock`, `display_off`, add to a note | run |
| | empty the Recycle Bin: Controls with its question up | prepare |
| `displays-keepawake` | keep awake `start`, `extend`, `stop`; display brightness `set` | run |
| | wallpaper: opens its section | prepare |
| `calendar` | `add_reminder`, tick one, what's next, today | run |
| | delete a reminder | prepare |
| `clipboard` | open, clean the copied link | run |
| | paste the last or nth copy, paste plain | confirm |
| | clear the history | prepare |
| `shelf` | open / add a named file / remove, share | run / confirm / prepare |
| `island-extras` | open a quick-access tile | run |
| | mirror: "open it with a click" | never |
| `huds-power`, `agent-usage` | battery; tokens and value today and this week | run |
| `chat-plus` | email: mail window filled in | prepare |
| base: island | open a section, close, Settings, app sounds | run |
| base: agents | what are they doing, what is it asking, jump to the terminal, plan usage | run |
| | allow ("I can't approve by voice. Click Allow."), deny | never |
| base: integrations, pills | service figures from kept state; add, remove, main, replace, only | run |
| base: chat | "ask the chat …": the text in the chat field | prepare |
| voice | open an app, search the web, weather, time, repeat, undo, stop, stop listening | run |
| | turn a feature off / one on that uses camera, hooks or elevation | confirm / prepare |

`lyrics` declares none. Later features add their own `voice.ts`.

## 6. Wake phrase

- **Phrase.** `"OK " + the first word of notch_brand::NAME`. Spike, made with the code name ("OK notch"): 15 of 18 with keywords and matcher (6/6 clean, 5/6 white noise, 4/6 under competing speech), 0 false accepts in 315 runs; "Hey notch" 6 of 18: not offered. Not measured again for "OK METEOR". Keywords file: the phrase, the noun.
- **Matching.** First word in {ok, okay, oaky, okie, o, oke, okey}; second the name, within edit distance 1 of it, or in the table of what the engine writes for it. The table is per name: for the code name it was measured (noth, nosh, nash, natch, knotch, noche, knocch, nach; "not" excluded, it matched "Okay, not that one" 6 times in 63); for METEOR V1 it holds three guesses (lentil, lintle, lentle) and is not measured yet. Anywhere at a word boundary, the last occurrence wins, the text after it is the command.
- **Energy gate.** Transcribed only with 0.3 s voiced and voiced frames at least 40 % of the span (clicks and hum fail).
- **Hallucination filter.** Whistle wrote "Thank you." on hum and "Opening notch." in noise (the spike used the code name). A transcript that is only a stock phrase ("thank you", "thanks for watching", "you", "bye") from under 0.6 s voiced is dropped; a keyword without an OK word never wakes.
- **Second pass.** Voiced power under noise × 10, or media playing: transcribed again without keywords; the OK word must still be there (0.13 s).
- **False wakes.** A wake blips and shows the strip; no command in 5 s closes silently. Three empty wakes in 10 minutes double the gate for 10 minutes. At worst a *run*-class action runs.
- **Music or speech playing.** The 30 s forced end makes steady sound the ambient; meanwhile the window slides, one run per 2 s; three empty runs space the next 0.5 to 5 s apart.
- **Its own speech.** Only the wake phrase is looked for, and it interrupts.
- **The phrase follows the name.** Phrase, keywords and the distance rule follow `NAME`; the mishearing table is per name: redo it with recorded clips and real voices.

## 7. Speaking and showing

### 7.1 Voices

| Voice | When | First audio |
|---|---|---|
| **Kokoro**, on-device | main, once downloaded | 0.6 to 1.2 s after its text *est.* |
| **Windows voices** | until then, and when Kokoro fails | about 0.1 s; on the development laptop: David, Zira, Mark, none natural |
| **ElevenLabs** | optional, user's key | each sentence leaves the PC |

All feed one player: a WASAPI shared render stream (the mirror of `capture.rs`), not `MediaPlayer`, which would register a media session that METEOR V1's own `media` would show and duck. It gives an exact "is speaking", a stop within 30 ms and the output level the Visor talks with. **Picker**: Kokoro's 28 English voices, the English voices of `SpeechSynthesizer::AllVoices`, the account's ElevenLabs voices, each with Test; a link to `ms-settings:speech`.

### 7.2 Kokoro on Windows

- **Model.** Kokoro-82M v1.0, 82 M parameters, 24 kHz, Apache-2.0. English: 20 US and 8 British voices; its card grades `af_heart` and `af_bella` highest and warns that lines under 10 to 20 tokens may sound weaker.
- **Runtime.** sherpa-onnx 1.13.8 (Apache-2.0), its own Rust crate pinned to `=1.13.8`, `shared` feature (its static libraries use the static C runtime, which does not mix with the workspace's). It links under MSVC as it is. Carries ONNX Runtime (MIT) and espeak-ng (GPL-3.0-or-later, as METEOR V1). CPU, 4 threads in bursts. The crate's build script fetches the prebuilt Windows x64 runtime of that version (8 MB, a GitHub release asset) into `target/sherpa-onnx-prebuilt/` once and copies its DLLs beside the built exe.
- **Shipped, not downloaded:** `meteor-speak.exe` (0.2 MB), `sherpa-onnx-c-api.dll` (4.6 MB) and `onnxruntime.dll` (17.8 MB) go into the installer, together in a folder `speak\`. Windows has an older `onnxruntime.dll` of its own in System32, which is found before any folder but the exe's own; and the exe and its DLLs must be of one version.
- **Server.** `meteor-speak.exe`, a small crate of ours: one request per line on stdin (`{"id","text","voice","speed"}`), a header and 24 kHz PCM on stdout, no port. It loads once, says so with one frame (or says why not: "engine not available", and the Windows voice stays), and waits blocked on its pipe. It ends when its stdin ends, with its parent (the job), and by itself `keepVoice` minutes after its last sentence; the next sentence starts it again. The engine hands audio over sentence by sentence, not within one: a chunk of one sentence is one frame.
- **Download** (one click, 371 MB): `model.onnx` (326 MB), `voices.bin` (28 MB), tokens and two lexicons from the Hugging Face repository of sherpa-onnx's author at one revision, and `espeak-ng-data.zip` (9 MB), a GitHub release asset, which has no revision: the hash is the pin. Each file is checked by SHA-256 before it is kept; what the server loads is checked again before it starts (hashed once per run of the app, 1.6 s; after that by size and date). The zip is unpacked by System32 `tar.exe`. 363 MB on disk. Measured here: 362 MB from Hugging Face in 20 s; from GitHub 0.1 to 0.17 MB a second (the 9 MB took 98 s; the model as GitHub's one archive would have taken 35 minutes, which is why it is not the source). The 132 MB int8 export is not measured.
- **Cost, measured** (`voice-kokoro-probe`, i7-1260P, 4 threads, while other builds kept the laptop 25 to 50 % busy; a quiet machine is not measured):

| | |
|---|---|
| Load | 2.2 to 2.5 s (3.4 s once) |
| Real-time factor | 0.52 to 1.08 over six runs, most often 0.65 to 0.9; 6 threads 0.55 to 0.76; 2 threads 1.25 to 1.48 |
| First chunk after its text | the whole chunk's time: 1.0 to 1.6 s for six words (1.5 s of audio), 3.0 to 5.1 s for a 112-letter sentence (5.7 s of audio) |
| From nothing to the first chunk | 3.7 to 4.9 s (start, load, six words) |
| Memory while loaded | 673 MB working set, 685 MB private |
| Idle | 0.0 ms of processor time in 20 s. In the half second after a sentence the engine's three worker threads spin (1.5 s of processor time) before they block; its C API has no switch for it |
| Ends itself / with its parent | 2.1 s after an idle time of 2 s; 47 to 83 ms after its parent ended without ending it |

  The 1.0 s rule (Kokoro stays the default only if a first chunk plays within 1.0 s of its text) is **not met on the development laptop under that load**: six words took 1.0 to 1.6 s. To be judged again on a quiet machine. And a long chunk after a short one leaves a silence (2.1 to 3.4 s before the 5.7 s sentence that followed 1.5 s of audio): a chunk plays without a gap only when its audio is at most about 1.2 times as long as the one before it, so the splitter (§7.3) should let chunks grow step by step rather than cut only the first one short.

### 7.3 Sentence by sentence

1. **Splitter** (`brain/split.ts`, fed by fixed replies and token deltas). Text is cleaned for speech (markdown, links, `[1]`, bullets, code marks). Cut at `. ! ? …` or a newline, not after an abbreviation or inside a number. The **first** chunk is also cut at `, ; :` once it has six words. Under 12 characters joins the next; over 220 is cut at a space. After the first, a chunk is at most about 1.2 times as long as the one before it (§7.2; characters stand in for audio): a sentence that runs on more than 12 characters past that is cut at the last `, ; :` within it, else at a space between words, never in a number.
2. **Queue.** `mouth.say {gen, seq, text}`: synthesis runs at most two chunks ahead of the player.
3. **Player.** In `seq` order, 150 ms apart; `voice:spoke {seq}` as a chunk starts (the capsule brightens it).
4. **Phrase cache.** Fixed replies ("Done.", "Paused") kept as PCM per voice, 64 entries: instant with Kokoro. A chunk the main voice fails on goes to the Windows voice.

### 7.4 Interruption

Each raises the generation number: the player fades in 30 ms and empties, late synthesis is dropped, the text stream is cancelled.

| By | Then |
|---|---|
| Esc | the turn ends. A 30 ms `GetAsyncKeyState` poll during a turn only: Esc is observed, not taken |
| Talk shortcut, wake phrase | stops and listens |
| The capsule's stop button | the turn ends |
| Speaking | headphones: 0.3 s of voice; speakers: only voice louder than the learned echo of the playback. Not proven with a real voice in a room |

### 7.5 Strip, capsule, open island

- **Waiting for the wake phrase.** A 28 × 3 px bar, `--accent` at 70 %, centred in the rest strip; `--ink` for 0.3 s when a voice is heard. Drawn by the island (`cx.island.ear`), by events, no frame loop.
- **Strip in a turn.** Activity `turn`, order −1, `hold`, 340 wide, one `stripRow`: mark = the Visor; title "Listening", "Thinking" or the result ("Timer · 12 min"); right = the microphone meter, the playing bars, or Yes.
- **Capsule.** Factory window: frameless, transparent, topmost, no-activate; 380 wide, one to three lines, 6 px under the island's lower edge (strip or open island; 6 px from the top over a full-screen app). `--bg` at 92 %, `--r-md`, 1 px `--hairline`, `--t-sm`. Left: an 8 px dot, `--bad` pulsing while the microphone is open. Line 1: what was heard, `--dim`. Lines 2 and 3: the answer, `--ink` 500, bright from the sentence being spoken. Right: stop. A click opens it to 480 × 240. In 120 ms, out 220 ms, gone 2 s after the turn. Captions off: it shrinks to the dot and stop; the indicator never goes.
- **Open island.** Nothing is covered; the capsule sits under it; the island's own mascot takes the face (`cx.island.face`). A waiting card stays put.

### 7.6 The Visor

`d` diameter, `th` bar thickness, as in `src/mascot/engine.ts`.

- **listening** (new). Five segments 0.06 d wide, 0.03 d apart (three of 0.10 d under d = 30). Height `th × (0.6 + 2.2 × level × e[i])`, `e = [0.5, 0.8, 1, 0.8, 0.5]`, each shimmering ±15 % at 9 Hz; `level` = microphone, attack 60 ms, release 180 ms. Silent: a flat dashed bar breathing 85 to 100 % over 1.2 s. Tint `--accent`; bob halved.
- **thinking** (exists), looking up and right (`lookX` 0.4, `lookY` 0.5).
- **talking** (new). One white bar: width `0.34 d × (1 − 0.12 × out)`, height `th × (0.7 + 1.6 × out)`, `out` = level of what is played; bounce 0.018 d at 3.5 Hz while `out` > 0.05.
- **asking** = `question`. End `wink`; a miss `surprised`.
- **Hidden mascot**: the dot's ring `--accent` swelling with the level; the arc turning; the ring `--ink` flickering with `out`. Reduced motion: no shimmer or bounce.

### 7.7 Sounds

Existing names, under the app's sound switch: `blip` when it listens; `pop` when the turn is taken; `think` after 1.2 s; `annoyed` on a miss; `wink` at a clean end with nothing spoken; `gulp` and `approve` for a dropped attachment.

## 8. Privacy and safety

**ARCHITECTURE §12 rule 9, replacement:**

> 9. The camera starts only from a click and stops on every hide path; the LED must go off when the section hides. The microphone is opened in two ways only. (a) For one recording, by a click or a shortcut press (dictation, a push-to-talk turn), closed when that recording ends; and while the voice assistant itself is speaking, for the one word "stop" (nothing else heard then is kept or acted on), closed when it has finished. (b) Continuously, by the voice assistant waiting for its wake phrase, only while the user has switched `voice` on and "Listen for the wake phrase" on (off by default), and none of these pauses it: the tray's Pause, lock, sleep, display off, energy saver, a full-screen app in front, dictation or a recording using the microphone, every microphone muted. Whenever the app holds the microphone open an indicator is on screen the whole time (the bar of light while it waits and while it speaks, the listening row and the capsule's dot during a turn); if the indicator cannot be shown the microphone is not opened. Audio stays in memory, and for one engine run in a temporary file deleted when the run ends; it never leaves the PC, is never logged, and no transcript is logged. A webview is given the camera only (`webview_permission.rs`, §8), never the microphone: dictation and voice record in Rust.

**Rule 2, added:** "…and for voice, each only after the user's click or switch: the action model, the natural voice and the local text model (SHA-256 checked before kept and before run); Open-Meteo weather for the city typed; an online text model or voice with the user's own key."

- **Indicators.** §7.5, plus a tray item that stops it. **Logged**: states, counts, timings (`turn: stt= parse= needle= brain= speak= ms`). **Never**: words, names, addresses. The last 20 transcripts are in memory only.
- **Telemetry.** Both variables on every start of every child (already in `dictation/whistle.rs`). **Keys.** `voice/elevenlabs-key` in Credential Manager; the Claude key is read by the app crate only.
- **First run.** The microphone rule; shortcut only or also the wake phrase; download buttons with sizes (speech model 19 MB, action model 35 MB, natural voice 350 MB, text model per the research), only the first required; a level meter; the online options, all off, each with what would leave.
- **Voice can never**: allow a permission or answer an agent's question; send a mail; delete, clear or empty; start the camera; unmute the microphones; prompt or start an agent; install or uninstall; write a foreign config; elevate; turn on an online service or a feature that uses camera, hooks or elevation; paste without a yes.

## 9. Settings page

| Option | Default |
|---|---|
| Voice (feature switch) | off |
| Listen for the wake phrase · wake word | off · the product name |
| Talk key · hold to talk | Right Ctrl on its own · on |
| Speak answers · short replies · captions | on · spoken (or a sound) · on |
| Voice | Kokoro when downloaded, else the first English Windows voice |
| Kokoro voice, rate · Windows rate, pitch | `af_heart`, 1.0 · 1.1, 1.0 |
| Keep the voice loaded | 5 min |
| Interrupt by speaking · Esc stops it | off · on |
| Lower the music while I talk | on |
| Action model · commands it chose | on once downloaded · run when grounded, with Undo |
| Text model · threads · keep loaded | automatic · 2 · 3 min |
| My local server from chat | off |
| Voice may use a free online model | off |
| Voice may use my Claude key · model · web search | off · smallest · off |
| ElevenLabs voice | off |
| Weather · city | off · empty |
| Web search | opens the browser |
| Read chat answers aloud | off |
| Address book, downloads, microphone test, last 20 transcripts | — |

Manifest: `group: Tools`, `default_on: false`, `uses: [Microphone, Network]`, shortcut `talk`, tray item `listen`.

## 10. Status in 1.0.0

Built and seen working end to end in a debug copy, with recorded clips standing in for a voice: the talk key (Right Ctrl: a tap starts a turn, a hold talks until release), hearing and the end of a turn, speech to text, the command parser, the action model with its "every detail was said" rule and Undo, the runner and its safety classes, the voice actions of the features, spoken answers from what the app already knows, the local text model, the natural voice with a Windows voice as the fallback, the strip row, the caption capsule, the Settings page, and the wake phrase (off by default).

Not yet proven with a real voice in a real room: wake-phrase accuracy and false wakes, the word "stop" over the app's own voice through speakers, short yes and no answers, the first word of a held key, and how the voices sound. Not seen in the running app: the optional online models, ElevenLabs, weather by voice, reading chat aloud, and the mail window filled by voice. Not built: the wake phrase standing down while dictation or the recorder has the microphone.

The wait before a spoken answer from the local text model was 9 to 15 seconds on the development laptop once warm, and about half a minute cold. Commands that the parser or the action model understands answer in about 4 to 6 seconds.

## 11. Choices made for 1.0.0

Listening starts with the talk key; the wake phrase is a switch the user turns on, with a choice of "only while plugged in". The local text model is the first brain; a free online model and Claude through the user's own key are separate switches, both off by default. Weather by voice and a Wikipedia lookup are switches that start off. Voice never approves or denies a permission card: it reads the request's tool aloud and says to click. Speech stops on Esc, the talk key, or the word "stop".

## 12. Risks

**Not knowable until a person speaks into a real microphone** (every spike clip was a Windows voice written to a file): wake accuracy for a real voice, accent, distance and a laptop microphone; false wakes per hour beside a film, a podcast or a call; whether the microphone hears METEOR V1's own voice and the music loudly enough to break detection (no echo cancelling); whether the 0.5 s wake tail and 1.6 s end of turn feel right; whether Kokoro's short replies are liked; CPU and battery with the microphone open all day.

| Risk | Answer |
|---|---|
| A false wake runs something | only *run*-class actions can; blip, strip, Undo |
| Whistle invents text in noise | energy gate, stock-phrase filter, OK-word rule, second pass |
| Needle fires on small talk (1 in 6 unguarded) | grounding, trigger, `no_action` |
| Sixty tools lower its accuracy | shortlist by template score; measured in voice-needle |
| Local text at 2 to 3 tokens per second feels slow | two sentences, caption first, early first chunk; Claude for those who want speed |
| Kokoro slower or larger than estimated, or will not link | measured with a pass line; the Windows voice stays |
| "Mute my microphones" deafens it | it says so; unmute is a key or a click |
| A brain's reply tries to cause an action | its calls pass the same classes and deny list |
| The rename | phrase follows `NAME`; mishearing table and real voices again |

**Later.** Other languages (Whistle hears six more, Kokoro speaks seven more, upstream's French lists exist). Play an artist through a player's search. A resident Whistle for true live captions.

Considered and dropped: Jev, a decision model for choosing actions; its open version needs a GPU.

## 13. As built

One spoken turn runs from the ear to the mouth; the microphone was never opened in these tests (recorded clips stood in for it), so everything below is true of clips, not yet of a voice in a room.

**Where the code differs from the sections above**

- **The talk key** is Right Ctrl on its own (§9 still says Ctrl+Alt+H): a tap starts a turn, holding it talks until release, with another key it stays an ordinary Ctrl. The chord `talk` is unbound.
- **Saying "stop"** (§7.4): with the default settings the microphone opens while METEOR V1 speaks, for that one word. The detection runs, the speech model reads each segment, and only a transcript of three words or fewer is handed to the page, which acts on the stop words alone; nothing else is kept. The indicator (ear bar, capsule dot) is on for as long. There is no echo cancelling: over speakers its own voice is in the same segment as the user's "stop". With "Interrupt by speaking" on and the wake wanted, the wake phrase interrupts too.
- **Kokoro** loads at the start of a turn (the first word of §3.3), not at the first sentence: `mouth::warm` from `ears`.
- **The text model** starts on the first question that needs it. A cold start is not counted against the first-word limit (`Brain.start`), and a cold model gets the whole 30 s for its first word. An earlier question is sent again exactly as it was sent the first time (date line included), so that the server keeps what it has read.
- **`chat.ask` belongs to voice**, not to the Chat switch: the built-in model on this computer answers with Chat off; every other provider is refused by the host unless voice's own switch for it is on (`chat/headless.rs` `permit`).
- **The model that answered** is shown in the capsule (dim, after the answer) for an online brain, never spoken.
- **Chunks** grow step by step: a chunk is at most about 1.2 times as long as the one before it (§7.2's finding), cut at a comma or a space.
- **Email by voice**: `mail_send` takes a mail without a file (the default mail program's compose window, or Resend); Send is still only the mail window's button.
- **Debug builds only**: `METEOR_VOICE_CLIPS` makes a pretend microphone that plays the clips given to `ears.feed {path}` and never opens the device; `METEOR_VOICE_MUTE` mutes the app's own audio session for as long as the player's stream is open. Neither exists in a release build.

**Measured in the running app** (debug build, i7-1260P development laptop, other programs busy on it)

| | |
|---|---|
| Parsed command, end of speech to the first word of the reply | 3.6 s warm (1.6 s of it is the silence rule); 5.6 s with Kokoro cold |
| Speech to text | 0.16 to 0.55 s over one to three runs a turn |
| Action model | up in 1.3 s; "Give me seven minutes on the timer" ran with Undo 2.5 s after the turn ended |
| Open question, end of speech to the first word | 9.4 s and 14.6 s warm (14.2 s and 15.8 s before the conversation was kept in the server's cache); 24 to 34 s when the model is cold |
| Text model | first token 4.6 to 6 s after the turn ends, about 3.4 tokens a second; cold: up in 8 to 13 s, its warm-up 5.7 to 8.7 s more |
| Where an open question's seconds go | 1.6 s silence rule; 4.6 to 6 s to the first token; then the first chunk waits for a comma or a full stop (up to 14 words: 4 to 6 s); 1.4 to 1.7 s to make its sound |
| Caption before voice | the chunk is shown 1.5 to 1.8 s before it is heard |
| Stopping | Esc: 0.13 s to closing; the talk key: 0.2 s to listening; "stop" from a clip: 1.3 to 1.5 s after the word began |
| Memory while loaded | Kokoro 0.44 to 0.55 GB; action model 55 to 67 MB; text model 4.2 to 4.6 GB; the app itself 85 to 96 MB |
| Leaving | Kokoro 30 s after its last sentence with the setting at half a minute, the text model 60 s after its last token with one minute, the action model 3 min after its last request; all three within 1.1 s of the tray's Pause; all with the app's own Quit; all at voice off |
| Voice off | 42 to 43 threads and no child process, as before voice was ever switched on; voice on and idle: 43 to 49 threads |
| Speech model download (17 MB, two files) | 5.5 s, the row counting "0 MB of 18 MB" to "Ready" |

**Found and left**

- A cold text model makes the first question wait half a minute. Warming it when a turn starts would cost 4.4 GB for every command.
- The first chunk of an answer without an early comma is its whole first clause: the first word waits for it.
- The model sometimes talks about the date it is given ("…not just on October 11").
- A "no" of under 0.3 s of voice never reaches the speech model: the question closes after 5 s as a no, unacknowledged.
- The speech model wavers on synthetic voices ("Timeer", "Delite", "wall paper"); the parser now joins and corrects the ones seen. A real voice may show others.
- The answer appears in the capsule a chunk at a time, not word by word as it is written.
