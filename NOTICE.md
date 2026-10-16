# Notice

METEOR V1 is licensed under the GNU General Public License, version 3 or (at your option) any later version. See [LICENSE](LICENSE).

This file says where METEOR V1 comes from and names every third-party component it ships or downloads, with its licence. The licence texts are in [licenses/](licenses/).

## Where METEOR V1 comes from

METEOR V1 grew from three open-source projects. It would not exist without them, and their authors and contributors have our thanks.

### Coucou (the project METEOR V1 was forked from)

- Source: https://github.com/Louis-CFM/coucou
- Copyright (c) 2026 Louis Raillé. Code licensed under the MIT License; the full text is in [licenses/coucou-MIT.txt](licenses/coucou-MIT.txt).
- What came from it: METEOR V1 started as a fork of the Tauri application in its `windows/` folder. The island window, the relay that agent programs call, the permission, question and finished cards, the chat, the integrations and the weekly recap all began as that code.
- Fixes from open pull requests of Coucou were ported in October 2026 (the relay pipe's access list and timeouts, the relay-only permission card, keys sent only over https, hook clean-up on uninstall, the pretend session script, the wake strip, rendering on scaled displays, the default Gemini model). Thanks to their authors: w1w4, morg1z, Ohualtex, arreina, Totopo27, hieudepzai14122007-ship-it, JhoanG956 and Edurique.
- The names "Coucou" and "Mochi", the Mochi character, the Coucou icons, sounds and media are **not** covered by the MIT License and are **not** used here. METEOR V1 has its own name, character, icon and sounds, and is not affiliated with or endorsed by Coucou or its author.

### Vorssaint (features re-implemented for Windows)

- Source: https://github.com/vorssaint/vorssaint-utils
- Copyright 2026 Vorssaint. Licensed under GPL-3.0-or-later; the full text is in [LICENSE](LICENSE).
- What came from it: most of the system utilities (among them the system monitor, the clipboard history, the audio tools, the input tools, the window layout, capture and the cleaner) and several island behaviours are Windows re-implementations of features of this macOS app, written with reference to its source code. A file that is substantially ported names the file it came from in its header.
- The Vorssaint name, logo and look are covered by its trademark policy and are **not** used here. METEOR V1 is not affiliated with or endorsed by Vorssaint.

### Boring Notch (features re-implemented for Windows)

- Source: https://github.com/TheBoredTeam/boring.notch
- Copyright TheBoredTeam. Licensed under GPL-3.0; the full text is in [LICENSE](LICENSE).
- What came from it: several island features (among them the music player and its lyrics, the shelf, the level displays, the calendar strip and the battery notices) are Windows re-implementations of features of this macOS app, written with reference to its source code.
- METEOR V1 is not affiliated with or endorsed by TheBoredTeam.

## Needle engine and Whistle model (Cactus Compute)

- Dictation can transcribe speech on this device with Cactus Compute's Needle engine and Whistle speech model. Both are Apache-2.0; the full text is in [licenses/apache-2.0.txt](licenses/apache-2.0.txt).
- Needle engine (`needle.exe`): https://huggingface.co/Cactus-Compute/needle3/tree/main/windows-x86_64
- Whistle model (`whistle.cact`): https://huggingface.co/Cactus-Compute/whistle
- Nothing of them is bundled or committed: the app downloads both files, pinned to a repository revision and checked by SHA-256, only when the user clicks the download button in the Dictation settings. METEOR V1 is not affiliated with or endorsed by Cactus Compute.

## Poppins typeface

- The interface is set in Poppins, copyright 2020 The Poppins Project Authors (https://github.com/itfoundry/Poppins), licensed under the SIL Open Font License 1.1; the full text is in [licenses/OFL-1.1-poppins.txt](licenses/OFL-1.1-poppins.txt).
- The font files are bundled with the app through the `@fontsource/poppins` package. Nothing is loaded from the network.
- Other scripts use further open fonts, bundled the same way and licensed under the SIL Open Font License 1.1: Hind Siliguri for Bengali (copyright 2015 Indian Type Foundry, `@fontsource/hind-siliguri`, [licenses/OFL-1.1-hind-siliguri.txt](licenses/OFL-1.1-hind-siliguri.txt)), Montserrat for Cyrillic (copyright 2011 The Montserrat Project Authors, https://github.com/JulietaUla/Montserrat, `@fontsource/montserrat`, [licenses/OFL-1.1-montserrat.txt](licenses/OFL-1.1-montserrat.txt)) and Cairo for Arabic (copyright 2009 The Cairo Project Authors, https://github.com/Gue3bara/Cairo, `@fontsource/cairo`, [licenses/OFL-1.1-cairo.txt](licenses/OFL-1.1-cairo.txt)).

## Hardware helper (`lintel-hw.exe`)

The installer ships the hardware helper as one file with its own .NET runtime. It contains:

- LibreHardwareMonitorLib 0.9.6 (https://github.com/LibreHardwareMonitor/LibreHardwareMonitor), DiskInfoToolkit 1.1.2, RAMSPDToolkit-NDD 1.4.2 and BlackSharp.Core 1.0.7 (https://github.com/Blacktempel), licensed under the Mozilla Public License 2.0; the full text is in [licenses/MPL-2.0.txt](licenses/MPL-2.0.txt). They are used unmodified; their source is at the addresses above.
- HidSharp 2.6.4 (James F. Bellinger and contributors), licensed under Apache-2.0; the full text is in [licenses/apache-2.0.txt](licenses/apache-2.0.txt).
- Mono.Posix.NETStandard 1.0.0, System.Management 10.0.2, System.IO.Ports 10.0.3, System.CodeDom 10.0.2, System.Threading.AccessControl 10.0.3 and the .NET runtime 8.0.31 (https://github.com/dotnet/runtime), copyright (c) .NET Foundation and Contributors and Microsoft Corporation, licensed under the MIT License; the full text is in [licenses/MIT-dotnet-runtime.txt](licenses/MIT-dotnet-runtime.txt).
- The PawnIO driver is **not** bundled: the helper uses it when the user has installed it themselves (https://pawnio.eu).

## Natural voice (`lintel-speak.exe`)

The installer ships the voice assistant's speech server with two libraries beside it, in the folder `speak`:

- sherpa-onnx 1.13.8 (`sherpa-onnx-c-api.dll`, https://github.com/k2-fsa/sherpa-onnx), licensed under Apache-2.0; the full text is in [licenses/apache-2.0.txt](licenses/apache-2.0.txt). It carries espeak-ng (https://github.com/espeak-ng/espeak-ng), licensed under GPL-3.0-or-later, as METEOR V1 is; the full text is in [LICENSE](LICENSE). Both are used unmodified; their source is at the addresses above.
- ONNX Runtime (`onnxruntime.dll`, https://github.com/microsoft/onnxruntime), copyright (c) Microsoft Corporation, licensed under the MIT License; the full text is in [licenses/MIT-onnxruntime.txt](licenses/MIT-onnxruntime.txt).
- The voice itself is **not** bundled: the Kokoro-82M v1.0 model (https://huggingface.co/hexgrad/Kokoro-82M, licensed under Apache-2.0), in the export made for sherpa-onnx (https://huggingface.co/csukuangfj/kokoro-multi-lang-v1_0), and the espeak-ng data are downloaded, pinned and checked by SHA-256, only when the user clicks the download button in the Voice settings.

## Local text model (llama.cpp and Qwen3.5-4B)

- The voice assistant can answer questions on this device with llama.cpp's `llama-server` (https://github.com/ggml-org/llama.cpp), licensed under the MIT License, running the Qwen3.5-4B model (https://huggingface.co/unsloth/Qwen3.5-4B-GGUF), licensed under Apache-2.0; the full text is in [licenses/apache-2.0.txt](licenses/apache-2.0.txt).
- Nothing of them is bundled or committed: the app downloads both, pinned to a release and to a repository revision and checked by SHA-256, only when the user clicks the download button in the Voice settings. METEOR V1 is not affiliated with or endorsed by their authors.

## Other shipped components

- This product includes taktus, a tempo and beat engine by Shakib Bin Kabir (https://github.com/shakibbinkabir/taktus), licensed under Apache-2.0; the full text is in [licenses/apache-2.0.txt](licenses/apache-2.0.txt).
- QR codes in a capture are read with rqrr (https://github.com/WanzenBug/rqrr), licensed under (MIT OR Apache-2.0) AND ISC.
- The interface runs in the Microsoft Edge WebView2 runtime. It is **not** bundled: it is part of Windows 11, and on Windows 10 the installer fetches Microsoft’s own installer when it is missing.

## Rust and npm dependencies, by license

The app links the open-source Rust crates counted below (those of a Windows build; METEOR V1's own crates are left out) and ships the npm packages named under the table. Each crate's own copyright notice and license text are in its source, at https://crates.io/crates/<name>. The table is written by `node scripts/licenses.mjs`.

<!-- deps:start -->
| License | Rust crates | Which |
| --- | ---: | --- |
| MIT OR Apache-2.0 | 148 |  |
| MIT | 56 |  |
| Apache-2.0 OR MIT | 31 |  |
| Unicode-3.0 | 18 |  |
| Unlicense OR MIT | 11 |  |
| MPL-2.0 | 5 | cssparser, cssparser-macros, dtoa-short, option-ext, selectors |
| BSD-3-Clause | 3 | alloc-no-stdlib, alloc-stdlib, subtle |
| Apache-2.0 | 3 | sync_wrapper, taktus, tao |
| Zlib | 2 | foldhash, zlib-rs |
| Apache-2.0 OR ISC OR MIT | 2 | hyper-rustls, rustls |
| MIT OR Apache-2.0 OR Zlib | 2 | lru-slab, raw-window-handle |
| MIT OR Zlib OR Apache-2.0 | 1 | miniz_oxide |
| ISC | 2 | rustls-webpki, untrusted |
| 0BSD OR MIT OR Apache-2.0 | 1 | adler2 |
| BSD-3-Clause AND MIT | 1 | brotli |
| BSD-3-Clause OR MIT | 1 | brotli-decompressor |
| Apache-2.0 AND MIT | 1 | dpi |
| CC0-1.0 OR MIT-0 OR Apache-2.0 | 1 | dunce |
| Apache-2.0 AND ISC | 1 | ring |
| (MIT OR Apache-2.0) AND ISC | 1 | rqrr |
| Apache-2.0 OR BSL-1.0 | 1 | ryu |
| Zlib OR Apache-2.0 OR MIT | 1 | tinyvec |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 1 | unicode-ident |
| CDLA-Permissive-2.0 | 1 | webpki-roots |

npm packages shipped in the app: @fontsource/cairo 5.3.0 (OFL-1.1), @fontsource/hind-siliguri 5.3.0 (OFL-1.1), @fontsource/montserrat 5.3.0 (OFL-1.1), @fontsource/poppins 5.3.0 (OFL-1.1), @tauri-apps/api 2.12.0 (Apache-2.0 OR MIT).
<!-- deps:end -->
