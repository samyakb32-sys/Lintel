// Weather by voice and the short lookup: the only two network calls of the questions (VOICE §5, §8);
// everything else is answered in the page from kept state. Package voice-questions.
//
// - Weather: Open-Meteo (the same free, keyless service as the Weather feature, crates/weather). Only
//   while the setting `weather` is on and `city` is not empty. The city's name goes to the geocoding
//   host once (the answer is kept for as long as the city is the same); the forecast request carries
//   nothing but the coordinates.
// - Lookup: Wikipedia's action API, only while the setting `lookup` is on. Wikipedia rather than
//   DuckDuckGo's instant answer: one request finds the best page for a free question and returns its
//   first sentences as plain text, always the same shape; DuckDuckGo's instant answer is empty for most
//   open questions and answers with an empty 202 when it rate-limits. The words asked leave the PC.
// With a switch off nothing is requested, whatever else is set.

use crate::{Result, VoiceError};
use notch_core::{net, Ctx};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(8);
const LIMIT: usize = 128 * 1024;
/// What is spoken of a page: at most this many characters, cut at a sentence.
const SPOKEN: usize = 320;

/// The three addresses, so that a probe can point them at a stand-in on this machine.
#[derive(Clone, Debug)]
pub struct Urls {
    pub geocode: String,
    pub forecast: String,
    pub wiki: String,
}

impl Urls {
    pub fn live() -> Self {
        Urls {
            geocode: "https://geocoding-api.open-meteo.com/v1/search".into(),
            forecast: "https://api.open-meteo.com/v1/forecast".into(),
            wiki: "https://en.wikipedia.org/w/api.php".into(),
        }
    }
}

pub fn start(_cx: &Ctx) -> Result<()> {
    Ok(())
}

pub fn stop(_cx: &Ctx) {
    *PLACE.lock().unwrap() = None;
}

pub fn settings(_cx: &Ctx, _settings: &Value) {}

/// The place kept for the last city asked: (the city as typed, name, lat, lon).
static PLACE: Mutex<Option<(String, String, f64, f64)>> = Mutex::new(None);

fn encode(s: &str) -> String {
    s.bytes().fold(String::new(), |mut o, b| {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
        o
    })
}

fn get(url: &str) -> Result<Value> {
    let body = tauri::async_runtime::block_on(net::bounded_get(url, TIMEOUT, LIMIT)).map_err(VoiceError::Failed)?;
    serde_json::from_slice(&body).map_err(|e| VoiceError::Failed(e.to_string()))
}

// ── The switches ──────────────────────────────────────────────────────────────

/// The city to ask about, or why not: the switch is off, or no city is set.
pub fn gate_weather(s: &Value) -> Result<String> {
    if s["weather"] != json!(true) {
        return Err(VoiceError::Refused("Weather by voice is off.".into()));
    }
    let city = s["city"].as_str().unwrap_or("").trim();
    if city.is_empty() {
        return Err(VoiceError::Refused("No city is set for the weather.".into()));
    }
    Ok(city.to_string())
}

pub fn gate_lookup(s: &Value) -> Result<()> {
    if s["lookup"] != json!(true) {
        return Err(VoiceError::Refused("Online lookup is off.".into()));
    }
    Ok(())
}

// ── Weather ───────────────────────────────────────────────────────────────────

fn places(urls: &Urls, name: &str, count: u8) -> Result<Vec<Value>> {
    let v = get(&format!("{}?name={}&count={count}&language=en&format=json", urls.geocode, encode(name)))?;
    Ok(v["results"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(json!({
                        "name": r["name"].as_str()?, "country": r["country"].as_str().unwrap_or(""),
                        "lat": r["latitude"].as_f64()?, "lon": r["longitude"].as_f64()?,
                    }))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Cities by name for the Settings page's field.
pub fn find_with(urls: &Urls, city: &str) -> Result<Value> {
    Ok(Value::Array(places(urls, city.trim(), 5)?))
}

/// The WMO code in words.
pub fn describe(code: i64) -> &'static str {
    match code {
        0 => "clear",
        1 => "mostly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "foggy",
        51..=57 => "drizzling",
        61..=65 => "rainy",
        66 | 67 => "freezing rain",
        71..=77 => "snowy",
        80..=82 => "showery",
        85 | 86 => "snow showers",
        95 => "stormy",
        96 | 99 => "stormy with hail",
        _ => "unsettled",
    }
}

/// `day` 0 is today (the current reading), 1 tomorrow (its forecast).
pub fn now_with(urls: &Urls, city: &str, day: usize, fahrenheit: bool) -> Result<Value> {
    let name = city.split(',').next().unwrap_or(city).trim().to_string();
    let kept = PLACE.lock().unwrap().clone().filter(|p| p.0 == city);
    let (shown, lat, lon) = match kept {
        Some((_, n, la, lo)) => (n, la, lo),
        None => {
            let first = places(urls, &name, 1)?.into_iter().next().ok_or_else(|| VoiceError::Failed("City not found.".into()))?;
            let (n, la, lo) = (first["name"].as_str().unwrap_or(&name).to_string(), first["lat"].as_f64().unwrap_or(0.0), first["lon"].as_f64().unwrap_or(0.0));
            *PLACE.lock().unwrap() = Some((city.to_string(), n.clone(), la, lo));
            (n, la, lo)
        }
    };
    let (unit, tail) = if fahrenheit { ("F", "&temperature_unit=fahrenheit") } else { ("C", "") };
    let v = get(&format!(
        "{}?latitude={lat}&longitude={lon}&current=temperature_2m,apparent_temperature,weather_code\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max&forecast_days=2&timezone=auto{tail}",
        urls.forecast
    ))?;
    let d = &v["daily"];
    let (high, low) = (d["temperature_2m_max"][day].as_f64(), d["temperature_2m_min"][day].as_f64());
    let (temp, feels, code) = if day == 0 {
        (v["current"]["temperature_2m"].as_f64(), v["current"]["apparent_temperature"].as_f64(), v["current"]["weather_code"].as_i64())
    } else {
        (high, high, d["weather_code"][day].as_i64())
    };
    let (Some(temp), Some(code), Some(high), Some(low)) = (temp, code, high, low) else {
        return Err(VoiceError::Failed("The forecast was incomplete.".into()));
    };
    Ok(json!({
        "city": shown, "temp": temp, "unit": unit, "feels": feels.unwrap_or(temp), "high": high, "low": low,
        "code": code, "text": describe(code), "rainChance": d["precipitation_probability_max"][day].as_f64().unwrap_or(0.0),
    }))
}

// ── Lookup ────────────────────────────────────────────────────────────────────

/// The first sentences of an extract, without parentheses (pronunciations, dates), cut at a sentence end.
pub fn speakable(extract: &str) -> String {
    let (mut out, mut depth) = (String::new(), 0u32);
    for c in extract.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(if c == '\n' { ' ' } else { c }),
            _ => {}
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ").replace(" ,", ",").replace(" .", ".");
    if out.chars().count() <= SPOKEN {
        return out;
    }
    let cut: String = out.chars().take(SPOKEN).collect();
    match cut.rfind(". ") {
        Some(i) => cut[..=i].to_string(),
        None => format!("{}.", cut.trim_end_matches(|c: char| !c.is_alphanumeric()).rsplit_once(' ').map_or(cut.as_str(), |x| x.0)),
    }
}

pub fn lookup_with(urls: &Urls, query: &str) -> Result<Value> {
    let q: String = query.trim().chars().take(100).collect();
    if q.is_empty() {
        return Err(VoiceError::Failed("Nothing to look up.".into()));
    }
    let v = get(&format!(
        "{}?action=query&generator=search&gsrsearch={}&gsrnamespace=0&gsrlimit=1&prop=extracts&exintro=1&explaintext=1\
         &exsentences=3&redirects=1&format=json&formatversion=2",
        urls.wiki,
        encode(&q)
    ))?;
    let page = &v["query"]["pages"][0];
    let (title, extract) = (page["title"].as_str().unwrap_or(""), page["extract"].as_str().unwrap_or(""));
    let low = extract.to_lowercase();
    if extract.trim().is_empty() || low.contains("may refer to") || low.contains("may also refer to") {
        return Err(VoiceError::Failed("No short answer found.".into()));
    }
    Ok(json!({ "source": "Wikipedia", "title": title, "text": speakable(extract) }))
}

// ── The page's calls ──────────────────────────────────────────────────────────

/// `now {day?: 0 | 1}` → `{city, temp, unit, feels, high, low, code, text, rainChance}` for the setting
/// `city` (0 = today, 1 = tomorrow); `Err(Refused)` while the switch is off or the city is empty ·
/// `find {city}` → `[{name, country, lat, lon}]` (the Settings page's city field; needs the switch) ·
/// `lookup {query}` → `{source, title, text}`; `Err(Refused)` while the lookup switch is off.
pub fn call(cx: &Ctx, method: &str, args: Value) -> Result<Value> {
    let s = cx.settings(crate::ID);
    if cx.paused() {
        return Err(VoiceError::Refused("METEOR V1 is paused.".into()));
    }
    let urls = Urls::live();
    match method {
        "now" => {
            let city = gate_weather(&s)?;
            let day = args["day"].as_u64().unwrap_or(0).min(1) as usize;
            now_with(&urls, &city, day, cx.settings("weather")["fahrenheit"] == json!(true))
        }
        "find" => {
            if s["weather"] != json!(true) {
                return Err(VoiceError::Refused("Weather by voice is off.".into()));
            }
            find_with(&urls, args["city"].as_str().unwrap_or(""))
        }
        "lookup" => {
            gate_lookup(&s)?;
            lookup_with(&urls, args["query"].as_str().unwrap_or(""))
        }
        _ => Err(VoiceError::Failed("unknown method".into())),
    }
}
