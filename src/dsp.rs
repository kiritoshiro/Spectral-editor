// Sound processing: speech finder, speaker leveler, peak tamer, loudness setter.
// Numbers come from MECHANICS.md (RX 12 presets) or are marked estimated there.

use rustfft::{num_complex::Complex32, FftPlanner};
use std::sync::atomic::{AtomicU32, Ordering};

pub const HOP_S: f32 = 0.010; // analysis step, 10 ms

// Values read from iZotope RX 12 Audio Editor.exe (see MECHANICS.md for the addresses).
/// Leveler analysis frame, 35 ms rounded up to a power of two (FUN_142f87de0).
pub const FRAME_S: f32 = 0.035;
/// Responsiveness range and its mapping to the inner "Time Scale" 0..4 (FUN_142bc27e0).
pub const RESP_MIN: f32 = 0.3;
pub const RESP_MAX: f32 = 7.1;
pub const RESP_SCALE: f32 = 1.7;
/// Half-length of the Hann-weighted level window at Time Scale 1 (FUN_142f87de0).
pub const LEVELER_HANN_S: f32 = 2.59;
/// SNR gate and level range the RX Leveler sets on its inner leveler (FUN_142bc3b50).
pub const SNR_THRESHOLD_DB: f32 = 10.0;
pub const LEVEL_RANGE_DB: f32 = 1.0;
/// Speech band edges used by RX's Leveler speech/breath detector (FUN_142f864b0).
pub const SPEECH_LO_HZ: f32 = 300.0;
pub const SPEECH_HI_HZ: f32 = 5000.0;
/// Speech needs a rise of at least this much within 150 ms somewhere in the last/next 0.5 s (estimated).
pub const ONSET_DB: f32 = 6.0;
/// Moments whose speech level is this far under the whole file's are not ridden (estimated backstop).
pub const LEVELER_REL_GATE_DB: f32 = 25.0;
/// Preserve Dynamic Range -> Maximum Gain = ((P - 100) / -100)^2 * 30 dB (FUN_142bc4d50).
pub const MAX_GAIN_SCALE_DB: f32 = 30.0;
/// Least gated material before the whole-file offset is used (FUN_142f8c110): 1 s.
pub const MIN_GATED_S: f32 = 1.0;
/// Optimize For: gate speed and release in seconds, Dialog / Music (FUN_142bc3830).
pub const DIALOG_GATE: (f32, f32) = (0.1, 0.6);
pub const MUSIC_GATE: (f32, f32) = (0.5, 0.4);

pub struct Audio {
    pub sr: u32,
    pub ch: Vec<Vec<f32>>,
}

impl Audio {
    pub fn len(&self) -> usize {
        self.ch.first().map_or(0, |c| c.len())
    }
    pub fn secs(&self) -> f32 {
        self.len() as f32 / self.sr as f32
    }
    pub fn mono(&self) -> Vec<f32> {
        let k = 1.0 / self.ch.len() as f32;
        (0..self.len())
            .map(|i| self.ch.iter().map(|c| c[i]).sum::<f32>() * k)
            .collect()
    }
}

pub struct Progress(AtomicU32);
impl Progress {
    pub fn new() -> Self {
        Progress(AtomicU32::new(0))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

#[derive(Clone, PartialEq)]
pub struct Settings {
    // Find speech (Dialogue Isolate)
    pub sensitivity: f32,
    // Even out speakers (Leveler)
    pub level_on: bool,
    pub target_level: f32,
    pub responsiveness: f32,
    pub preserve: f32,
    pub dialog: bool,
    pub background_db: f32,
    // Lift the quiet parts (Loudness Optimize)
    pub opt_on: bool,
    pub threshold: f32,
    pub ratio: f32,
    pub speed_ms: f32,
    pub boost: f32,
    // Final loudness (Loudness Control)
    pub loud_on: bool,
    pub target_lufs: f32,
    pub margin: f32,
    pub tolerance: f32,
    pub speech_gated: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sensitivity: 6.0,
            level_on: true,
            target_level: -20.0,
            responsiveness: 1.0,
            preserve: 20.0,
            dialog: true,
            background_db: 0.0,
            opt_on: true,
            threshold: -9.0,
            ratio: 4.0,
            speed_ms: 120.0,
            boost: 3.0,
            loud_on: true,
            target_lufs: -16.0,
            margin: -1.0,
            tolerance: 0.5,
            speech_gated: true,
        }
    }
}

// ---------------------------------------------------------------- analysis

pub struct Features {
    pub hop: usize,
    pub level_db: Vec<f32>,
    pub band_ratio: Vec<f32>,
    pub flatness: Vec<f32>,
    pub voicing: Vec<f32>,
}

pub fn hop_for(sr: u32) -> usize {
    ((sr as f32 * HOP_S).round() as usize).max(1)
}

pub fn analyse(a: &Audio, prog: &Progress, p0: f32, p1: f32) -> Features {
    let sr = a.sr as f32;
    let hop = hop_for(a.sr);
    let mono = a.mono();
    let n_hops = mono.len() / hop + 1;
    let win = ((sr * FRAME_S) as usize).next_power_of_two().max(256);
    let half = win / 2;
    let hann: Vec<f32> = (0..win)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / win as f32).cos())
        .collect();
    let w2: f32 = hann.iter().map(|w| w * w).sum();
    let mut planner = FftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(win);
    let inv = planner.plan_fft_inverse(win);

    // autocorrelation of the window itself, to undo its taper
    let mut wbuf: Vec<Complex32> = hann.iter().map(|&w| Complex32::new(w, 0.0)).collect();
    wbuf.resize(win, Complex32::new(0.0, 0.0));
    fwd.process(&mut wbuf);
    for c in wbuf.iter_mut() {
        *c = Complex32::new(c.norm_sqr(), 0.0);
    }
    inv.process(&mut wbuf);
    let wac: Vec<f32> = wbuf.iter().map(|c| c.re / wbuf[0].re).collect();

    let bin = |hz: f32| ((hz * win as f32 / sr).round() as usize).clamp(1, half);
    let (all0, all1) = (bin(60.0), bin(8000.0));
    // speech band of RX's breath/speech detector, 300 Hz - 5 kHz (FUN_142f864b0, 0x1452d8618 / 0x1452d8780)
    let (sp0, sp1) = (bin(SPEECH_LO_HZ), bin(SPEECH_HI_HZ));
    let (ac0, ac1) = (bin(60.0), bin(3000.0));
    let lag_min = (sr / 400.0) as usize;
    let lag_max = ((sr / 70.0) as usize).min(half - 1);

    let mut f = Features {
        hop,
        level_db: Vec::with_capacity(n_hops),
        band_ratio: Vec::with_capacity(n_hops),
        flatness: Vec::with_capacity(n_hops),
        voicing: Vec::with_capacity(n_hops),
    };
    let mut buf = vec![Complex32::new(0.0, 0.0); win];
    let mut pw = vec![0f32; half + 1];
    for i in 0..n_hops {
        let start = (i * hop) as isize - half as isize;
        let mut e = 0f32;
        for j in 0..win {
            let idx = start + j as isize;
            let x = if idx >= 0 && (idx as usize) < mono.len() {
                mono[idx as usize] * hann[j]
            } else {
                0.0
            };
            e += x * x;
            buf[j] = Complex32::new(x, 0.0);
        }
        f.level_db.push(10.0 * (e / w2 + 1e-10).log10());
        fwd.process(&mut buf);
        for k in 0..=half {
            pw[k] = buf[k].norm_sqr();
        }
        let e_all: f32 = pw[all0..=all1].iter().sum::<f32>() + 1e-20;
        let e_sp: f32 = pw[sp0..=sp1].iter().sum();
        f.band_ratio.push(e_sp / e_all);
        let nb = (sp1 - sp0 + 1) as f32;
        let lmean: f32 = pw[sp0..=sp1].iter().map(|p| (p + 1e-14).ln()).sum::<f32>() / nb;
        let amean = e_sp / nb + 1e-14;
        f.flatness.push((lmean.exp() / amean).clamp(0.0, 1.0));
        // periodicity: autocorrelation of the 60 Hz – 3 kHz band
        for k in 0..win {
            let kk = if k <= half { k } else { win - k };
            let p = if kk >= ac0 && kk <= ac1 { pw[kk] } else { 0.0 };
            buf[k] = Complex32::new(p, 0.0);
        }
        inv.process(&mut buf);
        let r0 = buf[0].re;
        let mut best = 0f32;
        if r0 > 1e-12 && lag_min < lag_max {
            for lag in lag_min..=lag_max {
                let v = buf[lag].re / r0 / wac[lag].max(0.05);
                best = best.max(v);
            }
        }
        f.voicing.push(best.clamp(0.0, 1.0));
        if i % 2000 == 0 {
            prog.set(p0 + (p1 - p0) * i as f32 / n_hops as f32);
        }
    }
    f
}

fn sig(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn moving<F: Fn(f32) -> f32>(v: &[f32], r: usize, map: F) -> Vec<f32> {
    let mut pre = vec![0f64; v.len() + 1];
    for (i, &x) in v.iter().enumerate() {
        pre[i + 1] = pre[i] + map(x) as f64;
    }
    (0..v.len())
        .map(|i| {
            let a = i.saturating_sub(r);
            let b = (i + r + 1).min(v.len());
            ((pre[b] - pre[a]) / (b - a) as f64) as f32
        })
        .collect()
}

pub struct Speech {
    pub score: Vec<f32>,
    pub active: Vec<bool>,
    pub region: Vec<bool>,
}

pub fn find_speech(f: &Features, sensitivity: f32, release_s: f32) -> Speech {
    let n = f.level_db.len();
    // noise floor: lowest 50 ms block within ±4 s
    let blk = 5;
    let nb = n.div_ceil(blk);
    let bmean: Vec<f32> = (0..nb)
        .map(|b| {
            let s = &f.level_db[b * blk..((b + 1) * blk).min(n)];
            s.iter().sum::<f32>() / s.len() as f32
        })
        .collect();
    let reach = 80;
    let bfloor: Vec<f32> = (0..nb)
        .map(|b| {
            let a = b.saturating_sub(reach);
            let e = (b + reach + 1).min(nb);
            bmean[a..e]
                .iter()
                .cloned()
                .fold(f32::MAX, f32::min)
                .max(-100.0)
        })
        .collect();
    let band_db: Vec<f32> = (0..n)
        .map(|i| f.level_db[i] + 10.0 * (f.band_ratio[i] + 1e-6).log10())
        .collect();
    let band_s = moving(&band_db, 1, |x| x);
    let m1 = moving(&band_s, 25, |x| x);
    let m2 = moving(&band_s, 25, |x| x * x);
    let voiced_ratio = moving(&f.voicing, 30, |v| if v > 0.5 { 1.0 } else { 0.0 });
    // syllable onsets: speech keeps rising again (several dB within 150 ms), a fade-out
    // or a decaying note only falls, so its "rhythm" isn't speech (estimated)
    let rise: Vec<f32> = (0..n)
        .map(|i| {
            band_s[i]
                - band_s[i.saturating_sub(15)..=i]
                    .iter()
                    .cloned()
                    .fold(f32::MAX, f32::min)
        })
        .collect();
    let onset: Vec<f32> = (0..n)
        .map(|i| {
            rise[i.saturating_sub(50)..(i + 51).min(n)]
                .iter()
                .cloned()
                .fold(0.0, f32::max)
        })
        .collect();
    let thr = (0.35 - (sensitivity - 6.0) * 0.05).clamp(0.05, 0.9);

    let mut score = vec![0f32; n];
    let mut active = vec![false; n];
    for i in 0..n {
        let snr = f.level_db[i] - bfloor[i / blk];
        let modu = (m2[i] - m1[i] * m1[i]).max(0.0).sqrt();
        let vr = voiced_ratio[i];
        let s = sig((snr - SNR_THRESHOLD_DB) / 2.5)
            * sig((modu - 3.5) / 1.2)
            * sig((f.band_ratio[i] - 0.35) / 0.1)
            * sig((0.45 - f.flatness[i]) / 0.1)
            * sig((f.voicing[i] - 0.4) / 0.07)
            * sig((vr - 0.15) / 0.06)
            * sig((onset[i] - ONSET_DB) / 1.5);
        let music = sig((vr - 0.9) / 0.03) * sig((5.0 - modu) / 1.0);
        score[i] = s * (1.0 - music);
        active[i] = score[i] > thr && f.level_db[i] > -75.0;
    }
    // drop blips shorter than 40 ms
    let mut i = 0;
    while i < n {
        if active[i] {
            let s = i;
            while i < n && active[i] {
                i += 1;
            }
            if i - s < 4 {
                active[s..i].iter_mut().for_each(|a| *a = false);
            }
        } else {
            i += 1;
        }
    }
    // regions: join gaps shorter than the gate release, pad 150 ms each side
    let join = (release_s / HOP_S) as usize;
    let mut region = active.clone();
    let mut last: Option<usize> = None;
    for i in 0..n {
        if active[i] {
            if let Some(l) = last {
                if i - l <= join {
                    region[l..i].iter_mut().for_each(|r| *r = true);
                }
            }
            last = Some(i);
        }
    }
    let base = region.clone();
    for i in 0..n {
        if base[i] {
            let a = i.saturating_sub(15);
            let b = (i + 16).min(n);
            region[a..b].iter_mut().for_each(|r| *r = true);
        }
    }
    Speech {
        score,
        active,
        region,
    }
}

// ---------------------------------------------------------------- loudness (ITU-R BS.1770)

struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}
impl Biquad {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

fn k_filters(sr: f64) -> (Biquad, Biquad) {
    use std::f64::consts::PI;
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (PI * f0 / sr).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (PI * f0 / sr).tan();
    let d = 1.0 + k / q + k * k;
    let hp = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / d, (1.0 - k / q + k * k) / d],
        z: [0.0; 2],
    };
    (shelf, hp)
}

/// K-weighted power per 10 ms hop, summed over channels.
pub fn k_power(a: &Audio, hop: usize) -> Vec<f64> {
    let n_hops = a.len() / hop + 1;
    let mut out = vec![0f64; n_hops];
    let mut cnt = vec![0u32; n_hops];
    for (ci, c) in a.ch.iter().enumerate() {
        let (mut s, mut h) = k_filters(a.sr as f64);
        for (n, &x) in c.iter().enumerate() {
            let y = h.run(s.run(x as f64));
            let b = ((n + hop / 2) / hop).min(n_hops - 1);
            out[b] += y * y;
            if ci == 0 {
                cnt[b] += 1;
            }
        }
    }
    for (o, c) in out.iter_mut().zip(cnt) {
        *o /= c.max(1) as f64;
    }
    out
}

fn lufs(p: f64) -> f32 {
    (-0.691 + 10.0 * (p + 1e-20).log10()) as f32
}

/// Gated integrated loudness. With `only` given, a 400 ms block uses only those hops.
pub fn integrated(kp: &[f64], only: Option<&[bool]>) -> f32 {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i + 40 <= kp.len().max(40) {
        let end = (i + 40).min(kp.len());
        let (mut s, mut c) = (0f64, 0usize);
        for j in i..end {
            if only.map_or(true, |m| m[j]) {
                s += kp[j];
                c += 1;
            }
        }
        if c >= if only.is_some() { 8 } else { 1 } {
            blocks.push(s / c as f64);
        }
        i += 10;
    }
    let abs: Vec<f64> = blocks.into_iter().filter(|&p| lufs(p) > -70.0).collect();
    if abs.is_empty() {
        return -70.0;
    }
    let rel = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let g: Vec<f64> = abs.into_iter().filter(|&p| lufs(p) > rel).collect();
    if g.is_empty() {
        return -70.0;
    }
    lufs(g.iter().sum::<f64>() / g.len() as f64)
}

/// Short-term (3 s) speech loudness readings, every 0.5 s.
pub fn speech_short_term(kp: &[f64], active: &[bool]) -> Vec<f32> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < kp.len() {
        let end = (i + 300).min(kp.len());
        let (mut s, mut c) = (0f64, 0usize);
        for j in i..end {
            if active[j] {
                s += kp[j];
                c += 1;
            }
        }
        if c >= 60 {
            v.push(lufs(s / c as f64));
        }
        i += 50;
    }
    v
}

fn pct(v: &[f32], p: f32) -> f32 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[((s.len() - 1) as f32 * p).round() as usize]
}

// ---------------------------------------------------------------- peaks

fn interp(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    0.5 * (2.0 * p1
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t)
}

/// Per-sample peak including an estimate of the peaks between samples (4x).
fn peak_env(ch: &[Vec<f32>]) -> Vec<f32> {
    let n = ch[0].len();
    let mut e = vec![0f32; n];
    for c in ch {
        for i in 0..n {
            let g = |k: isize| c[(i as isize + k).clamp(0, n as isize - 1) as usize];
            let (p0, p1, p2, p3) = (g(-1), g(0), g(1), g(2));
            let mut m = p1.abs();
            for t in [0.25, 0.5, 0.75] {
                m = m.max(interp(p0, p1, p2, p3, t).abs());
            }
            e[i] = e[i].max(m);
        }
    }
    e
}

pub fn true_peak_db(ch: &[Vec<f32>]) -> f32 {
    let m = peak_env(ch).into_iter().fold(0f32, f32::max);
    20.0 * (m + 1e-9).log10()
}

pub fn sample_peak_db(ch: &[Vec<f32>]) -> f32 {
    let m = ch
        .iter()
        .flat_map(|c| c.iter())
        .fold(0f32, |m, x| m.max(x.abs()));
    20.0 * (m + 1e-9).log10()
}

/// Look-ahead brickwall limiter (5 ms look-ahead, 80 ms release).
fn limit(ch: &mut [Vec<f32>], sr: u32, ceiling_db: f32) {
    let n = ch[0].len();
    if n == 0 {
        return;
    }
    let ceil = 10f32.powf(ceiling_db / 20.0);
    let la = ((sr as f32 * 0.005) as usize).max(1);
    let r: Vec<f32> = peak_env(ch)
        .into_iter()
        .map(|p| if p > ceil { ceil / p } else { 1.0 })
        .collect();
    // h[i] = min r[i..=i+la]
    let mut h = vec![1f32; n];
    let mut dq: std::collections::VecDeque<usize> = Default::default();
    for j in (0..n).rev() {
        while dq.back().is_some_and(|&k| r[k] >= r[j]) {
            dq.pop_back();
        }
        dq.push_back(j);
        while dq.front().is_some_and(|&k| k > j + la) {
            dq.pop_front();
        }
        h[j] = r[*dq.front().unwrap()];
    }
    drop(r);
    // g[i] = mean h[i-la..=i], then slow release
    let rel = (-1.0 / (sr as f32 * 0.080)).exp();
    let mut sum = 0f64;
    let mut gs = 1f32;
    for i in 0..n {
        sum += h[i] as f64;
        let len = if i >= la {
            sum -= h[i - la] as f64;
            0
        } else {
            la - i
        };
        let g = ((sum + len as f64) / (la + 1) as f64) as f32;
        gs = if g < gs { g } else { g + (gs - g) * rel };
        for c in ch.iter_mut() {
            c[i] = (c[i] * gs).clamp(-ceil, ceil);
        }
    }
}

// ---------------------------------------------------------------- processing

#[derive(Clone, Default)]
pub struct Stats {
    pub integrated: f32,
    pub speech: f32,
    pub true_peak: f32,
    pub spread: f32,
    pub quietest: f32,
    pub loudest: f32,
    pub speech_pct: f32,
}

pub fn stats(a: &Audio, sp: &Speech, hop: usize) -> Stats {
    let kp = k_power(a, hop);
    let st = speech_short_term(&kp, &sp.active);
    let (q, l) = if st.len() >= 3 {
        (pct(&st, 0.1), pct(&st, 0.9))
    } else if st.is_empty() {
        (0.0, 0.0)
    } else {
        (pct(&st, 0.0), pct(&st, 1.0))
    };
    Stats {
        integrated: integrated(&kp, None),
        speech: integrated(&kp, Some(&sp.active)),
        true_peak: true_peak_db(&a.ch),
        spread: l - q,
        quietest: q,
        loudest: l,
        speech_pct: 100.0 * sp.region.iter().filter(|&&r| r).count() as f32
            / sp.region.len().max(1) as f32,
    }
}

pub struct Segment {
    pub a: usize,
    pub b: usize,
    pub before: f32,
    pub after: f32,
}

pub struct Processed {
    pub out: std::sync::Arc<Audio>,
    pub speech: Speech,
    pub hop: usize,
    pub gain_db: Vec<f32>,
    pub before: Stats,
    pub after: Stats,
    pub segments: Vec<Segment>,
}

fn smooth_zero_phase(v: &mut [f32], tau_hops: f32) {
    let a = (-1.0 / tau_hops.max(0.5)).exp();
    let mut s = v.first().copied().unwrap_or(0.0);
    for x in v.iter_mut() {
        s = a * s + (1.0 - a) * *x;
        *x = s;
    }
    let mut s = v.last().copied().unwrap_or(0.0);
    for x in v.iter_mut().rev() {
        s = a * s + (1.0 - a) * *x;
        *x = s;
    }
}

impl Settings {
    /// Inner "Time Scale" 0..4 from Responsiveness (FUN_142bc27e0).
    pub fn time_scale(&self) -> f32 {
        ((self.responsiveness - RESP_MIN) / RESP_SCALE).clamp(0.0, 4.0)
    }
    /// Largest change the leveler may make around the whole-file offset (FUN_142bc4d50).
    pub fn max_gain_db(&self) -> f32 {
        let f = (self.preserve - 100.0) / -100.0;
        f * f * MAX_GAIN_SCALE_DB
    }
    pub fn gate(&self) -> (f32, f32) {
        if self.dialog {
            DIALOG_GATE
        } else {
            MUSIC_GATE
        }
    }
}

/// Speaker leveler, two passes like RX (FUN_142f8c110): first the whole-file
/// offset from the gated speech level, then gain riding from a Hann-weighted
/// speech-only level around each moment, kept within Maximum Gain of the offset.
fn leveler_curve(kp: &[f64], sp: &Speech, s: &Settings) -> Vec<f32> {
    let n = kp.len();
    let mut gain = vec![0f32; n];
    if s.level_on {
        // pass 1: gated average over the whole file
        let (mut sum, mut cnt) = (0f64, 0usize);
        for i in 0..n {
            if sp.active[i] {
                sum += kp[i];
                cnt += 1;
            }
        }
        let speech_level = if cnt > 0 {
            lufs(sum / cnt as f64)
        } else {
            -100.0
        };
        let offset = if cnt as f32 * HOP_S >= MIN_GATED_S {
            (s.target_level - speech_level).clamp(-40.0, 40.0)
        } else {
            0.0
        };
        // pass 2: ride the gain
        let half = ((LEVELER_HANN_S * s.time_scale()).max(0.1) / HOP_S) as usize;
        let w: Vec<f64> = (0..=half)
            .map(|k| 0.5 + 0.5 * (std::f64::consts::PI * k as f64 / (half + 1) as f64).cos())
            .collect();
        let max_gain = s.max_gain_db();
        let mut defined: Vec<Option<f32>> = vec![None; n];
        for i in 0..n {
            let (mut ps, mut pw) = (0f64, 0f64);
            let a = i.saturating_sub(half);
            let b = (i + half).min(n - 1);
            for j in a..=b {
                if sp.active[j] {
                    let wj = w[i.abs_diff(j)];
                    ps += wj * kp[j];
                    pw += wj;
                }
            }
            // a moment far below the file's speech level is a fade-out, breath or noise the
            // speech finder took for speech; riding it up blew up music fade-outs by ~18 dB
            if pw >= 5.0 && lufs(ps / pw) >= speech_level - LEVELER_REL_GATE_DB {
                let local = s.target_level - lufs(ps / pw) - offset;
                // inside the level range nothing is ridden (estimated reading of "Level Range")
                let ride = if local.abs() <= LEVEL_RANGE_DB {
                    0.0
                } else {
                    local - LEVEL_RANGE_DB * local.signum()
                };
                defined[i] = Some(offset + ride.clamp(-max_gain, max_gain));
            }
        }
        // between speech the gain slides from one part to the next
        let idx: Vec<usize> = (0..n).filter(|&i| defined[i].is_some()).collect();
        if idx.is_empty() {
            gain.iter_mut().for_each(|g| *g = offset);
        } else {
            let mut k = 0;
            for i in 0..n {
                while k + 1 < idx.len() && idx[k + 1] <= i {
                    k += 1;
                }
                gain[i] = if i <= idx[0] {
                    defined[idx[0]].unwrap()
                } else if k + 1 >= idx.len() {
                    defined[idx[k]].unwrap()
                } else {
                    let (a, b) = (idx[k], idx[k + 1]);
                    let t = (i - a) as f32 / (b - a) as f32;
                    defined[a].unwrap() * (1.0 - t) + defined[b].unwrap() * t
                };
            }
        }
    }
    if s.background_db < 0.0 {
        let mut mask: Vec<f32> = sp
            .region
            .iter()
            .map(|&r| if r { 0.0 } else { 1.0 })
            .collect();
        smooth_zero_phase(&mut mask, 8.0);
        for (g, m) in gain.iter_mut().zip(mask) {
            *g += m * s.background_db;
        }
    }
    // gate speed as the smoothing time (estimated use of RX's "Gate Speed")
    smooth_zero_phase(&mut gain, s.gate().0 / HOP_S);
    gain
}

fn apply_curve(a: &mut Audio, gain_db: &[f32], hop: usize) {
    let lin: Vec<f32> = gain_db.iter().map(|g| 10f32.powf(g / 20.0)).collect();
    let last = lin.len() - 1;
    for c in a.ch.iter_mut() {
        for (n, x) in c.iter_mut().enumerate() {
            let p = n as f32 / hop as f32;
            let i = (p as usize).min(last);
            let j = (i + 1).min(last);
            let t = p - i as f32;
            *x *= lin[i] * (1.0 - t) + lin[j] * t;
        }
    }
}

fn gain(a: &mut Audio, db: f32) {
    let g = 10f32.powf(db / 20.0);
    a.ch.iter_mut()
        .for_each(|c| c.iter_mut().for_each(|x| *x *= g));
}

/// Loudness Optimize's gain law (FUN_143156da0): with boost B, ratio R and
/// threshold T, levels under the knee T - B*R/(R-1) get the full boost, the boost
/// fades to 0 dB between the knee and T (output rises at 1/R there), and levels
/// above T are left alone. An upward compressor: quiet parts come up.
pub fn optimize_gain_db(x_db: f32, s: &Settings) -> f32 {
    let r = s.ratio.max(1.5);
    let knee = s.threshold - s.boost * r / (r - 1.0);
    if s.boost <= 0.0 || x_db > s.threshold {
        0.0
    } else if x_db <= knee {
        s.boost
    } else {
        s.boost * (s.threshold - x_db) / (s.threshold - knee)
    }
}

/// Detector as in ConfigurableUpwardsCompress (FUN_1431562a0, FUN_143156460):
/// peak hold over +/-Speed ms, then a one-pole envelope with time constant Speed ms.
fn optimize(a: &mut Audio, s: &Settings) {
    let n = a.len();
    if n == 0 || s.boost <= 0.0 {
        return;
    }
    let sr = a.sr as f32;
    let w = ((s.speed_ms * 0.001 * sr) as usize).max(1);
    let pk: Vec<f32> = (0..n)
        .map(|i| a.ch.iter().fold(0f32, |m, c| m.max(c[i].abs())))
        .collect();
    let mut held = vec![0f32; n];
    let mut dq: std::collections::VecDeque<usize> = Default::default();
    let mut next = 0;
    for i in 0..n {
        while next < n && next <= i + w {
            while dq.back().is_some_and(|&k| pk[k] <= pk[next]) {
                dq.pop_back();
            }
            dq.push_back(next);
            next += 1;
        }
        while dq.front().is_some_and(|&k| k + w < i) {
            dq.pop_front();
        }
        held[i] = pk[*dq.front().unwrap()];
    }
    drop(pk);
    let c = 1.0 - (-1.0 / (s.speed_ms / 1000.0 * sr)).exp();
    let mut env = held[0];
    for i in 0..n {
        env += c * (held[i] - env);
        let g = 10f32.powf(optimize_gain_db(20.0 * (env + 1e-9).log10(), s) / 20.0);
        for ch in a.ch.iter_mut() {
            ch[i] *= g;
        }
    }
}

pub fn process(input: &Audio, f: &Features, s: &Settings, prog: &Progress) -> Processed {
    let hop = f.hop;
    let sp = find_speech(f, s.sensitivity, s.gate().1);
    let kp_in = k_power(input, hop);
    let mut y = Audio {
        sr: input.sr,
        ch: input.ch.clone(),
    };

    let gain_db = leveler_curve(&kp_in, &sp, s);
    apply_curve(&mut y, &gain_db, hop);
    prog.set(0.7);

    let has_speech = sp.active.iter().any(|&a| a);
    let measure = |a: &Audio| {
        let kp = k_power(a, hop);
        if s.speech_gated && has_speech {
            integrated(&kp, Some(&sp.active))
        } else {
            integrated(&kp, None)
        }
    };
    if s.opt_on {
        // RX's "Optimized Podcast Loudness" chain sets the loudness first, then optimizes
        let l = measure(&y);
        gain(&mut y, (s.target_lufs - l).clamp(-40.0, 40.0));
        optimize(&mut y, s);
    }
    prog.set(0.8);

    if s.loud_on {
        let mut g = (s.target_lufs - measure(&y)).clamp(-40.0, 40.0);
        let pre = y.ch.clone();
        for _ in 0..3 {
            y.ch = pre.clone();
            gain(&mut y, g);
            limit(&mut y.ch, y.sr, s.margin);
            let err = s.target_lufs - measure(&y);
            if err.abs() <= s.tolerance {
                break;
            }
            g += err;
        }
    } else {
        limit(&mut y.ch, y.sr, s.margin);
    }
    prog.set(0.9);

    let before = stats(input, &sp, hop);
    let after = stats(&y, &sp, hop);
    let kp_out = k_power(&y, hop);
    let mut segments = Vec::new();
    let mut i = 0;
    let n = sp.region.len();
    while i < n {
        if sp.region[i] {
            let a = i;
            while i < n && sp.region[i] {
                i += 1;
            }
            let m: Vec<bool> = (0..n).map(|j| j >= a && j < i && sp.active[j]).collect();
            if i - a >= 100 && m.iter().filter(|&&x| x).count() >= 30 {
                segments.push(Segment {
                    a,
                    b: i,
                    before: integrated(&kp_in, Some(&m)),
                    after: integrated(&kp_out, Some(&m)),
                });
            }
        } else {
            i += 1;
        }
    }
    prog.set(1.0);
    Processed {
        out: std::sync::Arc::new(y),
        speech: sp,
        hop,
        gain_db,
        before,
        after,
        segments,
    }
}

// ---------------------------------------------------------------- automatic fixing

/// Setting names for `--set name=value`, in the order shown by `--help`.
pub const SETTING_KEYS: [&str; 17] = [
    "sensitivity",
    "level",
    "target_level",
    "responsiveness",
    "preserve",
    "optimize_for",
    "background",
    "optimize",
    "threshold",
    "ratio",
    "speed",
    "boost",
    "loudness",
    "target",
    "margin",
    "tolerance",
    "speech_only",
];

impl Settings {
    pub fn set(&mut self, key: &str, val: &str) -> Result<(), String> {
        let num = || {
            val.parse::<f32>()
                .map_err(|_| format!("{key}: '{val}' is not a number"))
        };
        let flag = || match val {
            "on" | "true" | "1" | "yes" => Ok(true),
            "off" | "false" | "0" | "no" => Ok(false),
            _ => Err(format!("{key}: use on or off")),
        };
        match key {
            "sensitivity" => self.sensitivity = num()?.clamp(0.0, 10.0),
            "level" => self.level_on = flag()?,
            "target_level" => self.target_level = num()?.clamp(-40.0, 0.0),
            "responsiveness" => self.responsiveness = num()?.clamp(RESP_MIN, RESP_MAX),
            "preserve" => self.preserve = num()?.clamp(0.0, 100.0),
            "optimize_for" => {
                self.dialog = match val {
                    "dialog" | "dialogue" | "speech" => true,
                    "music" => false,
                    _ => return Err("optimize_for: use dialog or music".into()),
                }
            }
            "background" => self.background_db = num()?.clamp(-20.0, 0.0),
            "optimize" => self.opt_on = flag()?,
            "threshold" => self.threshold = num()?.clamp(-48.0, 0.0),
            "ratio" => self.ratio = num()?.clamp(1.5, 30.0),
            "speed" => self.speed_ms = num()?.clamp(1.0, 500.0),
            "boost" => self.boost = num()?.clamp(0.0, 12.0),
            "loudness" => self.loud_on = flag()?,
            "target" => self.target_lufs = num()?.clamp(-42.0, -5.0),
            "margin" => self.margin = num()?.clamp(-20.0, 0.0),
            "tolerance" => self.tolerance = num()?.clamp(0.1, 10.0),
            "speech_only" => self.speech_gated = flag()?,
            _ => {
                return Err(format!(
                    "unknown setting '{key}' (known: {})",
                    SETTING_KEYS.join(", ")
                ))
            }
        }
        Ok(())
    }

    /// Delivery targets from RX's Loudness Control presets.
    pub fn preset(&mut self, name: &str) -> Result<(), String> {
        let (t, m) = match name {
            "podcast" => (-16.0, -1.0),
            "audiobook" => (-20.0, -3.0),
            "video" => (-14.0, -1.0),
            "broadcast" => (-23.0, -1.0),
            _ => {
                return Err(format!(
                    "unknown preset '{name}' (podcast, audiobook, video, broadcast)"
                ))
            }
        };
        self.target_lufs = t;
        self.margin = m;
        Ok(())
    }

    pub fn to_json(&self) -> String {
        let b = |v: bool| if v { "true" } else { "false" };
        format!(
            "{{\"sensitivity\": {:.1}, \"level\": {}, \"target_level\": {:.1}, \"responsiveness\": {:.2}, \"preserve\": {:.0}, \"optimize_for\": \"{}\", \"background\": {:.0}, \"optimize\": {}, \"threshold\": {:.1}, \"ratio\": {:.1}, \"speed\": {:.0}, \"boost\": {:.1}, \"loudness\": {}, \"target\": {:.1}, \"margin\": {:.1}, \"tolerance\": {:.1}, \"speech_only\": {}}}",
            self.sensitivity, b(self.level_on), self.target_level, self.responsiveness, self.preserve, if self.dialog { "dialog" } else { "music" }, self.background_db,
            b(self.opt_on), self.threshold, self.ratio, self.speed_ms, self.boost, b(self.loud_on), self.target_lufs, self.margin, self.tolerance, b(self.speech_gated)
        )
    }
}

impl Stats {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"loudness_lufs\": {:.1}, \"speech_loudness_lufs\": {:.1}, \"peak_db\": {:.1}, \"quiet_to_loud_db\": {:.1}, \"speech_percent\": {:.0}}}",
            self.integrated, self.speech, self.true_peak, self.spread, self.speech_pct
        )
    }
}

/// Fix without anyone touching a slider: find enough speech, then tighten the
/// leveler until every part is within 2 dB. Settings named in `locked` stay as given.
pub fn auto_fix(
    input: &Audio,
    f: &Features,
    base: &Settings,
    locked: &[String],
    prog: &Progress,
) -> (Settings, Processed) {
    let free = |k: &str| !locked.iter().any(|l| l == k);
    let mut s = base.clone();
    let mut r = process(input, f, &s, prog);
    for _ in 0..2 {
        if r.before.speech_pct >= 25.0 || s.sensitivity >= 9.0 || !free("sensitivity") {
            break;
        }
        s.sensitivity = (s.sensitivity + 1.5).min(10.0);
        r = process(input, f, &s, prog);
    }
    for _ in 0..4 {
        if r.after.spread <= 2.0 || !s.level_on {
            break;
        }
        let before = s.clone();
        if free("preserve") {
            s.preserve = (s.preserve - 10.0).max(0.0);
        }
        if free("responsiveness") {
            s.responsiveness = (s.responsiveness * 0.75).max(RESP_MIN + 0.2);
        }
        if s == before {
            break;
        }
        r = process(input, f, &s, prog);
    }
    (s, r)
}
