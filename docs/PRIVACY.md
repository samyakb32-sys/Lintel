# Privacy: what METEOR V1 sends and where

METEOR V1 has no account and no server of its own. This page lists every host the app can call, what turns the call on, and what is sent.

| Host | Feature | What turns it on | What is sent | On by default |
|---|---|---|---|---|
| lrclib.net | Lyrics | Turning on "Fetch lyrics from LRCLIB" (the setting is off by default), then a song playing | Song title, artist, album and length | No. The feature is on, but the fetch setting is off. |
| api.open-meteo.com | Weather | Turning Weather on and choosing a place | Latitude and longitude of the chosen place | No. Weather is off by default. |
| geocoding-api.open-meteo.com | Weather | Searching for a city in Weather settings | The city name you typed | No. Weather is off by default. |
| speed.cloudflare.com | System monitor | Clicking the speed test button | Test downloads and uploads of random data. Nothing about you. | No. Click only. |
| github.com (and the release file host it redirects to) | Apps | Clicking "Check for updates" under Release notes | A plain request for the release feed. No identifier. | No. Click only. |
| winget sources (run by winget.exe, not by METEOR V1) | Apps | Clicking "Check now" or a package search, or choosing a daily or weekly check | What winget itself sends: your installed package list and search words | No. The schedule is off by default. |
| winget sources (run by winget.exe, not by METEOR V1) | Hardware sensors | Clicking "Install with winget" on the PawnIO row | What winget itself sends to install the package `namazso.PawnIO` | No. Click only, and the feature is off by default. |
| huggingface.co (and its file host) | Dictation | Clicking the button that downloads the speech model and engine | A plain download request for two pinned files | No. Dictation is off by default. |
| api.anthropic.com, or the gateway address you entered | Chat | Saving an Anthropic API key, then sending a message or listing models | Your key, your message and the context you chose | No. Needs a saved key. |
| api.openai.com | Chat | Saving an OpenAI API key, then sending a message or listing models | Your key, your message and the context you chose | No. Needs a saved key. |
| generativelanguage.googleapis.com | Chat | Saving a Google API key, then sending a message or listing models | Your key, your message and the context you chose | No. Needs a saved key. |
| openrouter.ai | Chat | Saving an OpenRouter API key, then sending a message or listing models | Your key, your message and the context you chose | No. Needs a saved key. |
| The address you entered (OpenAI-compatible server) | Chat | Entering an address and key in Settings | Your key (only to that address), your message and context | No. Needs an address. |
| The address you entered (Ollama or LM Studio; usual address 127.0.0.1) | Chat | Choosing the server, then sending a message or listing models | Your message and context | No. Needs a choice in Settings. |
| 127.0.0.1 on local ports | Chat | Opening the chat model list (looks for a local model server) | A model list request. It stays on this PC. | No. |
| api.github.com | Integrations (GitHub) | Saving a GitHub token, or turning on "Use the gh login" (off by default) | Your token and a request for your pull requests, repositories and checks | No. Needs a token. |
| api.stripe.com | Integrations (Stripe) | Saving a Stripe key | Your key and a read request for balance and charges | No. Needs a key. |
| api.vercel.com | Integrations (Vercel) | Saving a Vercel token | Your token and a read request for deployments | No. Needs a token. |
| api.resend.com | Integrations (Resend), Chat (send by email) | Saving a Resend key. Sending mail also needs your click. | Your key. For mail: the recipient, subject, text and attached file you chose. | No. Needs a key. |
| api.notion.com | Integrations (Notion) | Saving a Notion key | Your key and a search request | No. Needs a key. |
| api.cal.com | Integrations (Cal.com) | Saving a Cal.com key | Your key and a request for upcoming bookings | No. Needs a key. |
| The address you entered (n8n) | Integrations (n8n) | Saving an n8n address and API key | Your key (only to that address) and a read request | No. Needs both. |
| The address you entered (calendar link) | Calendar | Adding a calendar link in Calendar settings | A plain request for the .ics file. The link is stored in Credential Manager. | No. Needs a link. |

## Before you configure anything

On a fresh install with default settings, METEOR V1 makes no call to any host. This was checked for each case below.

- Lyrics: the feature is on, but the "fetch" setting is off, so no request is made, even while a song plays.
- Updates: the check runs only when you click "Check for updates". There is no check at start.
- Apps: the winget schedule is off. A scan or search runs only on your click.
- Weather, Dictation: off by default.
- Speed test: runs only on a click.
- Integrations: every poller returns at once when no key is saved. Without a key there is no request.
- Chat: needs a saved key, a chosen server or your click. Model lists, favicons and cover art are not fetched from the web. Album art comes from Windows, and the chat window scan only talks to this PC.

If you use a command-line agent for chat (such as Claude Code), that program makes its own calls. METEOR V1 does not control them.

## No telemetry

METEOR V1 has no analytics, no crash reporting and no usage counters. The code has no such client and no dependency for it. The Dictation engine says its own telemetry is on unless told otherwise, so METEOR V1 starts it with `NEEDLE_TELEMETRY=0` and `DO_NOT_TRACK=1`. The log records only that a lyrics lookup happened, never the song.

## Where keys are kept

API keys and tokens are stored in Windows Credential Manager through the `keyring` crate. They are not written to disk or to git. The settings window can ask only whether a key exists. A key is sent only to the one host it belongs to, and redirects are not followed for requests that carry a key.

Links that open in your browser on a click (for example the integration dashboards, a GitHub profile, release notes and website shortcuts in the command bar) are not calls made by METEOR V1.
