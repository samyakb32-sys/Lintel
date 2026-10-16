# Contributing to METEOR V1

Thank you for being here. Everyone is welcome to contribute, and that includes people who have never sent a pull request before. A typo fix, a better translation, a bug report with clear steps and a new feature are all real contributions. If something in this guide is unclear, open an issue and ask: the question is useful too.

Please read the [code of conduct](CODE_OF_CONDUCT.md) first. It is short.

## Build and run

You need Windows 10 or 11, Node.js, the Rust toolchain and the WebView2 runtime.

```
npm install
npm run build            # type-check and vite build (also builds the hook relay)
cargo check --workspace  # needs dist/ to exist, so run `npm run build` first
npm run tauri dev        # run the app
```

Two more commands help while you work:

```
npm test                 # the node --test suites
npm run fake-session     # a pretend agent session through the relay, while the app runs
```

`npm run pack` builds the installer into `release/`. It also builds the hardware helper, which needs the .NET 8 SDK. Nothing else needs it.

## A map of the tree

- `crates/<feature>/`: one Rust crate per feature. Each implements the `Feature` trait of `crates/core`.
- `crates/svc-<name>/`: shared services (audio, media, capture, input and so on) that several features use.
- `crates/core/`: the `Feature` trait, storage, the log, network helpers, secrets, translations and Win32 helpers.
- `crates/brand/`: the product's name in one place. The page side has `src/core/brand.ts`.
- `src-tauri/`: the app host. The island window, the feature registry, settings, the tray, shortcuts, the pipe server and the base features.
- `src/features/<feature>/`: the TypeScript side of each feature.
- `src/core/`, `src/ui/`, `src/island/`, `src/views/`, `src/settings/`, `src/window/`, `src/i18n/`: the shared page code. `src/ui/` holds the design tokens and components.
- `hook/`: the relay `meteor-hook.exe` that agent programs start on each hook event. It talks to the app over a named pipe.
- `helper/`: the hardware helper `meteor-hw.exe` (.NET). It is the only part that runs as administrator.
- `tests/`: the node test suites. `scripts/`: the generator, the packer and the checks.
- `docs/`: [ARCHITECTURE.md](docs/ARCHITECTURE.md), [FEATURES.md](docs/FEATURES.md), [PRIVACY.md](docs/PRIVACY.md), [VOICE.md](docs/VOICE.md) and [ROADMAP.md](docs/ROADMAP.md).

The programs are `meteor.exe`, `meteor-hook.exe`, `meteor-hw.exe` and `meteor-speak.exe`. Inside the code, crate names keep an older prefix (`notch-core`, `notch-feat-<id>`). That is on purpose: do not rename them.

## Add a feature

1. Pick a short id in lower case with dashes, for example `night-light`.
2. Run the generator:

   ```
   node scripts/new-feature.mjs night-light
   ```

   It creates `crates/night-light/` (the crate `notch-feat-night-light`, a `Feature` stub that is off by default) and `src/features/night-light/` (`index.ts`, `style.css`, `strings.json`, `tsconfig.json`). It adds one line to `src-tauri/Cargo.toml`. Nothing else needs to change: the app's `build.rs` and `src/core/registry.ts` find the feature by themselves.
3. Run `cargo check --workspace` once so `Cargo.lock` is complete.
4. Write the Rust side in `crates/night-light/src/lib.rs` and the page side in `src/features/night-light/index.ts`.
5. Check your feature alone while you work:

   ```
   cargo check -p notch-feat-night-light
   npx tsc --noEmit -p src/features/night-light
   ```

What a feature owns:

- Its two folders, and nothing outside them. If you need a change in shared code, say so in the pull request and keep that change separate.
- Its CSS classes, which all start with `.f-<id>-`.
- Its strings, in its own `strings.json`.
- Its settings, through `FeatureCtx.settings`. It reaches the island only through `IslandApi`.

The rule that matters most: **a feature that is off loads nothing.** No side effects when the Rust value is built or when the TypeScript file is imported. Everything starts in `start` and ends in `stop`: hooks, timers, threads, windows, subscriptions. Read sections 2, 3, 5 and 7 of [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before you begin, and section 6 if your feature opens a window.

## Add or fix a translation

Translations are very welcome, and you do not need to build the app to help.

- The strings of a feature live in `src/features/<id>/strings.json`. The strings of the base app live in `src/i18n/strings.json` and `src/i18n/extra.json`.
- Each entry is keyed by its English text and holds ten languages: `en`, `zh-Hans`, `hi`, `es`, `ar`, `fr`, `bn`, `pt-BR`, `ru`, `id`.
- Placeholders in braces, such as `{app}` and `{count}`, stay exactly as they are. `{app}` becomes the product's name. Move them to where they belong in your sentence.
- To fix a translation, change the value for your language and leave the key alone.
- New strings ship in English first. If you add one in code, add its entry with the `en` value; the other languages can follow in another pull request.
- Never put the name of another product in a string.

## The design language in five lines

- One look, dark everywhere: the island, its sections, the windows and Settings.
- Colours, sizes, radii and curves come only from the tokens in `src/ui/tokens.css`. Never write a colour value anywhere else.
- Buttons, switches, rows and cards come only from `src/ui/components.css` and `src/ui/components.ts`. Reuse the DOM helpers in `src/views/dom.ts`.
- Animate transform and opacity, not width or height, and respect `prefers-reduced-motion`.
- Plain TypeScript, DOM and Canvas. No UI framework.

Section 7 of [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) has the full design language. If your feature needs a component that does not exist, propose it in an issue before you invent it.

## Rules that are not negotiable

Each rule protects the person using METEOR V1.

- **Windows only.** METEOR V1 is made for Windows 10 and 11. Branches for other systems are code nobody can test here.
- **No telemetry and no account.** Network calls go only to services the user configured. People must be able to trust what leaves their computer.
- **Never act for the user without a click.** Nothing is approved, sent or deleted without an explicit confirmation. Above all, nothing ever answers a permission card except the user's own click.
- **Never block an agent program.** If the app does not answer, the relay exits at once and the agent carries on as if METEOR V1 were not installed.
- **Never overwrite an agent's settings file.** Dated backup, merge, show the diff, write only after the user confirms. That file belongs to the user.
- **Idle means idle.** No measurable processor use when the island is hidden and nothing is active. Prefer Windows events to polling, and pause any poller that nobody is watching.
- **Every feature can be turned off,** and a feature that is off loads nothing. People keep only what they need.
- **Secrets live only in Windows Credential Manager.** Never on disk, never in git, never handed to a web view.
- **The app runs unelevated.** Only the hardware helper asks for administrator rights, and only after the user opts in.
- **No upstream branding in code or assets.** The projects METEOR V1 grew from are credited in `NOTICE.md` and `README.md`. Their names, icons, characters, sounds and artwork must not appear in shipped code, strings or assets. `node scripts/check-brand.mjs` checks this.

Two smaller ones: keep the code minimal (reuse what is in the tree before adding a helper or a dependency), and name the upstream file in a header comment when a file is substantially ported from one.

## Propose a change

- **For anything large, open an issue first.** A new feature, a change to shared code or a change in behaviour is easier to agree on before the code exists. Small fixes can go straight to a pull request.
- **Keep pull requests small.** One change per pull request. A fix and a refactor are two pull requests.
- **Write a description that stands alone.** Say what changes for the user, why, how you tried it (what you clicked and what you saw), and add a screenshot for anything visible. Use made-up content in screenshots, never your own files or keys.
- **Run the checks before you push.** All four must pass:

  ```
  npm run build
  cargo check --workspace
  npm test
  node scripts/check-brand.mjs
  ```

- Mark a known shortcut in code with a one-line `// debt:` comment, and say so in the description.

## How reviews go

A maintainer reads every pull request. You can expect questions and requests for changes: they are about the code, not about you. A review looks at whether the change works when it is used, whether it keeps the rules above, and whether it fits the design language. Push further commits to the same branch to answer a review. If a pull request has been quiet for a while, a comment to ask is welcome. A pull request that cannot be merged gets a reason.

## Licence of contributions

METEOR V1 is licensed under GPL-3.0-or-later. By submitting a contribution you agree that it is licensed under the same terms, and you confirm that you have the right to submit it. If you bring in code from another project, say where it comes from and under which licence, so that `NOTICE.md` can credit it.

## Security

Do not open a public issue for a vulnerability. [SECURITY.md](SECURITY.md) says how to report one privately.
