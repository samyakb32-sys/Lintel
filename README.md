# METEOR V1

A notch toolkit for Windows: an island that stays hidden at the top of the screen and slides out when you hover it or when something needs you.

![The open island on its Home view](docs/img/island-home.png)

METEOR V1 shows your AI coding agent sessions and lets you answer their permission requests without leaving your work. It also holds a chat, the music that is playing, system tools, screenshots and recording, and a voice assistant. Every part has its own switch.

## Contents

- [Why METEOR V1](#why-meteor-v1)
- [What it does](#what-it-does)
- [Requirements](#requirements)
- [Install](#install)
- [First steps](#first-steps)
- [Guide and documentation](#guide-and-documentation)
- [Build from source](#build-from-source)
- [Privacy](#privacy)
- [Downloads and extra consent](#downloads-and-extra-consent)
- [Known limits](#known-limits)
- [Code signing policy](#code-signing-policy)
- [Contributing](#contributing)
- [Credits](#credits)
- [Licence](#licence)

## Why METEOR V1

The Mac has a whole shelf of notch apps: NotchNook, The Boring Notch, Alcove, DynamicLake, MediaMate, TopNotch, NotchBar, NotchFlow, DropNotch, Coucou, Vorssaint. Windows has only a handful (DynamicWin, which is open source, Notchify, Venu, Dynamic Edge), and they are mostly about media and widgets. So there are very few for Windows, and none that does all of this. That is why METEOR V1 was built.

Where it comes from: METEOR V1 grew from the Windows edition of Coucou (MIT). It re-implements for Windows the best ideas of Vorssaint and Boring Notch (both GPL-3.0), and then adds its own. [NOTICE.md](NOTICE.md) has the exact credits.

The name: a lintel is the beam across the top of a door or a window. METEOR V1 sits at the top of your Windows.

It is made for Windows 10 and 11 first, and it holds to four values:

- **Efficiency.** The core is Rust, with a light web view for the interface (Tauri 2). Hidden and idle, the optimised build used 0.49 % of one processor core over two minutes in the audit of this release.
- **Optimisation.** A feature that is switched off loads nothing: no hooks, no timers, no threads, no windows. Where Windows offers an event, METEOR V1 waits for the event and does not poll.
- **Privacy.** No telemetry and no account. Network calls go only to what you turned on. Keys are kept in Windows Credential Manager. Nothing is approved, sent or deleted without your click.
- **Necessity.** Every feature can be turned off. You keep what you use and switch off the rest.

## What it does

### The island

![The Dashboard with its cards](docs/img/island-dashboard.png)

- Stays hidden at the top edge. Opens on hover, on a click or with a shortcut. Folds away when you press outside it.
- A Home view with what matters now, and a section for each feature that has something to show.
- A resting strip with a small mascot. The island can also be dragged away from the top and float as a pill.
- Steps aside while a full-screen app is in front. You choose the display it lives on.
- Ten interface languages. Global shortcuts that you can change.

### Coding agents and approvals

![A permission card on the island](docs/img/island-permission.png)

- Shows the sessions of Claude Code, Codex, Gemini CLI, Cursor, GitHub Copilot CLI, OpenCode, Amp, Hermes, Antigravity and the Claude desktop app.
- A permission request becomes a card with the command or the diff. You answer Allow, Always or Deny from the island. METEOR V1 never answers a card by itself.
- Questions from an agent, a live diff of its edits, and a card with the summary when it finishes.

![The card an agent leaves when it has finished](docs/img/island-finished.png)

- Jumps to the terminal or editor a session runs in.
- Plan usage for Claude and Codex, local usage figures, and a weekly recap.
- Connecting an agent changes its settings file only after a dated backup, a diff on screen and your confirmation. If METEOR V1 is closed, the agent carries on as if it were not installed.

### Chat

- A chat with Anthropic, OpenAI, Google or OpenRouter models through your own key, with any OpenAI-compatible server, with a local model (Ollama, LM Studio), or through a command-line agent you already use.
- Drop a file on the island to ask about it. Drag the mascot onto a window to attach it as context.

### Media

- What is playing, from any player Windows knows: cover, title, seek bar and transport buttons.
- Synced lyrics (off until you turn the lookup on), a visualiser, and a volume mixer per app.
- Small level displays for volume, brightness and the microphone.

### System tools

![A running timer on the island](docs/img/island-timer.png)

- System monitor: processor, memory, graphics, disks and network. Temperatures and fans come from an optional hardware helper.
- Battery, clipboard history, a shelf for files, a scratchpad, a timer with Pomodoro and stopwatch.
- Calendar from links you paste, reminders, weather.
- Keep awake, display controls, quick toggles, window layout, input tools.
- A command bar and a radial menu.
- App updates through winget, a cleaner and an uninstaller.
- Read-only status from GitHub, Vercel, Stripe, n8n, Resend, Notion and Cal.com, each with your own key.
- A desktop companion: the mascot can leave the island and sit on your desktop.

### Capture and recording

![The capture editor](docs/img/capture-editor.png)

- Screenshots of an area or the whole screen, with an editor to mark them up.
- Copies text and reads QR codes in a capture. A colour picker and a ruler for the screen.
- Draw on the screen.
- Screen recording of an area, a window or the whole screen.
- Media tools to convert and resize pictures.

### Voice (new, off by default)

- Hold a key and talk, or say "OK METEOR" if you turn the wake phrase on.
- Commands such as "set a timer for five minutes" or "pause the music", and spoken answers to questions.
- Speech recognition, the command model, the voice and the text model all run on your computer. An online model is an option you turn on with your own key.
- Dictation: hold a key and your words are typed into the app in front.
- Voice is new in this release and has had little testing. See [Known limits](#known-limits).

### Settings

![The Features page of Settings](docs/img/settings-features.png)

Every feature has a switch on the Features page, and a page of its own for its options.

## Requirements

- Windows 10 or Windows 11, 64-bit.
- The WebView2 runtime. Windows 11 has it. On Windows 10 the installer fetches it if it is missing.

## Install

1. Download the installer from the [latest release](../../releases/latest), or from [lintel.thisissbk.com/download](https://lintel.thisissbk.com/download).
2. Run it. It installs for your user only and does not ask for administrator rights.
3. The installer is not code-signed, so Windows SmartScreen will warn. Choose "More info", then "Run anyway", if you trust the file you downloaded.

With winget: `winget install lintel`. The package is in review at Microsoft, so this works once it has been accepted.

To remove METEOR V1, use Settings, Apps in Windows. The uninstaller takes METEOR V1's entries out of your agents' settings and leaves your own data unless you tick the box to delete it.

## First steps

1. Start METEOR V1. Move the pointer to the top centre of the screen and the island opens.
2. Right-click the tray icon and open Settings.
3. On the Features page, switch on what you want and switch off the rest.
4. Under Coding agents, connect an agent. METEOR V1 shows the change to the agent's settings file and waits for your confirmation.
5. Start a session in that agent. Its permission requests now appear on the island.

## Guide and documentation

The guide and the documentation are on the website: <https://lintel.thisissbk.com>.

In this repository:

- [docs/FEATURES.md](docs/FEATURES.md): every feature and how far it is built.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): how the app is put together.
- [docs/PRIVACY.md](docs/PRIVACY.md): every host the app can call.
- [docs/VOICE.md](docs/VOICE.md): the voice assistant.
- [docs/ROADMAP.md](docs/ROADMAP.md): what is left open, and where help is wanted.
- [CHANGELOG.md](CHANGELOG.md): what each release holds.

## Build from source

You need Node.js, the Rust toolchain and the WebView2 runtime. The hardware helper also needs the .NET 8 SDK, but only `npm run pack` builds it.

```
npm install
npm run build            # type-check and vite build (also builds the hook relay)
npm test                 # node --test suites
cargo check --workspace  # needs dist/ to exist, so run `npm run build` first
npm run tauri dev        # run the app
npm run pack             # installer into release/
npm run fake-session     # a pretend agent session through the relay, while the app runs
```

## Privacy

- No telemetry, no analytics, no crash reports and no account.
- On a fresh install with default settings METEOR V1 calls no host. Network calls go only to the services you turned on or gave a key for.
- Keys and tokens are kept in Windows Credential Manager, never on disk.

[docs/PRIVACY.md](docs/PRIVACY.md) lists every host, what turns the call on and what is sent.

## Downloads and extra consent

Nothing below is fetched or started until you click its button.

| What | Size | Needed for |
| --- | --- | --- |
| Speech recognition model | about 19 MB | Voice and Dictation |
| Command model | about 35 MB | Voice commands in your own words |
| Natural voice (Kokoro) | about 370 MB | Spoken answers in a natural voice |
| Local text model (Qwen3.5-4B) | about 2.7 GB, and about 4.3 GB of memory while loaded | Answers to open questions without an online service |

Each file is pinned to one revision and checked by SHA-256 before it is kept.

Hardware sensors (temperatures and fans) need two more consents. The helper `lintel-hw.exe` runs as administrator, so Windows shows its prompt when you switch the feature on. It reads the hardware through the PawnIO driver, which is not bundled: you install it yourself, or METEOR V1 offers to install it through winget. The rest of METEOR V1 never runs elevated.

## Known limits

- The voice assistant understands English only.
- An answer from the local text model is slow on a laptop: 9 to 15 seconds before the first word, and about half a minute the first time after a start.
- Voice was tested with recorded clips more than with live speech. The wake phrase "OK METEOR" has not been measured with many voices.
- Mirroring the notifications of other apps is not available. Windows allows it only to a signed and packaged app.
- The installer is not code-signed, so SmartScreen warns.
- The interface is translated into ten languages, but some newer lines are English only.
- With "The small island stays" on and the mascot shown, the resting island uses about 10 % of one processor core. With the mascot hidden it is about 1 %.
- The island lives on one display at a time.
- On some laptops the sensor library cannot read the fans.

[docs/ROADMAP.md](docs/ROADMAP.md) has the full list.

## Code signing policy

Releases are not code-signed yet. METEOR V1 is applying to the SignPath Foundation's free code signing for open-source projects. Once it is granted, release installers will be signed on these terms:

- Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).
- Every signed installer is built from this repository by the [build workflow](.github/workflows/build.yml) on GitHub's own runners, never on a private machine.
- Committers and reviewers: [the repository's contributors](https://github.com/shakibbinkabir/lintel/graphs/contributors).
- Approvers: [the repository's owner](https://github.com/shakibbinkabir).

Privacy policy: this program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it. [docs/PRIVACY.md](docs/PRIVACY.md) lists every address METEOR V1 can call and what turns each one on.

Until then, each release gives the SHA-256 of its installer so a download can be checked.

## Contributing

Everyone is welcome here, first-time contributors too.
[CONTRIBUTING.md](CONTRIBUTING.md) explains how to build, how to add a feature or a translation, and the few rules that are not negotiable.
[docs/ROADMAP.md](docs/ROADMAP.md) lists work that is open, sorted by size, if you want a place to start.
Please read the [code of conduct](CODE_OF_CONDUCT.md). To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Credits

METEOR V1 grew from three open-source projects: Coucou, whose Windows edition it was forked from, and Vorssaint and Boring Notch, whose ideas it re-implements for Windows. Thank you to their authors and contributors. Their names, logos, characters, sounds and artwork are not used here, and METEOR V1 is not affiliated with them.

[NOTICE.md](NOTICE.md) has the exact credits and every third-party component with its licence. The licence texts are in [licenses/](licenses/).

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
