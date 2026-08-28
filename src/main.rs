// mado-clock — pixel plugin for Mado sidebar

use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Local;
use serde::Deserialize;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClockConfig {
    /// "12h" or "24h". Default: "24h"
    time_format: String,
    /// Weather location, e.g. "London". Empty = auto-detect from IP.
    location:    String,
    /// strftime date format. Default: "%A, %-d %B"
    date_format: String,
}

impl ClockConfig {
    fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let path = std::path::Path::new(&home)
            .join(".config/mado/plugins/clock.toml");
        let content = std::fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_default()
    }

    fn time_fmt(&self) -> &str {
        if self.time_format == "12h" { "%I:%M %p" } else { "%H:%M" }
    }

    fn date_fmt(&self) -> &str {
        if self.date_format.is_empty() { "%A, %-d %B" } else { &self.date_format }
    }

    fn weather_url(&self) -> String {
        if self.location.is_empty() {
            "https://wttr.in/?format=j1".into()
        } else {
            format!("https://wttr.in/{}?format=j1", self.location)
        }
    }
}

// ── Palette (Slate) ───────────────────────────────────────────────────────────

const BG:   [u8; 4] = [15,  23,  42,  255]; // slate-900
const TIME: [u8; 4] = [248, 250, 252, 255]; // slate-50
const DATE: [u8; 4] = [148, 163, 184, 255]; // slate-400
const DIM:  [u8; 4] = [71,  85,  105, 255]; // slate-600

// ── Font loading ──────────────────────────────────────────────────────────────

fn load_font(candidates: &[&str]) -> Option<fontdue::Font> {
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(font) = fontdue::Font::from_bytes(
                data.as_slice(), fontdue::FontSettings::default()) {
                return Some(font);
            }
        }
    }
    None
}

fn load_system_font() -> Option<fontdue::Font> {
    load_font(&[
        "/System/Library/Fonts/Helvetica.ttc",
        "/Library/Fonts/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
    ])
}

/// Font with Unicode Miscellaneous Symbols (U+2600–U+26FF) for weather icons.
fn load_symbol_font() -> Option<fontdue::Font> {
    load_font(&[
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/System/Library/Fonts/Supplemental/Symbol.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
    ])
}

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
}

#[derive(Deserialize)]
struct Area {
    #[serde(rename = "areaName")]
    area_name: Vec<Sv>,
}

#[derive(Deserialize)]
struct Sv { value: String }

fn weather_icon(code: &str) -> &'static str {
    match code.parse::<u16>().unwrap_or(0) {
        113                                      => "☀",   // Sunny
        116                                      => "⛅",  // Partly cloudy
        119 | 122                                => "☁",   // Cloudy
        143 | 248 | 260                          => "≋",   // Foggy
        176 | 263 | 266 | 293..=308              => "☔",  // Rainy
        179 | 227 | 230 | 323..=338 | 371 | 395 => "❄",   // Snowy
        182 | 185 | 281 | 284 | 311..=320        => "❆",   // Sleet
        200 | 386..=395                          => "☈",   // Stormy
        _                                        => "~",
    }
}

#[derive(Clone)]
struct WeatherData {
    icon: &'static str,
    temp: String,
    loc:  String,
}

fn fetch_weather(url: &str) -> Option<WeatherData> {
    let out = std::process::Command::new("curl")
        .args(["-s", "--max-time", "10", url])
        .output().ok()?;
    if !out.status.success() { return None; }
    let root: WttrRoot = serde_json::from_slice(&out.stdout).ok()?;
    let cond = root.current_condition.into_iter().next()?;
    let area = root.nearest_area.into_iter().next()?;
    Some(WeatherData {
        icon: weather_icon(&cond.weatherCode),
        temp: format!("{}°C", cond.temp_C),
        loc:  area.area_name.into_iter().next().map(|s| s.value).unwrap_or_default(),
    })
}

// ── Canvas ────────────────────────────────────────────────────────────────────

struct Canvas { pixels: Vec<u8>, w: usize, h: usize }

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        let mut pixels = vec![0u8; w * h * 4];
        for px in pixels.chunks_exact_mut(4) { px.copy_from_slice(&BG); }
        Canvas { pixels, w, h }
    }

    fn blend(&mut self, x: usize, y: usize, color: [u8; 4], alpha: f32) {
        if x >= self.w || y >= self.h { return; }
        let i = (y * self.w + x) * 4;
        let ia = 1.0 - alpha;
        for c in 0..3 {
            self.pixels[i + c] =
                (self.pixels[i + c] as f32 * ia + color[c] as f32 * alpha).round() as u8;
        }
        self.pixels[i + 3] = 255;
    }

    fn text(&mut self, font: &fontdue::Font, text: &str,
            size: f32, x: usize, y: usize, color: [u8; 4]) -> usize {
        let mut cx = x;
        for ch in text.chars() {
            let (m, bmp) = font.rasterize(ch, size);
            let gx = cx as isize + m.xmin as isize;
            let gy = y as isize - m.height as isize - m.ymin as isize;
            for (k, &cov) in bmp.iter().enumerate() {
                if cov == 0 { continue; }
                let px = gx + (k % m.width) as isize;
                let py = gy + (k / m.width) as isize;
                if px >= 0 && py >= 0 {
                    self.blend(px as usize, py as usize, color, cov as f32 / 255.0);
                }
            }
            cx += m.advance_width.round() as usize;
        }
        cx
    }

    fn write_frame(&self, out: &mut impl Write) {
        out.write_all(b"MADO").unwrap();
        out.write_all(&(self.w as u32).to_le_bytes()).unwrap();
        out.write_all(&(self.h as u32).to_le_bytes()).unwrap();
        out.write_all(&self.pixels).unwrap();
        out.flush().unwrap();
    }
}

// ── Resize event ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind:   String,
    width:  Option<u32>,
    height: Option<u32>,
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let cfg = ClockConfig::load();

    let font = load_system_font().unwrap_or_else(|| {
        eprintln!("mado-clock: no system font found");
        std::process::exit(1);
    });
    let sym_font = load_symbol_font();

    let dims:    Arc<Mutex<(u32, u32)>>          = Arc::new(Mutex::new((300, 400)));
    let weather: Arc<Mutex<Option<WeatherData>>> = Arc::new(Mutex::new(None));

    // Weather refresh thread
    {
        let weather = Arc::clone(&weather);
        let url = cfg.weather_url();
        std::thread::spawn(move || loop {
            *weather.lock().unwrap() = fetch_weather(&url);
            std::thread::sleep(Duration::from_secs(600));
        });
    }

    // Stdin event listener
    {
        let dims = Arc::clone(&dims);
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in BufReader::new(stdin.lock()).lines().flatten() {
                if let Ok(ev) = serde_json::from_str::<Event>(&line) {
                    if ev.kind == "resize" {
                        if let (Some(w), Some(h)) = (ev.width, ev.height) {
                            if w > 0 && h > 0 { *dims.lock().unwrap() = (w, h); }
                        }
                    }
                }
            }
        });
    }

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    loop {
        let (w, h) = *dims.lock().unwrap();
        let (w, h) = (w as usize, h as usize);

        let now  = Local::now();
        let time = now.format(cfg.time_fmt()).to_string();
        let date = now.format(cfg.date_fmt()).to_string();

        let pad       = (w as f32 * 0.08).max(8.0) as usize;
        let time_size = (w as f32 * 0.18).clamp(22.0, 54.0);
        let sub_size  = (w as f32 * 0.085).clamp(10.0, 18.0);
        let icon_size = (w as f32 * 0.12).clamp(14.0, 24.0);

        let mut canvas = Canvas::new(w, h);

        let time_y = (h as f32 * 0.32) as usize;
        canvas.text(&font, &time, time_size, pad, time_y, TIME);

        let date_y = time_y + (time_size * 1.35) as usize;
        canvas.text(&font, &date, sub_size, pad, date_y, DATE);

        if let Some(ref wx) = *weather.lock().unwrap() {
            let wx_y  = date_y + (sub_size * 2.4) as usize;
            let loc_y = wx_y   + (sub_size * 1.6) as usize;

            // Icon + temp on same line
            let after_icon = if let Some(ref sf) = sym_font {
                canvas.text(sf, wx.icon, icon_size, pad, wx_y, DATE)
            } else {
                pad // no symbol font — skip icon, show temp from left margin
            };
            canvas.text(&font, &wx.temp, sub_size, after_icon + 6, wx_y, DATE);
            canvas.text(&font, &wx.loc,  sub_size, pad, loc_y, DIM);
        }

        canvas.write_frame(&mut out);
        std::thread::sleep(Duration::from_secs(1));
    }
}
