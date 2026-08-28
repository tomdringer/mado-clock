// mado-clock — pixel plugin for Mado sidebar
//
// Reads resize/input events from stdin (newline-delimited JSON), renders a
// clock + weather panel at the requested physical pixel dimensions, and writes
// RGBA frames to stdout using the Mado pixel plugin protocol:
//
//   [4 B magic "MADO"] [u32 LE width] [u32 LE height] [w*h*4 B RGBA8]

use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Local;
use serde::Deserialize;

// ── Palette (Slate) ───────────────────────────────────────────────────────────

const BG:   [u8; 4] = [15,  23,  42,  255]; // slate-900
const TIME: [u8; 4] = [248, 250, 252, 255]; // slate-50
const DATE: [u8; 4] = [148, 163, 184, 255]; // slate-400
const DIM:  [u8; 4] = [71,  85,  105, 255]; // slate-600

// ── Font loading ──────────────────────────────────────────────────────────────

fn load_system_font() -> Option<fontdue::Font> {
    let candidates: &[&str] = &[
        // macOS
        "/System/Library/Fonts/Helvetica.ttc",
        "/Library/Fonts/Arial.ttf",
        // Linux
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        // Windows
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
    ];
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(font) = fontdue::Font::from_bytes(
                data.as_slice(),
                fontdue::FontSettings::default(),
            ) {
                return Some(font);
            }
        }
    }
    None
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
    weatherDesc: Vec<Sv>,
}

#[derive(Deserialize)]
struct Area {
    #[serde(rename = "areaName")]
    area_name: Vec<Sv>,
}

#[derive(Deserialize)]
struct Sv { value: String }

fn weather_label(code: &str) -> &'static str {
    match code.parse::<u16>().unwrap_or(0) {
        113                                      => "Sunny",
        116                                      => "Partly Cloudy",
        119 | 122                                => "Cloudy",
        143 | 248 | 260                          => "Foggy",
        176 | 263 | 266 | 293..=308              => "Rainy",
        179 | 227 | 230 | 323..=338 | 371 | 395 => "Snowy",
        182 | 185 | 281 | 284 | 311..=320        => "Sleet",
        200 | 386..=395                          => "Stormy",
        _                                        => "",
    }
}

#[derive(Clone)]
struct WeatherData {
    condition: &'static str,
    temp:      String,
    loc:       String,
}

fn fetch_weather() -> Option<WeatherData> {
    let out = std::process::Command::new("curl")
        .args(["-s", "--max-time", "10", "https://wttr.in/?format=j1"])
        .output().ok()?;
    if !out.status.success() { return None; }
    let root: WttrRoot = serde_json::from_slice(&out.stdout).ok()?;
    let cond = root.current_condition.into_iter().next()?;
    let area = root.nearest_area.into_iter().next()?;
    Some(WeatherData {
        condition: weather_label(&cond.weatherCode),
        temp: format!("{}°C", cond.temp_C),
        loc:  area.area_name.into_iter().next().map(|s| s.value).unwrap_or_default(),
    })
}

// ── Canvas ────────────────────────────────────────────────────────────────────

struct Canvas {
    pixels: Vec<u8>,
    w:      usize,
    h:      usize,
}

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

    /// Draw `text` at (`x`, baseline `y`) using `font` at `size` px; returns
    /// the x position after the last glyph.
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
    let font = match load_system_font() {
        Some(f) => f,
        None => {
            eprintln!("mado-clock: no system font found — cannot render");
            std::process::exit(1);
        }
    };

    let dims:    Arc<Mutex<(u32, u32)>>            = Arc::new(Mutex::new((300, 400)));
    let weather: Arc<Mutex<Option<WeatherData>>>   = Arc::new(Mutex::new(None));

    // Weather background thread — refreshes every 10 minutes
    {
        let weather = Arc::clone(&weather);
        std::thread::spawn(move || loop {
            *weather.lock().unwrap() = fetch_weather();
            std::thread::sleep(Duration::from_secs(600));
        });
    }

    // Stdin event listener — updates dims on resize
    {
        let dims = Arc::clone(&dims);
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in BufReader::new(stdin.lock()).lines().flatten() {
                if let Ok(ev) = serde_json::from_str::<Event>(&line) {
                    if ev.kind == "resize" {
                        if let (Some(w), Some(h)) = (ev.width, ev.height) {
                            if w > 0 && h > 0 {
                                *dims.lock().unwrap() = (w, h);
                            }
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
        let time = now.format("%H:%M").to_string();
        let date = now.format("%A, %-d %B").to_string();

        let pad       = (w as f32 * 0.08).max(8.0) as usize;
        let time_size = (w as f32 * 0.18).clamp(22.0, 54.0);
        let sub_size  = (w as f32 * 0.085).clamp(10.0, 18.0);

        let mut canvas = Canvas::new(w, h);

        // Time
        let time_y = (h as f32 * 0.32) as usize;
        canvas.text(&font, &time, time_size, pad, time_y, TIME);

        // Date
        let date_y = time_y + (time_size * 1.35) as usize;
        canvas.text(&font, &date, sub_size, pad, date_y, DATE);

        // Weather
        if let Some(ref wx) = *weather.lock().unwrap() {
            let wx_y   = date_y + (sub_size * 2.4) as usize;
            let loc_y  = wx_y   + (sub_size * 1.6) as usize;
            let wx_str = format!("{}  {}", wx.condition, wx.temp);
            canvas.text(&font, &wx_str, sub_size, pad, wx_y,  DATE);
            canvas.text(&font, &wx.loc, sub_size, pad, loc_y, DIM);
        }

        canvas.write_frame(&mut out);
        std::thread::sleep(Duration::from_secs(1));
    }
}
