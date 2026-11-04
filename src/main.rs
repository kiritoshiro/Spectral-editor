// Speech Fixer: find the speech, even out the speakers, tame the peaks,
// set the loudness. See MECHANICS.md for where every number comes from.

mod audio_io;
mod dsp;
mod player;

use dsp::{Audio, Features, Processed, Progress, Settings, Stats};
use macroquad::prelude::*;
use player::Player;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

const W: f32 = 1280.0;
const H: f32 = 760.0;
const LANE_X: f32 = 14.0;
const LANE_W: f32 = 880.0;
const LANE_H: f32 = 205.0;
const LANE1_Y: f32 = 78.0;
const LANE2_Y: f32 = 318.0;
const PANEL_X: f32 = 912.0;

const BG: Color = Color::new(0.110, 0.122, 0.141, 1.0);
const PANEL: Color = Color::new(0.161, 0.176, 0.204, 1.0);
const CARD: Color = Color::new(0.192, 0.208, 0.239, 1.0);
const LINE: Color = Color::new(0.270, 0.290, 0.330, 1.0);
const TEXT: Color = Color::new(0.86, 0.88, 0.91, 1.0);
const DIM: Color = Color::new(0.56, 0.60, 0.66, 1.0);
const WAVE_IN: Color = Color::new(0.36, 0.62, 0.86, 1.0);
const WAVE_OUT: Color = Color::new(0.30, 0.80, 0.68, 1.0);
const ACCENT: Color = Color::new(0.98, 0.62, 0.22, 1.0);
const SPEECH: Color = Color::new(0.44, 0.86, 0.42, 1.0);

fn window_conf() -> Conf {
    Conf {
        window_title: "Speech Fixer".into(),
        window_width: W as i32,
        window_height: H as i32,
        window_resizable: false,
        ..Default::default()
    }
}

/// Where the demo really has speech, and when its clap is.
type Truth = (Vec<(f32, f32)>, f32);

struct JobDone {
    audio: Arc<Audio>,
    name: String,
    truth: Option<Truth>,
    features: Arc<Features>,
    result: Processed,
    settings: Settings,
}

enum Source {
    File(PathBuf),
    Demo,
    Ready(Arc<Audio>, String, Option<Truth>, Option<Arc<Features>>),
}

fn run_job(
    src: Source,
    s: &Settings,
    auto: bool,
    locked: &[String],
    prog: &Progress,
) -> Result<JobDone, String> {
    prog.set(0.0);
    let (audio, name, truth, feats) = match src {
        Source::File(p) => {
            let a = audio_io::load(&p)?;
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            (Arc::new(a), name, None, None)
        }
        Source::Demo => {
            let (a, t, clap) = audio_io::demo();
            (
                Arc::new(a),
                "demo recording".to_string(),
                Some((t, clap)),
                None,
            )
        }
        Source::Ready(a, n, t, f) => (a, n, t, f),
    };
    let features = match feats {
        Some(f) => f,
        None => Arc::new(dsp::analyse(&audio, prog, 0.05, 0.6)),
    };
    let (settings, result) = if auto {
        dsp::auto_fix(&audio, &features, s, locked, prog)
    } else {
        (s.clone(), dsp::process(&audio, &features, s, prog))
    };
    Ok(JobDone {
        audio,
        name,
        truth,
        features,
        result,
        settings,
    })
}

struct Job {
    rx: mpsc::Receiver<Result<JobDone, String>>,
    prog: Arc<Progress>,
}

#[derive(Default)]
struct Mouse {
    x: f32,
    y: f32,
    down: bool,
    pressed: bool,
    released: bool,
}

struct App {
    audio: Option<Arc<Audio>>,
    name: String,
    truth: Option<Truth>,
    features: Option<Arc<Features>>,
    result: Option<Arc<Processed>>,
    settings: Settings,
    job: Option<Job>,
    dirty: bool,
    wave_in: Vec<(f32, f32)>,
    wave_out: Vec<(f32, f32)>,
    player: Player,
    listen_fixed: bool,
    status: String,
    m: Mouse,
    drag: Option<u32>,
    hud: Vec<String>,
}

fn peaks(a: &Audio, cols: usize) -> Vec<(f32, f32)> {
    let n = a.len();
    (0..cols)
        .map(|c| {
            let s = c * n / cols;
            let e = ((c + 1) * n / cols).max(s + 1).min(n);
            let (mut lo, mut hi) = (0f32, 0f32);
            for i in s..e {
                let v = a.ch.iter().map(|ch| ch[i]).sum::<f32>() / a.ch.len() as f32;
                lo = lo.min(v);
                hi = hi.max(v);
            }
            (lo, hi)
        })
        .collect()
}

fn fmt_time(t: f32) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0) as u32, t % 60.0)
}

impl App {
    fn new() -> Self {
        App {
            audio: None,
            name: String::new(),
            truth: None,
            features: None,
            result: None,
            settings: Settings::default(),
            job: None,
            dirty: false,
            wave_in: vec![],
            wave_out: vec![],
            player: Player::new(),
            listen_fixed: true,
            status: "Open a recording, or try the demo.".into(),
            m: Mouse::default(),
            drag: None,
            hud: vec![],
        }
    }

    fn start(&mut self, src: Source) {
        self.start_with(src, false);
    }

    fn start_with(&mut self, src: Source, auto: bool) {
        let (tx, rx) = mpsc::channel();
        let prog = Arc::new(Progress::new());
        let p = prog.clone();
        let s = self.settings.clone();
        std::thread::spawn(move || {
            let r = run_job(src, &s, auto, &[], &p);
            let _ = tx.send(r);
        });
        self.job = Some(Job { rx, prog });
        self.dirty = false;
    }

    fn auto_fix(&mut self) {
        if let Some(a) = self.audio.clone() {
            self.start_with(
                Source::Ready(
                    a,
                    self.name.clone(),
                    self.truth.clone(),
                    self.features.clone(),
                ),
                true,
            );
            self.status = "Auto fix: trying settings...".into();
        }
    }

    fn reprocess(&mut self) {
        if let Some(a) = self.audio.clone() {
            self.start(Source::Ready(
                a,
                self.name.clone(),
                self.truth.clone(),
                self.features.clone(),
            ));
        }
    }

    fn accept(&mut self, d: JobDone) {
        let new_file = self
            .audio
            .as_ref()
            .map_or(true, |a| !Arc::ptr_eq(a, &d.audio));
        self.wave_out = peaks(&d.result.out, LANE_W as usize);
        self.player
            .set_sources(Some(d.audio.clone()), Some(d.result.out.clone()));
        if new_file {
            self.wave_in = peaks(&d.audio, LANE_W as usize);
            self.player.stop();
            self.player.seek(0.0);
        }
        // keep slider edits made while this result was being worked out
        if !self.dirty {
            self.settings = d.settings.clone();
        }
        self.status = format!(
            "{}: speech evened from {:.1} dB apart to {:.1} dB, loudness {:.1} LUFS.",
            d.name, d.result.before.spread, d.result.after.spread, d.result.after.integrated
        );
        if let Some(e) = &self.player.error {
            self.status = format!("{e} - listening is off. {}", self.status);
        }
        self.audio = Some(d.audio);
        self.name = d.name;
        self.truth = d.truth;
        self.features = Some(d.features);
        self.result = Some(Arc::new(d.result));
    }

    fn run_blocking(&mut self, src: Source) {
        let prog = Progress::new();
        match run_job(src, &self.settings, false, &[], &prog) {
            Ok(d) => self.accept(d),
            Err(e) => self.status = e,
        }
    }

    fn poll(&mut self) {
        if let Some(j) = &self.job {
            if let Ok(r) = j.rx.try_recv() {
                self.job = None;
                match r {
                    Ok(d) => self.accept(d),
                    Err(e) => self.status = e,
                }
            }
        }
        if self.dirty && self.job.is_none() && self.drag.is_none() {
            self.reprocess();
        }
    }

    fn duration(&self) -> f32 {
        self.audio.as_ref().map_or(0.0, |a| a.secs())
    }

    fn position(&self) -> f32 {
        self.player.position() as f32
    }

    fn seek(&mut self, t: f32) {
        self.player.seek(t as f64);
    }

    fn toggle_play(&mut self) {
        if self.player.is_playing() {
            self.player.stop();
        } else {
            self.player
                .set_fixed(self.listen_fixed && self.result.is_some());
            self.player.play();
        }
    }

    fn listen(&mut self, fixed: bool) {
        self.listen_fixed = fixed;
        self.player.set_fixed(fixed);
    }

    fn open_dialog(&mut self) {
        let pick = rfd::FileDialog::new()
            .add_filter(
                "Sound",
                &[
                    "wav", "mp3", "flac", "m4a", "aac", "mp4", "ogg", "aif", "aiff", "caf",
                ],
            )
            .pick_file();
        if let Some(p) = pick {
            self.status = format!("Reading {}...", p.display());
            self.features = None;
            self.start(Source::File(p));
        }
    }

    fn save_dialog(&mut self) {
        let Some(r) = self.result.clone() else { return };
        let pick = rfd::FileDialog::new()
            .add_filter("WAV sound", &["wav"])
            .set_file_name(format!("{} (fixed).wav", self.name))
            .save_file();
        if let Some(p) = pick {
            self.status = match audio_io::save(&p, &r.out) {
                Ok(()) => format!("Saved {}", p.display()),
                Err(e) => format!("Couldn't save: {e}"),
            };
        }
    }

    // ------------------------------------------------------------ widgets

    fn button(&mut self, x: f32, y: f32, w: f32, label: &str, on: bool, enabled: bool) -> bool {
        let h = 28.0;
        let hover =
            enabled && self.m.x >= x && self.m.x <= x + w && self.m.y >= y && self.m.y <= y + h;
        let fill = if on {
            Color::new(0.24, 0.42, 0.62, 1.0)
        } else if hover {
            Color::new(0.27, 0.29, 0.34, 1.0)
        } else {
            CARD
        };
        draw_rectangle(x, y, w, h, fill);
        draw_rectangle_lines(x, y, w, h, 1.0, LINE);
        let d = measure_text(label, None, 18, 1.0);
        draw_text(
            label,
            x + (w - d.width) / 2.0,
            y + 19.0,
            18.0,
            if enabled { TEXT } else { DIM },
        );
        hover && self.m.pressed
    }

    fn toggle(&mut self, x: f32, y: f32, label: &str, v: &mut bool) -> bool {
        let hover =
            self.m.x >= x && self.m.x <= x + 300.0 && self.m.y >= y - 2.0 && self.m.y <= y + 18.0;
        draw_rectangle(x, y, 16.0, 16.0, if *v { ACCENT } else { CARD });
        draw_rectangle_lines(x, y, 16.0, 16.0, 1.0, LINE);
        if *v {
            draw_line(x + 3.0, y + 8.0, x + 7.0, y + 12.0, 2.0, BG);
            draw_line(x + 7.0, y + 12.0, x + 13.0, y + 4.0, 2.0, BG);
        }
        draw_text(label, x + 24.0, y + 14.0, 20.0, TEXT);
        if hover && self.m.pressed {
            *v = !*v;
            return true;
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn slider(
        &mut self,
        id: u32,
        x: f32,
        y: f32,
        label: &str,
        v: &mut f32,
        lo: f32,
        hi: f32,
        step: f32,
        unit: &str,
        enabled: bool,
    ) {
        let w = 340.0;
        let col = if enabled { TEXT } else { DIM };
        draw_text(label, x, y + 12.0, 17.0, col);
        let val = if step >= 1.0 {
            format!("{:.0} {unit}", v)
        } else {
            format!("{:.1} {unit}", v)
        };
        let d = measure_text(&val, None, 17, 1.0);
        draw_text(&val, x + w - d.width, y + 12.0, 17.0, col);
        let ty = y + 20.0;
        draw_rectangle(x, ty, w, 5.0, LINE);
        let t = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
        draw_rectangle(x, ty, w * t, 5.0, if enabled { ACCENT } else { DIM });
        draw_circle(x + w * t, ty + 2.5, 7.0, if enabled { TEXT } else { DIM });
        let hover = self.m.x >= x - 6.0
            && self.m.x <= x + w + 6.0
            && self.m.y >= y
            && self.m.y <= ty + 14.0;
        if hover && self.m.pressed && enabled {
            self.drag = Some(id);
        }
        if self.drag == Some(id) && self.m.down {
            let t = ((self.m.x - x) / w).clamp(0.0, 1.0);
            let nv = ((lo + t * (hi - lo)) / step).round() * step;
            *v = nv.clamp(lo, hi);
        }
    }

    // ------------------------------------------------------------ drawing

    fn draw_lane(&mut self, y: f32, title: &str, fixed: bool) {
        draw_rectangle(
            LANE_X,
            y,
            LANE_W,
            LANE_H,
            Color::new(0.08, 0.09, 0.105, 1.0),
        );
        draw_rectangle_lines(LANE_X, y, LANE_W, LANE_H, 1.0, LINE);
        let mid = y + LANE_H / 2.0;
        for db in [-6.0f32, -12.0, -24.0] {
            let a = 10f32.powf(db / 20.0) * (LANE_H / 2.0 - 8.0);
            draw_line(
                LANE_X,
                mid - a,
                LANE_X + LANE_W,
                mid - a,
                1.0,
                Color::new(0.16, 0.18, 0.21, 1.0),
            );
            draw_line(
                LANE_X,
                mid + a,
                LANE_X + LANE_W,
                mid + a,
                1.0,
                Color::new(0.16, 0.18, 0.21, 1.0),
            );
        }
        draw_line(LANE_X, mid, LANE_X + LANE_W, mid, 1.0, LINE);
        let wave = if fixed { &self.wave_out } else { &self.wave_in };
        let col = if fixed { WAVE_OUT } else { WAVE_IN };
        let amp = LANE_H / 2.0 - 8.0;
        for (i, (lo, hi)) in wave.iter().enumerate() {
            let x = LANE_X + i as f32 + 0.5;
            draw_line(x, mid - hi * amp, x, mid - lo * amp + 1.0, 1.0, col);
        }
        if let Some(r) = self.result.clone() {
            // speech strip along the bottom
            let n = r.speech.region.len();
            for c in 0..LANE_W as usize {
                let s = c * n / LANE_W as usize;
                let e = ((c + 1) * n / LANE_W as usize).max(s + 1).min(n);
                let f =
                    r.speech.region[s..e].iter().filter(|&&x| x).count() as f32 / (e - s) as f32;
                if f > 0.5 {
                    draw_rectangle(LANE_X + c as f32, y + LANE_H - 6.0, 1.0, 5.0, SPEECH);
                }
            }
            if !fixed {
                // volume change curve
                let gy = |g: f32| mid - (g / 24.0).clamp(-1.0, 1.0) * (LANE_H / 2.0 - 10.0);
                let n = r.gain_db.len();
                let mut prev: Option<(f32, f32)> = None;
                for c in (0..LANE_W as usize).step_by(2) {
                    let i = (c * n / LANE_W as usize).min(n - 1);
                    let p = (LANE_X + c as f32, gy(r.gain_db[i]));
                    if let Some(q) = prev {
                        draw_line(q.0, q.1, p.0, p.1, 2.0, ACCENT);
                    }
                    prev = Some(p);
                }
            }
            let secs = self.duration().max(0.01);
            for s in &r.segments {
                let x = LANE_X + (s.a as f32 * dsp::HOP_S / secs) * LANE_W;
                let wpx = ((s.b - s.a) as f32 * dsp::HOP_S / secs) * LANE_W;
                if wpx > 54.0 {
                    let v = if fixed { s.after } else { s.before };
                    let t = format!("{:.0} LUFS", v);
                    draw_rectangle(
                        x + 2.0,
                        y + 4.0,
                        measure_text(&t, None, 15, 1.0).width + 8.0,
                        17.0,
                        Color::new(0.0, 0.0, 0.0, 0.55),
                    );
                    draw_text(
                        &t,
                        x + 6.0,
                        y + 17.0,
                        15.0,
                        if fixed { WAVE_OUT } else { WAVE_IN },
                    );
                }
            }
        }
        let t = self.duration();
        if t > 0.0 {
            let px = LANE_X + (self.position() / t).clamp(0.0, 1.0) * LANE_W;
            draw_line(px, y, px, y + LANE_H, 1.5, WHITE);
        }
        draw_text(
            title,
            LANE_X,
            y - 7.0,
            20.0,
            if fixed { WAVE_OUT } else { WAVE_IN },
        );
        if !fixed && self.result.is_some() {
            draw_text(
                "orange line = volume change   green strip = speech found",
                LANE_X + 480.0,
                y - 7.0,
                16.0,
                DIM,
            );
        }
        // click to place the playhead
        let inside = self.m.x >= LANE_X
            && self.m.x <= LANE_X + LANE_W
            && self.m.y >= y
            && self.m.y <= y + LANE_H;
        if inside && self.m.pressed && self.drag.is_none() && t > 0.0 {
            self.seek((self.m.x - LANE_X) / LANE_W * t);
        }
    }

    fn draw_meters(&mut self, y: f32) {
        draw_rectangle(LANE_X, y, LANE_W, H - y - 10.0, PANEL);
        draw_text("Measurements", LANE_X + 12.0, y + 24.0, 20.0, TEXT);
        let cols = [LANE_X + 12.0, LANE_X + 470.0, LANE_X + 650.0];
        draw_text("Original", cols[1], y + 24.0, 18.0, WAVE_IN);
        draw_text("Fixed", cols[2], y + 24.0, 18.0, WAVE_OUT);
        let empty = Stats::default();
        let (b, a) = match &self.result {
            Some(r) => (r.before.clone(), r.after.clone()),
            None => (empty.clone(), empty),
        };
        let has = self.result.is_some();
        let rows: [(&str, String, String); 5] = [
            (
                "Loudness of the whole track",
                format!("{:.1} LUFS", b.integrated),
                format!("{:.1} LUFS", a.integrated),
            ),
            (
                "Loudness of the speech",
                format!("{:.1} LUFS", b.speech),
                format!("{:.1} LUFS", a.speech),
            ),
            (
                "Highest peak",
                format!("{:.1} dB", b.true_peak),
                format!("{:.1} dB", a.true_peak),
            ),
            (
                "Difference between quiet and loud parts",
                format!("{:.1} dB", b.spread),
                format!("{:.1} dB", a.spread),
            ),
            (
                "Share of the track that is speech",
                format!("{:.0} %", b.speech_pct),
                format!("{:.0} %", a.speech_pct),
            ),
        ];
        self.hud.clear();
        for (i, (label, vb, va)) in rows.iter().enumerate() {
            let ry = y + 52.0 + i as f32 * 24.0;
            draw_text(label, cols[0], ry, 18.0, DIM);
            let (vb, va) = if has {
                (vb.clone(), va.clone())
            } else {
                ("–".into(), "–".into())
            };
            draw_text(&vb, cols[1], ry, 18.0, TEXT);
            draw_text(&va, cols[2], ry, 18.0, TEXT);
            self.hud.push(format!("{label}: {vb} -> {va}"));
        }
    }

    fn card(&self, y: f32, h: f32, title: &str, sub: &str) {
        draw_rectangle(PANEL_X + 8.0, y, W - PANEL_X - 16.0, h, CARD);
        draw_text(title, PANEL_X + 40.0, y + 22.0, 20.0, TEXT);
        draw_text(sub, PANEL_X + 18.0, y + 42.0, 15.0, DIM);
    }

    fn draw_panel(&mut self) {
        draw_rectangle(PANEL_X, 52.0, W - PANEL_X, H - 52.0, PANEL);
        let before = self.settings.clone();
        let mut s = self.settings.clone();
        let x = PANEL_X + 18.0;

        let mut y = 62.0;
        self.card(
            y,
            82.0,
            "1  Find the speech",
            "Music, noise and claps are not measured",
        );
        self.slider(
            1,
            x,
            y + 50.0,
            "Sensitivity",
            &mut s.sensitivity,
            0.0,
            10.0,
            0.1,
            "",
            true,
        );

        y += 92.0;
        self.card(y, 206.0, "", "Every speaker and part at the same level");
        self.toggle(x, y + 8.0, "2  Even out the speakers", &mut s.level_on);
        let on = s.level_on;
        self.slider(
            2,
            x,
            y + 50.0,
            "Target level",
            &mut s.target_level,
            -40.0,
            0.0,
            1.0,
            "dB",
            on,
        );
        self.slider(
            3,
            x,
            y + 80.0,
            "Responsiveness",
            &mut s.responsiveness,
            dsp::RESP_MIN,
            dsp::RESP_MAX,
            0.1,
            "",
            on,
        );
        let keep = format!(
            "Keep natural ups and downs (max +/-{:.0} dB)",
            s.max_gain_db()
        );
        self.slider(
            4,
            x,
            y + 110.0,
            &keep,
            &mut s.preserve,
            0.0,
            100.0,
            1.0,
            "%",
            on,
        );
        draw_text(
            "Optimize for:",
            x,
            y + 156.0,
            17.0,
            if on { TEXT } else { DIM },
        );
        if self.button(x + 120.0, y + 140.0, 100.0, "Dialog", on && s.dialog, on) {
            s.dialog = true;
        }
        if self.button(x + 226.0, y + 140.0, 100.0, "Music", on && !s.dialog, on) {
            s.dialog = false;
        }
        self.slider(
            6,
            x,
            y + 170.0,
            "Lower music and noise",
            &mut s.background_db,
            -20.0,
            0.0,
            1.0,
            "dB",
            true,
        );

        y += 216.0;
        self.card(y, 172.0, "", "Lifts the quiet parts up towards the peaks");
        self.toggle(x, y + 8.0, "3  Squeeze the dynamics", &mut s.opt_on);
        let on = s.opt_on;
        self.slider(
            7,
            x,
            y + 50.0,
            "Threshold",
            &mut s.threshold,
            -48.0,
            0.0,
            0.5,
            "dB",
            on,
        );
        self.slider(
            8,
            x,
            y + 80.0,
            "Ratio",
            &mut s.ratio,
            1.5,
            30.0,
            0.5,
            ": 1",
            on,
        );
        self.slider(
            9,
            x,
            y + 110.0,
            "Speed",
            &mut s.speed_ms,
            1.0,
            500.0,
            1.0,
            "ms",
            on,
        );
        self.slider(
            10,
            x,
            y + 140.0,
            "Boost",
            &mut s.boost,
            0.0,
            12.0,
            0.5,
            "dB",
            on,
        );

        y += 182.0;
        self.card(y, 190.0, "", "Final loudness, peaks kept under the ceiling");
        self.toggle(x, y + 8.0, "4  Final loudness", &mut s.loud_on);
        let on = s.loud_on;
        self.slider(
            11,
            x,
            y + 50.0,
            "Target",
            &mut s.target_lufs,
            -42.0,
            -5.0,
            0.5,
            "LUFS",
            on,
        );
        let presets = [
            ("Podcast", -16.0),
            ("Audiobook", -20.0),
            ("Video", -14.0),
            ("Broadcast", -23.0),
        ];
        for (i, (label, v)) in presets.iter().enumerate() {
            let bx = x + i as f32 * 86.0;
            if self.button(
                bx,
                y + 82.0,
                82.0,
                label,
                on && (s.target_lufs - v).abs() < 0.01,
                on,
            ) {
                s.target_lufs = *v;
                s.margin = if *label == "Audiobook" { -3.0 } else { -1.0 };
            }
        }
        self.slider(
            12,
            x,
            y + 118.0,
            "Peak ceiling",
            &mut s.margin,
            -20.0,
            0.0,
            0.5,
            "dB",
            true,
        );
        self.toggle(x, y + 158.0, "Measure the speech only", &mut s.speech_gated);

        if s != before {
            self.settings = s;
            self.dirty = true;
        }
    }

    fn draw(&mut self) {
        clear_background(BG);
        draw_rectangle(0.0, 0.0, W, 52.0, PANEL);
        draw_text("Speech Fixer", 16.0, 33.0, 28.0, TEXT);
        let busy = self.job.is_some();
        if self.button(190.0, 12.0, 90.0, "Open...", false, !busy)
            || is_key_pressed(KeyCode::O) && !busy
        {
            self.open_dialog();
        }
        if self.button(288.0, 12.0, 70.0, "Demo", false, !busy)
            || is_key_pressed(KeyCode::D) && !busy
        {
            self.features = None;
            self.start(Source::Demo);
        }
        let can_save = self.result.is_some() && !busy && !self.dirty;
        if self.button(366.0, 12.0, 120.0, "Save fixed...", false, can_save)
            || is_key_pressed(KeyCode::S) && can_save
        {
            self.save_dialog();
        }
        let can_auto = self.audio.is_some() && !busy;
        if self.button(494.0, 12.0, 96.0, "Auto fix", false, can_auto)
            || is_key_pressed(KeyCode::A) && can_auto
        {
            self.auto_fix();
        }
        if let Some(j) = &self.job {
            let p = j.prog.get();
            draw_rectangle(604.0, 20.0, 300.0, 12.0, CARD);
            draw_rectangle(604.0, 20.0, 300.0 * p, 12.0, ACCENT);
            draw_text(
                &format!("Working... {:.0} %", p * 100.0),
                916.0,
                31.0,
                18.0,
                TEXT,
            );
        } else {
            draw_text(&self.status, 604.0, 31.0, 15.0, DIM);
        }

        self.draw_lane(LANE1_Y, &format!("Original  {}", self.name), false);
        self.draw_lane(LANE2_Y, "Fixed", true);

        // ruler
        let t = self.duration();
        let ry = LANE2_Y + LANE_H + 16.0;
        if t > 0.0 {
            let step = [1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0]
                .into_iter()
                .find(|s| t / s <= 12.0)
                .unwrap_or(1200.0);
            let mut k = 0.0;
            while k <= t {
                let x = LANE_X + k / t * LANE_W;
                draw_line(x, ry - 12.0, x, ry - 6.0, 1.0, DIM);
                draw_text(&fmt_time(k), x + 2.0, ry + 2.0, 15.0, DIM);
                k += step;
            }
        }

        // transport
        let ty = ry + 10.0;
        let has = self.audio.is_some();
        let label = if self.player.is_playing() {
            "Stop"
        } else {
            "Play"
        };
        if self.button(LANE_X, ty, 80.0, label, false, has) || is_key_pressed(KeyCode::Space) {
            self.toggle_play();
        }
        draw_text("Listen to:", LANE_X + 100.0, ty + 19.0, 18.0, DIM);
        let lf = self.listen_fixed;
        if self.button(LANE_X + 180.0, ty, 100.0, "Original", !lf, has)
            || is_key_pressed(KeyCode::Key1)
        {
            self.listen(false);
        }
        if self.button(
            LANE_X + 286.0,
            ty,
            100.0,
            "Fixed",
            lf,
            has && self.result.is_some(),
        ) || is_key_pressed(KeyCode::Key2)
        {
            self.listen(true);
        }
        draw_text(
            &format!("{} / {}", fmt_time(self.position()), fmt_time(t)),
            LANE_X + 410.0,
            ty + 19.0,
            18.0,
            TEXT,
        );
        draw_text(
            "Space play   1/2 compare   Left/Right seek   A auto",
            LANE_X + 560.0,
            ty + 19.0,
            14.0,
            DIM,
        );
        if is_key_pressed(KeyCode::Left) {
            self.seek(self.position() - 5.0);
        }
        if is_key_pressed(KeyCode::Right) {
            self.seek(self.position() + 5.0);
        }
        if is_key_pressed(KeyCode::Home) {
            self.seek(0.0);
        }

        self.draw_meters(ty + 40.0);
        self.draw_panel();
    }

    fn read_mouse(&mut self) {
        let (x, y) = mouse_position();
        self.m = Mouse {
            x,
            y,
            down: is_mouse_button_down(MouseButton::Left),
            pressed: is_mouse_button_pressed(MouseButton::Left),
            released: is_mouse_button_released(MouseButton::Left),
        };
        if self.m.released || !self.m.down {
            self.drag = None;
        }
    }
}

// ---------------------------------------------------------------- self-test

struct Moved {
    click_at: f32,
    start: Option<f64>,
    after: Option<f32>,
}

fn self_test_report(app: &mut App, mv: &Moved) -> String {
    let mut steps = Vec::new();
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "'");
    let mut step = |name: &str, ok: bool, detail: String| {
        steps.push(format!(
            "{{\"step\": \"{name}\", \"passed\": {ok}, \"detail\": \"{}\"}}",
            esc(&detail)
        ));
    };
    let after = mv.after.unwrap_or(mv.click_at);
    let sent = app.player.sent_peak();
    let how = if app.player.has_device() {
        format!(
            "through the speakers (volume turned to 0 for the test), loudest sample sent {:.1} dB",
            20.0 * (sent + 1e-9).log10()
        )
    } else {
        format!(
            "no sound device found ({}), playhead ran on the clock",
            app.player.error.clone().unwrap_or_default()
        )
    };
    let sound_ok = !app.player.has_device() || sent > 0.01;
    step(
        "move",
        (mv.click_at - 15.0).abs() < 0.5 && after > mv.click_at + 0.5 && sound_ok,
        format!("clicked the middle of the waveform: playhead went to {:.2} s; pressed Play for about 1 s and it moved to {after:.2} s, {how}", mv.click_at),
    );
    let r = app.result.clone().unwrap();
    let s = app.settings.clone();
    // speech found vs. where the demo really has speech
    let (truth, clap_t) = app.truth.clone().unwrap_or_default();
    let n = r.speech.region.len();
    let mut agree = 0;
    let (mut missed, mut extra) = (0f32, 0f32);
    for i in 0..n {
        let t = i as f32 * dsp::HOP_S;
        let real = truth.iter().any(|(a, b)| t >= *a && t < *b);
        if real == r.speech.region[i] {
            agree += 1;
        } else if real {
            missed += dsp::HOP_S;
        } else {
            extra += dsp::HOP_S;
        }
    }
    let acc = 100.0 * agree as f32 / n as f32;
    let clap = (clap_t / dsp::HOP_S).ceil() as usize..((clap_t + 0.03) / dsp::HOP_S) as usize;
    let clap_ignored = clap.clone().all(|i| !r.speech.active[i]);
    let loud_ok = (r.after.speech - s.target_lufs).abs() <= 1.0;
    let peak_ok = r.after.true_peak <= s.margin + 0.3;
    let spread_ok = r.after.spread < r.before.spread * 0.5 && r.after.spread <= 4.0;
    let path = std::env::temp_dir().join("speech_fixer_selftest.wav");
    let saved = audio_io::save(&path, &r.out)
        .and_then(|_| audio_io::load(&path))
        .map(|a| a.len() == r.out.len())
        .unwrap_or(false);
    let _ = std::fs::remove_file(&path);
    // the automatic fix, as an AI assistant would run it
    let (auto_s, auto_r) = dsp::auto_fix(
        app.audio.as_ref().unwrap(),
        app.features.as_ref().unwrap(),
        &Settings::default(),
        &[],
        &Progress::new(),
    );
    // the fade-out at the end must not be raised more than the speech was
    let level = |a: &Audio, t0: f32, t1: f32| {
        let (i0, i1) = (
            (t0 * a.sr as f32) as usize,
            ((t1 * a.sr as f32) as usize).min(a.len()),
        );
        let e: f64 = (i0..i1)
            .map(|i| a.ch.iter().map(|c| (c[i] as f64).powi(2)).sum::<f64>())
            .sum::<f64>()
            / ((i1 - i0) * a.ch.len()).max(1) as f64;
        10.0 * (e + 1e-12).log10() as f32
    };
    let secs = r.out.secs();
    let fade_gain =
        level(&r.out, secs - 1.0, secs) - level(app.audio.as_ref().unwrap(), secs - 1.0, secs);
    // compare with the speech just before the music (the last talker, 24-26 s); Loudness
    // Optimize may lift quiet sound by its Boost on top, as RX does
    let speech_gain = level(&r.out, 24.0, 26.0) - level(app.audio.as_ref().unwrap(), 24.0, 26.0);
    let fade_ok = fade_gain <= speech_gain + s.boost + 1.0;
    let auto_ok = auto_r.after.spread <= r.after.spread + 0.05
        && auto_r.after.spread <= 2.5
        && (auto_r.after.speech - auto_s.target_lufs).abs() <= 1.0;
    step(
        "act",
        acc >= 85.0 && clap_ignored && loud_ok && peak_ok && spread_ok && saved && auto_ok && fade_ok,
        format!(
            "speech finder agrees with the real speech times {acc:.0}% of the time ({missed:.1} s of speech missed, {extra:.1} s marked that isn't speech), clap at {clap_t:.2} s left out: {clap_ignored}; quiet-to-loud difference {:.1} dB -> {:.1} dB; speech loudness {:.1} LUFS (target {:.1}); highest peak {:.1} dB (ceiling {:.1}); saved and re-opened: {saved}; ending fade-out raised {fade_gain:+.1} dB, the last speech before it {speech_gain:+.1} dB; auto fix -> {:.1} dB apart at {:.1} LUFS (preserve {:.0} %, responsiveness {:.2})",
            r.before.spread, r.after.spread, r.after.speech, s.target_lufs, r.after.true_peak, s.margin,
            auto_r.after.spread, auto_r.after.speech, auto_s.preserve, auto_s.responsiveness
        ),
    );
    let hud_ok = app.hud.len() == 5 && app.hud.iter().all(|l| !l.contains('–'));
    step(
        "hud",
        hud_ok,
        format!("measurements drawn: {}", app.hud.join("; ")),
    );
    format!("{{\"version\": 1, \"steps\": [{}]}}", steps.join(", "))
}

// ---------------------------------------------------------------- command line

const HELP: &str = "Speech Fixer - evens out speech in a recording.

  speech-fixer                         open the window (with the demo)
  speech-fixer <file>                  open the window with a recording
  speech-fixer --process <in> <out>    fix without a window. <in> can be a file or a
                                       folder; <out> a .wav file or a folder.
      --manual          use the settings as given (default: auto fix tunes them)
      --preset <name>   podcast (-16 LUFS), audiobook (-20), video (-14), broadcast (-23)
      --set name=value  change one setting; auto fix leaves it alone. Names:
                        sensitivity (0-10), level (on/off), target_level (dB),
                        responsiveness (0.3-7.1), preserve (%),
                        optimize_for (dialog/music),
                        background (dB, 0 to -20), optimize (on/off), threshold (dB),
                        ratio, speed (ms), boost (dB), loudness (on/off),
                        target (LUFS), margin (dB), tolerance (LU),
                        speech_only (on/off)
      --json            print the results as JSON (for scripts and AI assistants)
  speech-fixer --screenshot <png>      save a picture of the window and quit
  speech-fixer --self-test <json>      run the built-in checks and quit";

const AUDIO_EXT: [&str; 10] = [
    "wav", "mp3", "flac", "m4a", "aac", "mp4", "ogg", "aif", "aiff", "caf",
];

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Runs the command-line modes. Returns an exit code, or None to open the window.
fn cli(args: &[String]) -> Option<i32> {
    if args.iter().any(|a| a == "--help" || a == "-h" || a == "/?") {
        println!("{HELP}");
        return Some(0);
    }
    let i = args.iter().position(|a| a == "--process")?;
    let fail = |msg: String| {
        eprintln!("{msg}\n\n{HELP}");
        Some(2)
    };
    let (Some(inp), Some(out)) = (args.get(i + 1), args.get(i + 2)) else {
        return fail("--process needs an input and an output".into());
    };
    let mut s = Settings::default();
    let mut locked: Vec<String> = Vec::new();
    let (mut auto, mut json) = (true, false);
    let mut k = 1;
    while k < args.len() {
        match args[k].as_str() {
            "--process" => k += 2,
            "--manual" => auto = false,
            "--auto" => auto = true,
            "--json" => json = true,
            "--preset" => {
                k += 1;
                if let Err(e) = s.preset(args.get(k).map_or("", |v| v.as_str())) {
                    return fail(e);
                }
                locked.extend(["target".to_string(), "margin".to_string()]);
            }
            "--set" => {
                k += 1;
                let Some((key, val)) = args.get(k).and_then(|kv| kv.split_once('=')) else {
                    return fail("--set needs name=value".into());
                };
                if let Err(e) = s.set(key.trim(), val.trim()) {
                    return fail(e);
                }
                locked.push(key.trim().to_string());
            }
            other => return fail(format!("unknown option '{other}'")),
        }
        k += 1;
    }

    let (inp, out) = (PathBuf::from(inp), PathBuf::from(out));
    let jobs: Vec<(PathBuf, PathBuf)> = if inp.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&inp)
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.retain(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| AUDIO_EXT.contains(&e.to_lowercase().as_str()))
        });
        files.sort();
        if let Err(e) = std::fs::create_dir_all(&out) {
            return fail(format!("can't make the output folder: {e}"));
        }
        files
            .into_iter()
            .map(|f| {
                let o = out.join(format!(
                    "{} (fixed).wav",
                    f.file_stem().unwrap_or_default().to_string_lossy()
                ));
                (f, o)
            })
            .collect()
    } else if out.is_dir() {
        let o = out.join(format!(
            "{} (fixed).wav",
            inp.file_stem().unwrap_or_default().to_string_lossy()
        ));
        vec![(inp, o)]
    } else {
        vec![(inp, out)]
    };
    if jobs.is_empty() {
        return fail("no sound files found in the input folder".into());
    }

    let mut results = Vec::new();
    let mut failed = 0;
    for (src, dst) in &jobs {
        let r = run_job(
            Source::File(src.clone()),
            &s,
            auto,
            &locked,
            &Progress::new(),
        )
        .and_then(|d| audio_io::save(dst, &d.result.out).map(|_| d));
        match r {
            Ok(d) => {
                let (b, a) = (&d.result.before, &d.result.after);
                if json {
                    results.push(format!(
                        "{{\"file\": {}, \"output\": {}, \"ok\": true, \"auto\": {auto}, \"settings\": {}, \"before\": {}, \"after\": {}}}",
                        json_str(&src.to_string_lossy()), json_str(&dst.to_string_lossy()), d.settings.to_json(), b.to_json(), a.to_json()
                    ));
                } else {
                    println!("{}\n  -> {}", src.display(), dst.display());
                    println!("  speech found {:.0} %, speech loudness {:.1} -> {:.1} LUFS, whole track {:.1} -> {:.1} LUFS", b.speech_pct, b.speech, a.speech, b.integrated, a.integrated);
                    println!("  quiet-to-loud difference {:.1} -> {:.1} dB, highest peak {:.1} -> {:.1} dB", b.spread, a.spread, b.true_peak, a.true_peak);
                    if auto {
                        println!("  auto fix used preserve {:.0} %, responsiveness {:.2} s, sensitivity {:.1}", d.settings.preserve, d.settings.responsiveness, d.settings.sensitivity);
                    }
                }
            }
            Err(e) => {
                failed += 1;
                if json {
                    results.push(format!(
                        "{{\"file\": {}, \"ok\": false, \"error\": {}}}",
                        json_str(&src.to_string_lossy()),
                        json_str(&e)
                    ));
                } else {
                    eprintln!("{}: {e}", src.display());
                }
            }
        }
    }
    if json {
        println!("{{\"version\": 1, \"results\": [{}]}}", results.join(", "));
    }
    Some(if failed > 0 { 1 } else { 0 })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = cli(&args) {
        std::process::exit(code);
    }
    macroquad::Window::from_config(window_conf(), gui(args));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_delivery_and_wav_roundtrip() {
        let job = run_job(
            Source::Demo,
            &Settings::default(),
            false,
            &[],
            &Progress::new(),
        )
        .expect("process synthetic demo");
        let r = &job.result;
        let (truth, clap_t) = job.truth.as_ref().expect("demo ground truth");
        let correct = r
            .speech
            .region
            .iter()
            .enumerate()
            .filter(|(i, speech)| {
                let t = *i as f32 * dsp::HOP_S;
                **speech == truth.iter().any(|(a, b)| t >= *a && t < *b)
            })
            .count();
        let accuracy = correct as f32 / r.speech.region.len() as f32;
        assert!(accuracy >= 0.85, "speech accuracy: {accuracy}");
        let clap = (*clap_t / dsp::HOP_S).ceil() as usize..((*clap_t + 0.03) / dsp::HOP_S) as usize;
        assert!(
            clap.into_iter().all(|i| !r.speech.active[i]),
            "clap detected as speech"
        );
        assert!(
            r.after.spread < r.before.spread * 0.5 && r.after.spread <= 4.0,
            "speaker spread: {} -> {}",
            r.before.spread,
            r.after.spread
        );
        assert!(
            (r.after.speech - job.settings.target_lufs).abs() <= 1.0,
            "speech loudness: {}",
            r.after.speech
        );
        assert!(
            r.after.true_peak <= job.settings.margin + 0.3,
            "peak ceiling: {}",
            r.after.true_peak
        );
        assert!(r.out.ch.iter().flatten().all(|s| s.is_finite()));
        let (settings, fixed) = dsp::auto_fix(
            &job.audio,
            &job.features,
            &Settings::default(),
            &[],
            &Progress::new(),
        );
        assert!(fixed.after.spread <= r.after.spread + 0.05 && fixed.after.spread <= 2.5);
        assert!((fixed.after.speech - settings.target_lufs).abs() <= 1.0);

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("speech-fixer-{}-{stamp}.wav", std::process::id()));
        audio_io::save(&path, &r.out).expect("export WAV");
        let decoded = audio_io::load(&path).expect("decode exported WAV");
        assert_eq!(decoded.sr, r.out.sr);
        assert_eq!(decoded.ch.len(), r.out.ch.len());
        assert_eq!(decoded.len(), r.out.len());
        let max_error = decoded
            .ch
            .iter()
            .flatten()
            .zip(r.out.ch.iter().flatten())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_error < 0.000001, "24-bit WAV error: {max_error}");
        std::fs::remove_file(path).expect("remove generated WAV");
    }
}

async fn gui(args: Vec<String>) {
    let arg = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let shot = arg("--screenshot");
    let test = arg("--self-test");
    let file = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--") && Some(*a) != shot.as_ref() && Some(*a) != test.as_ref())
        .cloned();

    let mut app = App::new();
    if shot.is_some() || test.is_some() {
        app.run_blocking(Source::Demo);
        app.seek(9.0);
    } else if let Some(f) = file {
        app.start(Source::File(PathBuf::from(f)));
    } else {
        app.start(Source::Demo);
    }

    let mut frame = 0u32;
    let mut mv = Moved {
        click_at: 0.0,
        start: None,
        after: None,
    };
    let t_begin = get_time();
    loop {
        app.read_mouse();
        if test.is_some() {
            // scripted input: click the middle of the original waveform, press Play, listen ~1 s
            if frame == 5 {
                app.m = Mouse {
                    x: LANE_X + LANE_W / 2.0,
                    y: LANE1_Y + LANE_H / 2.0,
                    down: true,
                    pressed: true,
                    released: false,
                };
            }
            if frame == 6 {
                mv.click_at = app.position();
                app.player.set_volume(0.0);
                app.toggle_play();
                mv.start = Some(get_time());
            }
            if let (Some(t0), None) = (mv.start, mv.after) {
                if get_time() - t0 >= 1.0 {
                    mv.after = Some(app.position());
                    app.toggle_play();
                }
            }
        }
        app.poll();
        app.draw();
        frame += 1;
        if frame >= 120 {
            if let Some(p) = &shot {
                get_screen_data().export_png(p);
                break;
            }
            if let Some(p) = &test {
                if mv.after.is_some() || get_time() - t_begin > 10.0 {
                    let rep = self_test_report(&mut app, &mv);
                    let _ = std::fs::write(p, rep);
                    break;
                }
            }
        }
        next_frame().await;
    }
}
