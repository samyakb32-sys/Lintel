// Lyrics — a METEOR V1 feature. Opt-in: with "fetch" off (the default) no request is ever made.
// With it on, the song svc-media reports is looked up on LRCLIB and its text sent to the page,
// which finds the current line itself (position moves on by itself, see svc-media).
//
// A song is asked for once it is reported whole (title, artist, length) and has stayed the same
// for a moment. What the service said about a song is remembered; what it failed to say is not:
// an error, a timeout or a busy service is never "no lyrics".
// Which version is taken, the retries and the late answers: after boring.notch pull requests
// 1459 and 1542 (no code of theirs).

use notch_core::{Ctx, Feature, Group, Manifest, Use};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

static MANIFEST: Manifest = Manifest {
    id: "lyrics",
    name: "Lyrics",
    group: Group::Tools,
    default_on: true,
    uses: &[Use::Network],
    shortcuts: &[],
    tray: &[],
    defaults: r#"{ "fetch": false }"#,
};

/// Where the texts come from: part of every cache key.
pub const SOURCE: &str = "lrclib";
pub const BASE: &str = "https://lrclib.net";
/// A version is accepted only when its length is this close to the track's (seconds).
const MAX_DIFF: f64 = 3.0;
/// The service answers a search with at most this many versions: a full answer may have left the right one out.
const PAGE: usize = 20;
/// A new song is reported in pieces (the title, then its length): it is asked for once it has stayed the same this long.
const SETTLE: Duration = Duration::from_millis(2000);
/// A lookup that failed is made once more after this long, if the song is still the one.
const AGAIN: Duration = Duration::from_secs(20);
const TIMEOUT: Duration = Duration::from_secs(10);
/// Seconds before the second and the third try of a request the service was too busy for, unless it says how long (`Retry-After`).
const BACKOFF: [u64; 2] = [1, 3];
/// A `Retry-After` longer than this (seconds) is not sat out: the lookup fails, and nothing is kept of it.
const MAX_WAIT: u64 = 20;
const CACHE_MAX: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub ms: u64,
    pub text: String,
}

/// A song's text. Not `synced`: the lines have no times (all 0) and nothing follows the position.
#[derive(Debug, Clone, PartialEq)]
pub struct Lyric {
    pub lines: Vec<Line>,
    pub synced: bool,
    /// What the version agreed with, for the probe: "title, artist, album", "title, artist" or "cleaned title".
    pub matched: &'static str,
}

/// What the player says it plays.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Seconds; 0 while the player has not said.
    pub duration: f64,
}

impl Track {
    /// Enough to ask with: a title, an artist and a length. The first report of a song often lacks one.
    pub fn complete(&self) -> bool {
        !self.title.trim().is_empty() && !self.artist.trim().is_empty() && self.duration >= 1.0
    }
}

/// `[mm:ss.xx]text`, several tags per line allowed; `[ar:..]`-style tags are skipped.
pub fn parse_lrc(src: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(body) = rest.strip_prefix('[') {
            let Some(end) = body.find(']') else { break };
            let tag = &body[..end];
            let Some((m, s)) = tag.split_once(':') else { break };
            match (m.trim().parse::<u64>(), s.trim().parse::<f64>()) {
                (Ok(m), Ok(s)) => stamps.push(m * 60_000 + (s * 1000.0).round() as u64),
                _ => break,
            }
            rest = body[end + 1..].trim_start();
        }
        for ms in stamps {
            out.push(Line { ms, text: rest.to_string() });
        }
    }
    out.sort_by_key(|l| l.ms);
    out
}

/// The line being sung at `ms`: the last one that has started. `None` before the first.
pub fn line_at(lines: &[Line], ms: u64) -> Option<&str> {
    let n = lines.partition_point(|l| l.ms <= ms);
    n.checked_sub(1).map(|i| lines[i].text.as_str())
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

// ── Is it the same song ───────────────────────────────────────────────────────

/// Letters and digits in lower case with one space between words: "Don't Stop Me Now!" is "dont stop me now".
fn norm(s: &str) -> String {
    let mut out = String::new();
    for c in s.replace('&', " and ").chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !matches!(c, '\'' | '’' | '`') && !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

/// Words that mark a note a player or a shop adds to a title. "remix" and "live" are not among
/// them: those are other recordings.
const NOTES: [&str; 15] = [
    "remaster", "remastered", "feat", "ft", "featuring", "version", "edit", "mono", "stereo", "deluxe", "bonus", "explicit",
    "anniversary", "edition", "mix",
];

fn is_note(s: &str) -> bool {
    let low = s.to_lowercase();
    low.trim_start().starts_with("with ") || low.split(|c: char| !c.is_alphanumeric()).any(|w| NOTES.contains(&w))
}

/// The title without its notes: "(Remastered 2011)", "[feat. X]", " - Radio Edit", " feat. X".
pub fn clean(title: &str) -> String {
    let mut out = String::new();
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest[open..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[open..].find(close) else { break };
        out.push_str(&rest[..open]);
        if !is_note(&rest[open + 1..open + len]) {
            out.push_str(&rest[open..=open + len]);
        }
        rest = &rest[open + len + 1..];
    }
    out.push_str(rest);
    if let Some(dash) = out.rfind(" - ").filter(|dash| is_note(&out[dash + 3..])) {
        out.truncate(dash);
    }
    // Same length as `out`: only ASCII letters change.
    let low = out.to_ascii_lowercase();
    if let Some(feat) = [" feat. ", " feat ", " ft. ", " ft ", " featuring "].iter().filter_map(|f| low.find(f)).min() {
        out.truncate(feat);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every name in an artist line: "A, B & C feat. D" is four of them.
fn artists(line: &str) -> Vec<String> {
    let mut low = format!(" {} ", line.to_lowercase());
    for sep in [" feat. ", " feat ", " ft. ", " ft ", " featuring ", " with ", " x ", " and ", "&", ";", "/"] {
        low = low.replace(sep, ",");
    }
    low.split(',').map(norm).filter(|name| !name.is_empty()).collect()
}

/// The first name of one line is among the names of the other.
fn same_artist(a: &str, b: &str) -> bool {
    let (a, b) = (artists(a), artists(b));
    a.first().is_some_and(|name| b.contains(name)) || b.first().is_some_and(|name| a.contains(name))
}

/// The version in a search answer that is `track`: same artist, same title, and a length within
/// `MAX_DIFF`. A synced text before a plain one; then the title as it is before the title without
/// its notes, the same album before another one, and the closest length.
pub fn pick(answer: &Value, track: &Track) -> Option<Lyric> {
    let (title, bare) = (norm(&track.title), norm(&clean(&track.title)));
    let album = norm(&clean(&track.album));
    answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let text = |key: &str| r[key].as_str().unwrap_or("");
            let diff = (r["duration"].as_f64()? - track.duration).abs();
            if diff > MAX_DIFF || !same_artist(text("artistName"), &track.artist) {
                return None;
            }
            let exact = norm(text("trackName")) == title;
            if !exact && norm(&clean(text("trackName"))) != bare {
                return None;
            }
            let same_album = !album.is_empty() && norm(&clean(text("albumName"))) == album;
            let matched = match (exact, same_album) {
                (true, true) => "title, artist, album",
                (true, false) => "title, artist",
                _ => "cleaned title",
            };
            let synced = parse_lrc(text("syncedLyrics"));
            let lyric = if synced.is_empty() {
                let plain = text("plainLyrics").lines().map(str::trim).filter(|l| !l.is_empty());
                Lyric { lines: plain.map(|l| Line { ms: 0, text: l.to_string() }).collect(), synced: false, matched }
            } else {
                Lyric { lines: synced, synced: true, matched }
            };
            (!lyric.lines.is_empty()).then_some(((!lyric.synced, !exact, !same_album), diff, lyric))
        })
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))
        .map(|(_, _, lyric)| lyric)
}

// ── Asking the service ────────────────────────────────────────────────────────

/// Why a request gave nothing. `again`: the service may answer later (busy, out of reach), in `wait` seconds if it said so.
struct Fail {
    error: String,
    again: bool,
    wait: Option<u64>,
}

async fn once(url: &str) -> Result<Vec<u8>, Fail> {
    let busy = |error: String| Fail { error, again: true, wait: None };
    let url = notch_core::net::fetchable(url).map_err(|error| Fail { error, again: false, wait: None })?;
    let client = notch_core::net::client(&url, TIMEOUT).map_err(busy)?;
    // The address stays out of the error: it holds the song, and errors are logged.
    let sent = client.get(url).header("User-Agent", notch_brand::SLUG).send().await;
    let response = sent.map_err(|e| busy(e.without_url().to_string()))?;
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return notch_core::net::read_capped(response, 2 << 20).await.map_err(busy);
    }
    let wait = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
    Err(Fail { error: format!("HTTP {status}"), again: matches!(status, 408 | 425 | 429) || status >= 500, wait })
}

/// One GET of the service. Busy or out of reach, it is asked twice more: after the time it names
/// (`Retry-After`), else after 1 and 3 seconds. Given up at once when `wanted` says the song has
/// moved on. An error is an error: the caller keeps nothing of it.
pub async fn get(url: &str, wanted: &(dyn Fn() -> bool + Sync)) -> Result<Vec<u8>, String> {
    let mut pauses = BACKOFF.into_iter();
    loop {
        if !wanted() {
            return Err("the song changed".into());
        }
        let fail = match once(url).await {
            Ok(body) => return Ok(body),
            Err(fail) => fail,
        };
        let Some(pause) = pauses.next().filter(|_| fail.again) else { return Err(fail.error) };
        let wait = fail.wait.unwrap_or(pause);
        if wait > MAX_WAIT {
            return Err(fail.error);
        }
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

/// Looks `track` up. `ask` makes one search of the service from its query string (`get`, in the
/// app). First the title without its notes and the first artist: one request answers for the
/// title as it is, with and without the album, and for the cleaned one (`pick`). Only when that
/// answer is full and holds no synced version is the service asked again, for the album alone.
/// `Ok(None)`: the service answered and has no such song. `Err`: it did not answer.
pub async fn lookup<F, Fut>(track: &Track, ask: F) -> Result<Option<Lyric>, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let ask = &ask;
    let search = |query: String| async move {
        let body = ask(query).await?;
        serde_json::from_slice::<Value>(&body).map_err(|e| format!("not an answer: {e}"))
    };
    let artist = enc(&artists(&track.artist).into_iter().next().unwrap_or_default());
    let bare = Some(clean(&track.title)).filter(|t| !t.is_empty()).unwrap_or_else(|| track.title.clone());
    let answer = search(format!("track_name={}&artist_name={artist}", enc(&bare))).await?;
    let mut best = pick(&answer, track);
    let full = answer.as_array().is_some_and(|versions| versions.len() >= PAGE);
    if full && !track.album.trim().is_empty() && !best.as_ref().is_some_and(|l| l.synced) {
        let narrow = format!("track_name={}&artist_name={artist}&album_name={}", enc(&track.title), enc(&track.album));
        let more = pick(&search(narrow).await?, track);
        if best.is_none() || more.as_ref().is_some_and(|l| l.synced) {
            best = more;
        }
    }
    Ok(best)
}

// ── Which song, and what is known of it ───────────────────────────────────────

/// Source, title, artist, album, whole seconds.
type Key = (&'static str, String, String, String, u64);

fn key(t: &Track) -> Key {
    (SOURCE, t.title.clone(), t.artist.clone(), t.album.clone(), t.duration.round() as u64)
}

/// A lookup under way, and the song it is for.
#[derive(Debug, Clone)]
pub struct Ticket {
    turn: u64,
    pub track: Track,
}

/// The song that plays, the state of its text, and what the service said of the songs before it.
#[derive(Default)]
pub struct Tracker {
    fetch: bool,
    track: Option<Track>,
    /// Counts the songs and the turns of the switch: an answer with an older number came too late.
    turn: u64,
    status: &'static str,
    lyric: Option<Lyric>,
    /// Changes with `lyric`: the page builds its lines again only then.
    rev: u64,
    /// What the service said, by source and song, the oldest first. An error is never in it.
    cache: VecDeque<(Key, Option<Lyric>)>,
}

impl Tracker {
    /// "off" (nothing asked: switched off, no song, or a report that is not whole yet), "loading",
    /// "found", "none" (the service has no such song) or "error" (it did not answer).
    pub fn status(&self) -> &'static str {
        if self.status.is_empty() { "off" } else { self.status }
    }

    pub fn lyric(&self) -> Option<&Lyric> {
        self.lyric.as_ref()
    }

    /// The switch in Settings.
    pub fn set_fetch(&mut self, on: bool) -> Option<Ticket> {
        if on == self.fetch {
            return None;
        }
        self.fetch = on;
        self.begin()
    }

    /// What the player reports. The song that is already known changes nothing; another one
    /// takes the old text away at once. A ticket: this song is to be looked up.
    pub fn report(&mut self, track: Option<Track>) -> Option<Ticket> {
        let track = track.filter(|t| !t.title.is_empty());
        if track.as_ref().map(key) == self.track.as_ref().map(key) {
            return None;
        }
        self.track = track;
        self.begin()
    }

    /// A lookup that failed, asked for again.
    pub fn retry(&mut self) -> Option<Ticket> {
        if self.status != "error" {
            return None;
        }
        self.begin()
    }

    fn begin(&mut self) -> Option<Ticket> {
        self.turn += 1;
        self.show("off", None);
        // Not whole: nothing is asked and nothing is concluded, the next report decides.
        let track = self.track.clone().filter(|t| self.fetch && t.complete())?;
        let wanted = key(&track);
        if let Some((_, known)) = self.cache.iter().find(|(k, _)| *k == wanted) {
            let known = known.clone();
            self.show(if known.is_some() { "found" } else { "none" }, known);
            return None;
        }
        self.status = "loading";
        Some(Ticket { turn: self.turn, track })
    }

    fn show(&mut self, status: &'static str, lyric: Option<Lyric>) {
        self.status = status;
        if lyric != self.lyric {
            self.lyric = lyric;
            self.rev += 1;
        }
    }

    /// The song of `ticket` is still the one, and nothing has asked for it again since.
    pub fn wanted(&self, ticket: &Ticket) -> bool {
        ticket.turn == self.turn && self.track.as_ref().map(key) == Some(key(&ticket.track))
    }

    /// The end of a lookup. What the service said is kept for that song, whoever plays now; an
    /// error is kept nowhere. `false`: the song has moved on meanwhile and nothing is shown.
    pub fn answer(&mut self, ticket: &Ticket, result: Result<Option<Lyric>, String>) -> bool {
        if let Ok(said) = &result {
            let k = key(&ticket.track);
            self.cache.retain(|(c, _)| *c != k);
            self.cache.push_back((k, said.clone()));
            if self.cache.len() > CACHE_MAX {
                self.cache.pop_front();
            }
        }
        if !self.wanted(ticket) {
            return false;
        }
        match result {
            Ok(said) => self.show(if said.is_some() { "found" } else { "none" }, said),
            Err(_) => self.status = "error",
        }
        true
    }
}

// ── The feature ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct State {
    tracker: Tracker,
    playing: bool,
    position: f64,
    at: u64,
    rate: f64,
}

impl State {
    fn json(&self) -> Value {
        let t = &self.tracker;
        let lines: Vec<_> = t.lyric.iter().flat_map(|l| &l.lines).map(|l| json!([l.ms, l.text])).collect();
        json!({ "track": t.track.is_some(), "fetch": t.fetch, "status": t.status(), "rev": t.rev,
                "synced": t.lyric.as_ref().is_some_and(|l| l.synced), "lines": lines,
                "playing": self.playing, "position": self.position, "at": self.at, "rate": self.rate })
    }
}

type Shared = Arc<Mutex<State>>;

#[derive(Default)]
pub struct Lyrics {
    state: Shared,
    sub: Mutex<Option<svc_media::Sub>>,
}

fn push(cx: &Ctx, st: &Shared) {
    let v = st.lock().unwrap().json();
    cx.emit("lyrics", "state", v);
}

/// Looks the ticket's song up once it has stayed the same for `settle`, and once more a while
/// later if the service did not answer. Ends without a word as soon as the song moves on.
fn look(cx: &Ctx, st: &Shared, ticket: Ticket, settle: Duration) {
    let (cx, st) = (cx.clone(), st.clone());
    tauri::async_runtime::spawn(async move {
        let wanted = || st.lock().unwrap().tracker.wanted(&ticket);
        let wanted: &(dyn Fn() -> bool + Sync) = &wanted;
        for pause in [settle, AGAIN] {
            tokio::time::sleep(pause).await;
            if !wanted() {
                return;
            }
            // Said in the log, without the track: the one moment something leaves for lrclib.net.
            notch_core::log::line("[lyrics] looking a track up on lrclib.net");
            let result = lookup(&ticket.track, |query| async move { get(&format!("{BASE}/api/search?{query}"), wanted).await }).await;
            let failed = result.as_ref().err().cloned();
            if !st.lock().unwrap().tracker.answer(&ticket, result) {
                return;
            }
            push(&cx, &st);
            let Some(error) = failed else { return };
            notch_core::log::line(format!("[lyrics] lookup failed: {error}"));
        }
    });
}

impl Feature for Lyrics {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        self.state.lock().unwrap().tracker.set_fetch(cx.settings("lyrics")["fetch"].as_bool() == Some(true));
        let (cx2, st) = (cx.clone(), self.state.clone());
        let report = move |p: Option<&svc_media::Playback>| {
            let ticket = {
                let mut s = st.lock().unwrap();
                if let Some(p) = p {
                    (s.playing, s.position, s.at, s.rate) = (p.playing, p.position, p.at_ms, p.rate);
                }
                s.tracker.report(p.map(|p| Track {
                    title: p.title.clone(),
                    artist: p.artist.clone(),
                    album: p.album.clone(),
                    duration: p.duration,
                }))
            };
            if let Some(ticket) = ticket {
                look(&cx2, &st, ticket, SETTLE);
            }
            push(&cx2, &st);
        };
        *self.sub.lock().unwrap() = Some(svc_media::subscribe(report.clone()));
        // The service only tells of changes: what already plays is read here.
        report(svc_media::state().as_ref());
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        self.sub.lock().unwrap().take();
        *self.state.lock().unwrap() = State::default();
    }

    fn settings_changed(&self, cx: &Ctx, settings: &Value) {
        let ticket = self.state.lock().unwrap().tracker.set_fetch(settings["fetch"].as_bool() == Some(true));
        if let Some(ticket) = ticket {
            look(cx, &self.state, ticket, Duration::ZERO);
        }
        push(cx, &self.state);
    }

    fn call(&self, cx: &Ctx, method: &str, _args: Value) -> Result<Value, String> {
        match method {
            "state" => Ok(self.state.lock().unwrap().json()),
            "retry" => {
                let ticket = self.state.lock().unwrap().tracker.retry();
                if let Some(ticket) = ticket {
                    look(cx, &self.state, ticket, Duration::ZERO);
                }
                Ok(self.state.lock().unwrap().json())
            }
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/lyrics/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(Lyrics::default())
}
