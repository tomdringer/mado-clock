use std::io::Write;
use chrono::Local;
use serde::Deserialize;

// ── Weather ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct WttrRoot {
    current_condition: Vec<Condition>,
    nearest_area:      Vec<Area>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct Condition {
    temp_C:      String,
    weatherCode: String,
    weatherDesc: Vec<Sv>,
}

#[derive(Deserialize)]
struct Area {
    #[serde(rename = "areaName")]
    area_name: Vec<Sv>,
}

#[derive(Deserialize)]
struct Sv { value: String }

fn icon_for_code(code: &str) -> &'static str {
    match code.parse::<u16>().unwrap_or(0) {
        113                                      => "☀",
        116                                      => "⛅",
        119 | 122                                => "☁",
        143 | 248 | 260                          => "🌫",
        176 | 263 | 266 | 293..=308              => "🌧",
        179 | 227 | 230 | 323..=338 | 371 | 395 => "❄",
        182 | 185 | 281 | 284 | 311..=320        => "🌨",
        200 | 386..=395                          => "⛈",
        _                                        => "🌡",
    }
}

struct WeatherData {
    icon: &'static str,
    temp: String,
    desc: String,
    loc:  String,
}

fn fetch_weather() -> Option<WeatherData> {
    let out = std::process::Command::new("curl")
        .args(["-s", "--max-time", "10", "https://wttr.in/?format=j1"])
        .output()
        .ok()?;

    if !out.status.success() { return None; }

    let root: WttrRoot = serde_json::from_slice(&out.stdout).ok()?;
    let cond = root.current_condition.into_iter().next()?;
    let area = root.nearest_area.into_iter().next()?;

    Some(WeatherData {
        icon: icon_for_code(&cond.weatherCode),
        temp: format!("{}°C", cond.temp_C),
        desc: cond.weatherDesc.into_iter().next().map(|s| s.value).unwrap_or_default(),
        loc:  area.area_name.into_iter().next().map(|s| s.value).unwrap_or_default(),
    })
}

// ── Render ────────────────────────────────────────────────────────────────────

fn render(weather: &Option<WeatherData>) {
    let now  = Local::now();
    let time = now.format("%H:%M").to_string();
    let date = now.format("%A, %-d %B").to_string();

    // Slate palette: time bright, date/weather muted
    let bright = "\x1b[1;38;2;248;250;252m";  // slate-50 bold
    let mid    = "\x1b[38;2;148;163;184m";     // slate-400
    let dim    = "\x1b[38;2;71;85;105m";       // slate-600
    let reset  = "\x1b[0m";

    print!("\x1b[H\x1b[2J");  // clear + home
    println!();
    println!("  {bright}{time}{reset}");
    println!("  {dim}{date}{reset}");

    if let Some(w) = weather {
        println!();
        println!("  {mid}{} {}  {}{reset}", w.icon, w.temp, w.desc);
        println!("  {dim}{}{reset}", w.loc);
    }
}

// ── Main loop ─────────────────────────────────────────────────────────────────

fn main() {
    // Fetch weather once at startup, then refresh every 10 minutes.
    let mut weather = fetch_weather();
    let mut ticks_since_fetch: u32 = 0;

    loop {
        render(&weather);
        std::io::stdout().flush().ok();
        std::thread::sleep(std::time::Duration::from_secs(1));

        ticks_since_fetch += 1;
        if ticks_since_fetch >= 600 {
            weather = fetch_weather();
            ticks_since_fetch = 0;
        }
    }
}
