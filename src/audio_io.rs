// Opening, saving and the built-in demo recording.

use crate::dsp::Audio;
use std::path::Path;
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as SErr, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

pub fn load(path: &Path) -> Result<Audio, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Can't open the file: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("This file type isn't supported ({e})"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("No sound found in the file")?;
    let id = track.id;
    let mut sr = track.codec_params.sample_rate.unwrap_or(44100);
    let mut dec = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Can't decode this sound ({e})"))?;
    let mut ch: Vec<Vec<f32>> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SErr::IoError(_)) | Err(SErr::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != id {
            continue;
        }
        match dec.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                sr = spec.rate;
                let n = spec.channels.count().max(1);
                if ch.is_empty() {
                    ch = vec![Vec::new(); n];
                }
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(buf);
                for (i, s) in sb.samples().iter().enumerate() {
                    if let Some(c) = ch.get_mut(i % n) {
                        c.push(*s);
                    }
                }
            }
            Err(SErr::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    ch.truncate(2); // keep front left/right of surround files
    if ch.is_empty() || ch[0].len() < sr as usize / 10 {
        return Err("The file has no usable sound".into());
    }
    Ok(Audio { sr, ch })
}

pub fn save(path: &Path, a: &Audio) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: a.ch.len() as u16,
        sample_rate: a.sr,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for i in 0..a.len() {
        for c in &a.ch {
            w.write_sample((c[i].clamp(-1.0, 1.0) * 8_388_607.0) as i32)
                .map_err(|e| e.to_string())?;
        }
    }
    w.finalize().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- demo recording

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

struct Reso {
    b0: f32,
    a1: f32,
    a2: f32,
    y1: f32,
    y2: f32,
}
impl Reso {
    fn new(f: f32, bw: f32, sr: f32) -> Self {
        let r = (-std::f32::consts::PI * bw / sr).exp();
        let th = 2.0 * std::f32::consts::PI * f / sr;
        Reso {
            b0: 1.0 - r,
            a1: -2.0 * r * th.cos(),
            a2: r * r,
            y1: 0.0,
            y2: 0.0,
        }
    }
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x - self.a1 * self.y1 - self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

const VOWELS: [[f32; 3]; 5] = [
    [730.0, 1090.0, 2440.0],
    [530.0, 1840.0, 2480.0],
    [270.0, 2290.0, 3010.0],
    [570.0, 840.0, 2410.0],
    [300.0, 870.0, 2240.0],
];

fn rms_to(v: &mut [f32], db: f32) {
    let r = (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt();
    let g = 10f32.powf(db / 20.0) / r.max(1e-9);
    v.iter_mut().for_each(|x| *x *= g);
}

/// A talker: syllables of a buzzing voice through vowel resonances, with pauses.
fn voice(secs: f32, f0: f32, sr: f32, rng: &mut Rng) -> (Vec<f32>, Vec<(f32, f32)>) {
    let n = (secs * sr) as usize;
    let mut out = vec![0f32; n];
    let mut t = 0usize;
    let mut phase = 0f32;
    let mut phrases = Vec::new();
    while t < n {
        let p0 = t;
        let words = 2 + (rng.next() * 4.0) as usize;
        for _ in 0..words {
            let syl = 1 + (rng.next() * 3.0) as usize;
            for _ in 0..syl {
                let len = (rng.range(0.14, 0.26) * sr) as usize;
                let v = VOWELS[(rng.next() * 5.0) as usize % 5];
                let mut rs: Vec<Reso> = v
                    .iter()
                    .zip([90.0, 110.0, 170.0])
                    .map(|(&f, bw)| Reso::new(f, bw, sr))
                    .collect();
                let lift = rng.range(-0.15, 0.2);
                for k in 0..len {
                    if t + k >= n {
                        break;
                    }
                    let u = k as f32 / len as f32;
                    let pitch = f0 * (1.0 + lift * u) * (1.0 + 0.01 * (rng.next() - 0.5));
                    phase = (phase + pitch / sr) % 1.0;
                    let src = 1.0 - 2.0 * phase + 0.05 * (rng.next() - 0.5);
                    let env = (std::f32::consts::PI * u).sin().powf(0.6);
                    let y: f32 = rs
                        .iter_mut()
                        .zip([1.0, 0.6, 0.25])
                        .map(|(r, g)| r.run(src) * g)
                        .sum();
                    out[t + k] = y * env;
                }
                t += len + (rng.range(0.02, 0.07) * sr) as usize;
            }
            t += (rng.range(0.12, 0.25) * sr) as usize;
        }
        phrases.push((p0 as f32 / sr, (t.min(n) as f32 / sr)));
        t += (rng.range(0.35, 0.7) * sr) as usize;
    }
    (out, phrases)
}

fn music(secs: f32, sr: f32) -> Vec<f32> {
    let n = (secs * sr) as usize;
    let chords = [
        [220.0, 277.18, 329.63],
        [196.0, 246.94, 293.66],
        [174.61, 220.0, 261.63],
        [196.0, 246.94, 329.63],
    ];
    (0..n)
        .map(|i| {
            let t = i as f32 / sr;
            let c = chords[(t / 2.0) as usize % 4];
            let mut s = 0.0;
            for f in c {
                for h in 1..6 {
                    s += (2.0 * std::f32::consts::PI * f * h as f32 * t).sin() / (h * h) as f32;
                }
            }
            s * (1.0 + 0.1 * (2.0 * std::f32::consts::PI * 0.5 * t).sin())
        })
        .collect()
}

/// A 30 s recording with a music intro, a loud talker, a hand clap, a quiet
/// talker, the loud talker again and a music outro. Returns the phrase times
/// and the clap time too.
pub fn demo() -> (Audio, Vec<(f32, f32)>, f32) {
    let sr = 44100f32;
    let n = (30.0 * sr) as usize;
    let mut rng = Rng(0x2545F4914F6CDD1D);
    let mut mix = vec![0f32; n];
    let put = |mix: &mut Vec<f32>, at: f32, v: &[f32]| {
        let s = (at * sr) as usize;
        for (i, x) in v.iter().enumerate() {
            if s + i < mix.len() {
                mix[s + i] += x;
            }
        }
    };
    let mut intro = music(4.5, sr);
    rms_to(&mut intro, -26.0);
    let fade = (1.5 * sr) as usize;
    let il = intro.len();
    for i in 0..fade {
        intro[il - fade + i] *= 1.0 - i as f32 / fade as f32 * 0.85;
    }
    put(&mut mix, 0.0, &intro);
    let mut truth = Vec::new();
    let mut talk = |mix: &mut Vec<f32>, at: f32, secs: f32, f0: f32, db: f32, rng: &mut Rng| {
        let (mut v, ph) = voice(secs, f0, sr, rng);
        rms_to(&mut v, db);
        put(mix, at, &v);
        truth.extend(ph.into_iter().map(|(a, b)| (a + at, b + at)));
        v
    };
    let a = talk(&mut mix, 3.0, 8.0, 115.0, -15.0, &mut rng);
    talk(&mut mix, 12.0, 8.0, 205.0, -33.0, &mut rng);
    talk(&mut mix, 21.0, 5.5, 120.0, -21.0, &mut rng);
    let mut outro = music(3.0, sr);
    rms_to(&mut outro, -24.0);
    // the music fades out over the last 1.5 s, like a real ending
    let fade = (1.5 * sr) as usize;
    let ol = outro.len();
    for i in 0..fade {
        outro[ol - fade + i] *= (1.0 - i as f32 / fade as f32).powi(3);
    }
    put(&mut mix, 27.0, &outro);
    // a single hand clap, almost full scale, in the first pause after 6.5 s
    let quiet = (0.08 * sr) as usize;
    let mut k = (3.5 * sr) as usize;
    while k + quiet < a.len() && a[k..k + quiet].iter().any(|x| x.abs() > 1e-4) {
        k += quiet / 4;
    }
    let clap_t = 3.0 + (k + quiet / 4) as f32 / sr;
    let clap: Vec<f32> = (0..(0.03 * sr) as usize)
        .map(|i| (rng.next() - 0.5) * 2.0 * (-(i as f32) / (0.004 * sr)).exp())
        .collect();
    put(
        &mut mix,
        clap_t,
        &clap.iter().map(|x| x * 0.98).collect::<Vec<_>>(),
    );
    // room noise
    let mut lp = 0f32;
    for x in mix.iter_mut() {
        lp = 0.97 * lp + 0.03 * (rng.next() - 0.5);
        *x += lp * 0.006;
    }
    for x in mix.iter_mut() {
        *x = x.clamp(-1.0, 1.0);
    }
    let right = mix.clone();
    (
        Audio {
            sr: sr as u32,
            ch: vec![mix, right],
        },
        truth,
        clap_t,
    )
}
