// Weather — a METEOR V1 feature: current conditions and the next hours from Open-Meteo, for a city the user
// searches by name. No key, no account. Nothing is requested before a city is set. The only thread
// sleeps 30 minutes at a time on a condvar. Temperatures are kept in Celsius; the page converts.

use notch_core::{net, Ctx, Feature, Group, Manifest, Use};
use serde_json::{json, Value};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static MANIFEST: Manifest = Manifest {
    id: "weather",
    name: "Weather",
    group: Group::Tools,
    // Network feature: off until the user turns it on.
    default_on: false,
    uses: &[Use::Network],
    shortcuts: &[],
    tray: &[],
    defaults: r#"{ "place": null, "fahrenheit": false }"#,
};

const TICK: Duration = Duration::from_secs(1800);
const LIMIT: usize = 512 * 1024;

/// The forecast address for a place; both hosts of this feature are Open-Meteo's.
pub fn forecast_url(lat: &Value, lon: &Value) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code,is_day\
         &hourly=temperature_2m,weather_code,is_day&daily=temperature_2m_max,temperature_2m_min&forecast_days=2&timezone=auto"
    )
}

pub fn geocode_url(query: &str) -> String {
    format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=6&language=en&format=json", encode(query))
}

/// Percent-encodes a query value (UTF-8 bytes outside the unreserved set).
pub fn encode(s: &str) -> String {
    s.bytes().fold(String::new(), |mut o, b| {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
        o
    })
}

/// Cities from a geocoding answer: `[{ name, region, country, lat, lon }]`.
pub fn parse_places(body: &str) -> Result<Vec<Value>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    Ok(v["results"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(json!({
                        "name": r["name"].as_str()?, "region": r["admin1"].as_str().unwrap_or(""),
                        "country": r["country"].as_str().unwrap_or(""),
                        "lat": r["latitude"].as_f64()?, "lon": r["longitude"].as_f64()?,
                    }))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The forecast answer as the page wants it: now, today's high and low, the next eight hours.
pub fn parse_forecast(body: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let c = &v["current"];
    let temp = c["temperature_2m"].as_f64().ok_or("no temperature")?;
    let code = c["weather_code"].as_i64().ok_or("no condition")?;
    let now = c["time"].as_str().unwrap_or("");
    // Hourly times and `current.time` are both local to the place: "2026-10-11T14:00" sorts as text.
    let hour_now = format!("{}:00", now.get(..13).unwrap_or(""));
    let h = &v["hourly"];
    let times = h["time"].as_array().ok_or("no hours")?;
    let from = times.iter().position(|t| t.as_str().is_some_and(|t| t > hour_now.as_str())).unwrap_or(times.len());
    let hours: Vec<Value> = (from..times.len().min(from + 8))
        .filter_map(|i| {
            Some(json!({
                "at": times[i].as_str()?.get(11..16)?, "temp": h["temperature_2m"][i].as_f64()?,
                "code": h["weather_code"][i].as_i64()?, "day": h["is_day"][i].as_i64().unwrap_or(1) == 1,
            }))
        })
        .collect();
    Ok(json!({
        "temp": temp, "code": code, "day": c["is_day"].as_i64().unwrap_or(1) == 1,
        "high": v["daily"]["temperature_2m_max"][0].as_f64(), "low": v["daily"]["temperature_2m_min"][0].as_f64(),
        "hours": hours,
    }))
}

#[derive(Default)]
struct Data {
    place: Value,
    forecast: Option<Value>,
    at_ms: u64,
    fetched: Option<Instant>,
    failed: bool,
}

struct Inner {
    cx: Ctx,
    data: Mutex<Data>,
    stop: Mutex<bool>,
    wake: Condvar,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Inner {
    fn place(&self) -> Option<Value> {
        let p = self.cx.settings("weather")["place"].clone();
        (p["lat"].is_f64() && p["lon"].is_f64()).then_some(p)
    }

    fn state(&self) -> Value {
        let d = self.data.lock().unwrap();
        json!({ "place": d.place, "forecast": d.forecast, "at": d.at_ms, "failed": d.failed })
    }

    fn stale(&self) -> bool {
        self.data.lock().unwrap().fetched.is_none_or(|t| t.elapsed() >= TICK)
    }
}

/// One forecast request for the city in the settings. A failure keeps the last data.
async fn refresh(i: Arc<Inner>) {
    let Some(place) = i.place() else {
        *i.data.lock().unwrap() = Data::default();
        i.cx.emit("weather", "changed", i.state());
        return;
    };
    if i.cx.paused() {
        return;
    }
    let result = match net::bounded_get(&forecast_url(&place["lat"], &place["lon"]), Duration::from_secs(15), LIMIT).await {
        Ok(body) => parse_forecast(&String::from_utf8_lossy(&body)),
        Err(e) => Err(e),
    };
    {
        let mut d = i.data.lock().unwrap();
        if d.place != place {
            *d = Data::default(); // another city: the old reading does not belong to it
        }
        d.place = place;
        d.fetched = Some(Instant::now()); // a failure also waits for the next tick
        match result {
            Ok(f) => (d.forecast, d.at_ms, d.failed) = (Some(f), now_ms(), false),
            Err(e) => {
                d.failed = true;
                notch_core::log::line(format!("weather: {e}"));
            }
        }
    }
    i.cx.emit("weather", "changed", i.state());
}

fn spawn_refresh(i: &Arc<Inner>) {
    tauri::async_runtime::spawn(refresh(i.clone()));
}

pub struct Weather {
    inner: Mutex<Option<Arc<Inner>>>,
}

impl Feature for Weather {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        let mut slot = self.inner.lock().unwrap();
        if slot.is_some() {
            return Ok(());
        }
        let i = Arc::new(Inner { cx: cx.clone(), data: Mutex::new(Data::default()), stop: Mutex::new(false), wake: Condvar::new() });
        if i.place().is_some() {
            spawn_refresh(&i);
        }
        let t = i.clone();
        std::thread::spawn(move || loop {
            let (g, _) = t.wake.wait_timeout_while(t.stop.lock().unwrap(), TICK, |s| !*s).unwrap();
            if *g {
                return;
            }
            drop(g);
            if t.place().is_some() {
                tauri::async_runtime::block_on(refresh(t.clone()));
            }
        });
        *slot = Some(i);
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        if let Some(i) = self.inner.lock().unwrap().take() {
            *i.stop.lock().unwrap() = true;
            i.wake.notify_all();
        }
    }

    fn settings_changed(&self, _cx: &Ctx, s: &Value) {
        // A new city (or none) is fetched at once.
        if let Some(i) = self.inner.lock().unwrap().clone() {
            if i.data.lock().unwrap().place != s["place"] {
                spawn_refresh(&i);
            }
        }
    }

    fn call(&self, _cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        let i = self.inner.lock().unwrap().clone().ok_or("turned off")?;
        match method {
            "state" => Ok(i.state()),
            // The section was opened: fetch only when the data is older than 30 minutes.
            "seen" => {
                if i.place().is_some() && i.stale() {
                    spawn_refresh(&i);
                }
                Ok(i.state())
            }
            "search" => {
                let q = args["query"].as_str().unwrap_or("").trim();
                if q.is_empty() || q.len() > 80 {
                    return Err("empty".into());
                }
                let body = tauri::async_runtime::block_on(net::bounded_get(&geocode_url(q), Duration::from_secs(15), LIMIT))?;
                Ok(Value::Array(parse_places(&String::from_utf8_lossy(&body))?))
            }
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/weather/strings.json")
    }
}

pub fn feature() -> Arc<dyn Feature> {
    Arc::new(Weather { inner: Mutex::new(None) })
}
