// Listening: plays the original or the fixed version through the speakers.
// Both share one timeline, so switching between them is seamless.

use crate::dsp::Audio;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Default)]
struct Shared {
    orig: Option<Arc<Audio>>,
    fixed: Option<Arc<Audio>>,
    fixed_on: bool,
    playing: bool,
    pos: f64,
    volume: f32,
    sent_peak: f32,
}

pub struct Player {
    shared: Arc<Mutex<Shared>>,
    stream: Option<cpal::Stream>,
    pub error: Option<String>,
    // without a sound device the playhead still runs on the clock
    clock: Option<(Instant, f64)>,
}

impl Player {
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            volume: 1.0,
            fixed_on: true,
            ..Default::default()
        }));
        match open(shared.clone()) {
            Ok(s) => Player {
                shared,
                stream: Some(s),
                error: None,
                clock: None,
            },
            Err(e) => Player {
                shared,
                stream: None,
                error: Some(e),
                clock: None,
            },
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn has_device(&self) -> bool {
        self.stream.is_some()
    }

    pub fn set_sources(&self, orig: Option<Arc<Audio>>, fixed: Option<Arc<Audio>>) {
        let mut s = self.lock();
        s.orig = orig;
        s.fixed = fixed;
    }

    pub fn set_fixed(&self, on: bool) {
        self.lock().fixed_on = on;
    }

    pub fn set_volume(&self, v: f32) {
        self.lock().volume = v;
    }

    fn duration(s: &Shared) -> f64 {
        s.orig.as_ref().map_or(0.0, |a| a.secs() as f64)
    }

    pub fn play(&mut self) {
        let mut s = self.lock();
        if s.orig.is_none() {
            return;
        }
        if s.pos >= Self::duration(&s) - 0.05 {
            s.pos = 0.0;
        }
        s.playing = true;
        let pos = s.pos;
        drop(s);
        if self.stream.is_none() {
            self.clock = Some((Instant::now(), pos));
        }
    }

    pub fn stop(&mut self) {
        let p = self.position();
        let mut s = self.lock();
        s.playing = false;
        s.pos = p;
        drop(s);
        self.clock = None;
    }

    pub fn seek(&mut self, t: f64) {
        let mut s = self.lock();
        s.pos = t.clamp(0.0, Self::duration(&s));
        let (pos, playing) = (s.pos, s.playing);
        drop(s);
        if playing && self.stream.is_none() {
            self.clock = Some((Instant::now(), pos));
        }
    }

    pub fn is_playing(&mut self) -> bool {
        if let Some((t0, p0)) = self.clock {
            let mut s = self.lock();
            if p0 + t0.elapsed().as_secs_f64() >= Self::duration(&s) {
                s.playing = false;
                s.pos = Self::duration(&s);
                drop(s);
                self.clock = None;
            }
        }
        self.lock().playing
    }

    pub fn position(&self) -> f64 {
        let s = self.lock();
        match self.clock {
            Some((t0, p0)) if s.playing => {
                (p0 + t0.elapsed().as_secs_f64()).min(Self::duration(&s))
            }
            _ => s.pos,
        }
    }

    /// Loudest sample sent to the speakers so far (before the volume knob).
    pub fn sent_peak(&self) -> f32 {
        self.lock().sent_peak
    }
}

fn open(shared: Arc<Mutex<Shared>>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let dev = host
        .default_output_device()
        .ok_or("No speakers or headphones found")?;
    let cfg = dev
        .default_output_config()
        .map_err(|e| format!("Can't use the speakers: {e}"))?;
    let fmt = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    use cpal::SampleFormat as F;
    let stream = match fmt {
        F::F32 => build::<f32>(&dev, config, shared),
        F::F64 => build::<f64>(&dev, config, shared),
        F::I16 => build::<i16>(&dev, config, shared),
        F::I32 => build::<i32>(&dev, config, shared),
        F::U16 => build::<u16>(&dev, config, shared),
        F::U8 => build::<u8>(&dev, config, shared),
        other => Err(format!("Speaker format {other:?} isn't supported")),
    }?;
    stream
        .play()
        .map_err(|e| format!("Can't start the speakers: {e}"))?;
    Ok(stream)
}

fn build<T: SizedSample + FromSample<f32>>(
    dev: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, String> {
    let out_sr = config.sample_rate as f64;
    let out_ch = (config.channels as usize).max(1);
    dev.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let Ok(mut s) = shared.try_lock() else {
                data.iter_mut().for_each(|x| *x = T::from_sample(0.0));
                return;
            };
            let src = if s.fixed_on && s.fixed.is_some() {
                s.fixed.clone()
            } else {
                s.orig.clone()
            };
            let (Some(a), true) = (src, s.playing) else {
                data.iter_mut().for_each(|x| *x = T::from_sample(0.0));
                return;
            };
            let n = a.len();
            let step = a.sr as f64 / out_sr;
            let mut idx = s.pos * a.sr as f64;
            let mut peak = s.sent_peak;
            for frame in data.chunks_mut(out_ch) {
                let i = idx as usize;
                if i + 1 >= n {
                    s.playing = false;
                    frame.iter_mut().for_each(|x| *x = T::from_sample(0.0));
                    continue;
                }
                let t = (idx - i as f64) as f32;
                for (c, out) in frame.iter_mut().enumerate() {
                    let ch = &a.ch[c.min(a.ch.len() - 1)];
                    let v = ch[i] * (1.0 - t) + ch[i + 1] * t;
                    peak = peak.max(v.abs());
                    *out = T::from_sample((v * s.volume).clamp(-1.0, 1.0));
                }
                idx += step;
            }
            s.sent_peak = peak;
            s.pos = (idx / a.sr as f64).min(n as f64 / a.sr as f64);
        },
        |e| eprintln!("sound: {e}"),
        None,
    )
    .map_err(|e| format!("Can't open the speakers: {e}"))
}
