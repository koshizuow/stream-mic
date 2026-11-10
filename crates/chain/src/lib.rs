//! The chain (build-plan.md step 5): the stages in order, two paths, presets.
//!
//!   send:    denoise -> low cut -> EQ -> de-esser -> expander -> compressor -> reverb -> limiter   (to the virtual mic)
//!   monitor:            low cut -> EQ -> de-esser -> expander -> compressor -> reverb -> clipper   (to the headphones)
//!
//! The low cut is a stage of its own (2026-11-10; it was the EQ's first filter): the rumble is gone before the EQ
//! and the dynamics read the voice. It comes right after denoise, which runs on its own thread with the raw mic.
//!
//! The monitor path skips denoise (20 ms) so hearing yourself is quick (routing.md); its stages are a second set
//! with the same settings. The reverb goes before the limiter so what it adds cannot pass the ceiling.
//!
//! Of all these only denoise (20 ms) and the limiter's look-ahead (2.15 ms) delay the sound; the others work sample by
//! sample. So the monitor ends in a soft clipper at the same ceiling instead of the limiter: no delay at all, still no
//! loud peaks in the ears, and it sounds like what is sent (user 2026-11-07: as little delay as possible when singing).
//!
//! Bypass (hear the raw voice) glides over 10 ms to the input delayed by the send path's latency, so the timing
//! does not jump. A stage switched off is skipped; denoise off also takes its 20 ms out of the send path.

pub mod autoset;
pub mod preset;

use std::time::Instant;

use stream_mic_denoise::Denoise;
use stream_mic_dsp::deesser::Deesser;
use stream_mic_dsp::dynamics::{Compressor, Expander};
use stream_mic_dsp::eq::{Eq, LowCut};
use stream_mic_dsp::limiter::Limiter;
use stream_mic_dsp::reverb::Reverb;
use stream_mic_dsp::Stage;

pub use preset::Preset;

pub const STAGE_NAMES: [&str; 8] = ["denoise", "lowcut", "eq", "deess", "exp", "comp", "verb", "lim"];

/// The monitor's end: straight through up to 0.7 of the ceiling, then bent smoothly towards it (never reaching it).
/// No look-ahead, so no delay; a peak is rounded off instead of turned down ahead of time.
fn soft_clip(x: &mut [f32], ceil_db: f64) {
    let c = 10f64.powf(ceil_db / 20.0) as f32;
    let k = 0.7 * c;
    for v in x {
        let a = v.abs();
        if a > k {
            *v = v.signum() * (k + (c - k) * ((a - k) / (c - k)).tanh());
        }
    }
}

/// One path's stages after denoise
struct Path {
    /// the monitor: a soft clipper where the send has the limiter
    clip: bool,
    lowcut: LowCut,
    eq: Eq,
    deess: Deesser,
    exp: Expander,
    comp: Compressor,
    verb: Reverb,
    lim: Limiter,
}

impl Path {
    fn new(p: &Preset, clip: bool) -> Path {
        let mut verb = Reverb::new(p.verb_params());
        if !p.follow_music {
            verb.without_music();
        }
        Path {
            clip,
            lowcut: LowCut::new(p.lowcut.freq),
            eq: Eq::new(p.eq_params()),
            deess: Deesser::new(p.deess_params()),
            exp: Expander::new(p.exp_params()),
            comp: Compressor::new(p.comp_params()),
            verb,
            lim: Limiter::new(p.lim_params()),
        }
    }

    fn prepare(&mut self, fs: f64, block: usize) {
        self.lowcut.prepare(fs, block);
        self.eq.prepare(fs, block);
        self.deess.prepare(fs, block);
        self.exp.prepare(fs, block);
        self.comp.prepare(fs, block);
        self.verb.prepare(fs, block);
        self.lim.prepare(fs, block);
    }

    fn set(&mut self, p: &Preset) {
        self.lowcut.set_hz(p.lowcut.freq);
        self.eq.set_params(p.eq_params());
        self.deess.set_params(p.deess_params());
        self.exp.set_params(p.exp_params());
        self.comp.set_params(p.comp_params());
        self.verb.set_params(p.verb_params());
        if !p.follow_music {
            self.verb.without_music();
        }
        self.lim.set_params(p.lim_params());
    }

    /// times: seconds spent per stage (index as STAGE_NAMES), added to when given
    fn process(&mut self, on: &preset::On, x: &mut [f32], music: Option<&[f32]>, mut times: Option<&mut [f64; 8]>) {
        let mut run = |i: usize, f: &mut dyn FnMut(&mut [f32]), x: &mut [f32]| {
            let t = times.as_ref().map(|_| Instant::now());
            f(x);
            if let (Some(tm), Some(t)) = (times.as_deref_mut(), t) {
                tm[i] += t.elapsed().as_secs_f64();
            }
        };
        if on.lowcut {
            run(1, &mut |x| self.lowcut.process(x), x);
        }
        if on.eq {
            run(2, &mut |x| self.eq.process(x), x);
        }
        if on.deess {
            run(3, &mut |x| self.deess.process(x), x);
        }
        if on.exp {
            run(4, &mut |x| self.exp.process(x), x);
        }
        if on.comp {
            run(5, &mut |x| self.comp.process(x), x);
        }
        if on.verb {
            if let Some(m) = music {
                self.verb.set_music(m);
            }
            run(6, &mut |x| self.verb.process(x), x);
        }
        if on.lim {
            run(7, &mut |x| self.lim.process(x), x);
        } else if self.clip {
            // the monitor without the limiter: the ears still get a ceiling
            let ceil = self.lim.params().ceil_db;
            run(7, &mut |x| soft_clip(x, ceil), x);
        }
    }

    fn latency(&self, on: &preset::On) -> usize {
        if on.lim {
            self.lim.latency()
        } else {
            0
        }
    }
}

pub struct Chain {
    p: Preset,
    fs: f64,
    denoise: Option<Denoise>,
    send: Path,
    monitor: Path,
    bypass: bool,
    /// 0 processed .. 1 bypassed, per sample
    xf: f64,
    xf_step: f64,
    dry: Vec<f32>,
    dry_pos: usize,
    max_block: usize,
    /// denoise done outside (the engine's worker thread, with this many samples of latency): the send path is given
    /// the denoised signal instead of running the model here
    ext: Option<usize>,
    /// seconds per stage since the last `take_times` (when profiling)
    pub times: Option<[f64; 8]>,
}

impl Chain {
    /// The denoise model loads here (it takes a moment); a chain whose preset never denoises still has it, so the
    /// knob can turn it on later.
    pub fn new(p: Preset) -> Result<Chain, String> {
        let denoise = Some(Denoise::new(p.denoise_params())?);
        Ok(Chain {
            send: Path::new(&p, false),
            monitor: Path::new(&p, true),
            p,
            fs: 48000.0,
            denoise,
            bypass: false,
            xf: 0.0,
            xf_step: 0.0,
            dry: Vec::new(),
            dry_pos: 0,
            max_block: 0,
            ext: None,
            times: None,
        })
    }

    pub fn prepare(&mut self, fs: f64, max_block: usize) {
        self.fs = fs;
        if let Some(d) = self.denoise.as_mut() {
            d.prepare(fs, max_block);
        }
        self.send.prepare(fs, max_block);
        self.monitor.prepare(fs, max_block);
        self.xf_step = 1.0 / (0.010 * fs);
        // the dry signal for bypass, as long as the longest send path (denoise + limiter)
        let longest = self.denoise.as_ref().map(|d| d.latency()).unwrap_or(0).max(self.ext.unwrap_or(0)) + self.send.lim.latency() + 1;
        self.dry = vec![0.0; longest + 1];
        self.dry_pos = 0;
        self.max_block = max_block;
    }

    pub fn preset(&self) -> &Preset {
        &self.p
    }

    /// A new preset or new settings: everything glides there (the stages ramp their own changes)
    pub fn set_preset(&mut self, p: Preset) {
        if let Some(d) = self.denoise.as_mut() {
            d.set_params(p.denoise_params());
        }
        self.send.set(&p);
        self.monitor.set(&p);
        self.p = p;
    }

    /// Denoise runs elsewhere with `latency` samples of delay (call before prepare): `process_with` is then given its
    /// output. None: the model runs inside `process` as usual.
    pub fn set_external_denoise(&mut self, latency: Option<usize>) {
        self.ext = latency;
    }

    pub fn set_bypass(&mut self, on: bool) {
        self.bypass = on;
    }

    /// Samples the send path's output lags its input
    pub fn send_latency(&self) -> usize {
        let d = if !self.p.on.denoise {
            0
        } else if let Some(e) = self.ext {
            e
        } else {
            self.denoise.as_ref().map(|d| d.latency()).unwrap_or(0)
        };
        d + self.send.latency(&self.p.on)
    }

    /// Samples the monitor lags the mic: the stages chosen for it (denoise and the limiter are the only ones that delay)
    pub fn monitor_latency(&self) -> usize {
        let m = self.p.on.and(&self.p.monitor);
        let d = if m.denoise { self.send_latency() - self.send.latency(&self.p.on) } else { 0 };
        d + self.monitor.latency(&m)
    }

    /// Where the send path's delay comes from, samples: (denoise, the limiter's look-ahead)
    pub fn send_parts(&self) -> (usize, usize) {
        let all = self.send_latency();
        let lim = self.send.latency(&self.p.on);
        (all - lim, lim)
    }

    /// Where the monitor's delay comes from, samples: (denoise, the limiter's look-ahead)
    pub fn monitor_parts(&self) -> (usize, usize) {
        let m = self.p.on.and(&self.p.monitor);
        let lim = self.monitor.latency(&m);
        (self.monitor_latency() - lim, lim)
    }

    /// How much the send path is turning the voice down now, dB (each <= 0): the expander (the pauses), the
    /// compressor and the limiter (loud parts). Stages that are off read 0.
    pub fn reduction_db(&self) -> (f64, f64, f64) {
        let on = &self.p.on;
        let r = |b: bool, v: f64| if b { v.min(0.0) } else { 0.0 };
        (r(on.exp, self.send.exp.reduction_db()), r(on.comp, self.send.comp.reduction_db()), r(on.lim, self.send.lim.reduction_db()))
    }

    /// One block: `input` the mic, `music` the backing track (for the reverb, singing), `send` and `monitor` the
    /// outputs (same length as input, up to max_block).
    pub fn process(&mut self, input: &[f32], music: Option<&[f32]>, send: &mut [f32], monitor: Option<&mut [f32]>) {
        self.process_with(input, None, music, send, monitor)
    }

    /// As `process`, with denoise done outside (set_external_denoise): `denoised` is the input through it, as late
    /// as it said. Used only while the preset has denoise on.
    pub fn process_with(&mut self, input: &[f32], denoised: Option<&[f32]>, music: Option<&[f32]>, send: &mut [f32], monitor: Option<&mut [f32]>) {
        let n = input.len();
        let on = self.p.on.clone();
        let mut times = self.times.take();
        match denoised {
            Some(d) if on.denoise && self.ext.is_some() => send[..n].copy_from_slice(&d[..n]),
            _ => send[..n].copy_from_slice(input),
        }
        let mon_on = on.and(&self.p.monitor);
        if on.denoise && self.ext.is_none() {
            if let Some(d) = self.denoise.as_mut() {
                let t = Instant::now();
                d.process(&mut send[..n]);
                if let Some(tm) = times.as_mut() {
                    tm[0] += t.elapsed().as_secs_f64();
                }
            }
        }
        let mut monitor = monitor;
        if let Some(m) = monitor.as_deref_mut() {
            // the monitor starts from the mic, or from the send's denoised signal when denoise is chosen for it
            m[..n].copy_from_slice(if mon_on.denoise { &send[..n] } else { input });
            self.monitor.process(&mon_on, &mut m[..n], music, None);
        }
        self.send.process(&on, &mut send[..n], music, times.as_mut());
        self.times = times;
        // bypass, both outputs (the user hears what goes out): the input delayed by each path's latency, crossfaded
        let (lat, mlat) = (self.send_latency(), self.monitor_latency());
        let len = self.dry.len();
        let fade = |x: f32, d: f32, k: f64| (x as f64 * (1.0 - k) + d as f64 * k) as f32;
        for i in 0..n {
            self.dry[self.dry_pos] = input[i];
            let d = self.dry[(self.dry_pos + len - lat.min(len - 1)) % len];
            let dm = self.dry[(self.dry_pos + len - mlat.min(len - 1)) % len];
            self.dry_pos = (self.dry_pos + 1) % len;
            let target = if self.bypass { 1.0 } else { 0.0 };
            if self.xf < target {
                self.xf = (self.xf + self.xf_step).min(1.0);
            } else if self.xf > target {
                self.xf = (self.xf - self.xf_step).max(0.0);
            }
            if self.xf > 0.0 {
                send[i] = fade(send[i], d, self.xf);
                if let Some(m) = monitor.as_deref_mut() {
                    m[i] = fade(m[i], dm, self.xf);
                }
            }
        }
    }

    pub fn reset(&mut self) {
        if let Some(d) = self.denoise.as_mut() {
            d.reset();
        }
        self.send.prepare(self.fs, self.max_block);
        self.monitor.prepare(self.fs, self.max_block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::On;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let amp = [0.003, 0.3, 0.05, 0.6][(i / 9000) % 4];
                (s as f32 / u32::MAX as f32 - 0.5) * amp
            })
            .collect()
    }

    fn run(name: &str, x: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
        let mut c = Chain::new(Preset::built_in(name).unwrap()).unwrap();
        c.prepare(48000.0, block);
        let music = noise(x.len(), 7);
        let (mut s, mut m) = (vec![0.0; x.len()], vec![0.0; x.len()]);
        let mut i = 0;
        while i < x.len() {
            let j = (i + block).min(x.len());
            c.process(&x[i..j], Some(&music[i..j]), &mut s[i..j], Some(&mut m[i..j]));
            i = j;
        }
        (s, m)
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = noise(48000, 1);
        for name in ["talk-directional", "sing"] {
            let want = run(name, &x, 1);
            for b in [128, 480, 4096] {
                assert!(run(name, &x, b) == want, "{name} block {b}");
            }
        }
    }

    #[test]
    fn denoise_outside_gives_the_same() {
        // the model run apart (as the engine's worker does) and handed over: the same output as inside
        let x = noise(48000, 5);
        let mut p = Preset::built_in("talk-directional").unwrap();
        p.set_knob("denoise", 60.0);
        let want = {
            let mut c = Chain::new(p.clone()).unwrap();
            c.prepare(48000.0, 128);
            let mut s = vec![0.0; x.len()];
            for (i, b) in x.chunks(128).enumerate() {
                c.process(b, None, &mut s[i * 128..i * 128 + b.len()], None);
            }
            s
        };
        let mut d = stream_mic_denoise::Denoise::new(p.denoise_params()).unwrap();
        d.prepare(48000.0, 128);
        let lat = d.latency();
        let mut dn = x.clone();
        for b in dn.chunks_mut(128) {
            d.process(b);
        }
        let mut c = Chain::new(p).unwrap();
        c.set_external_denoise(Some(lat));
        c.prepare(48000.0, 128);
        assert_eq!(c.send_latency(), lat + 103);
        let mut s = vec![0.0; x.len()];
        for (i, b) in x.chunks(128).enumerate() {
            c.process_with(b, Some(&dn[i * 128..i * 128 + b.len()]), None, &mut s[i * 128..i * 128 + b.len()], None);
        }
        assert!(s == want);
    }

    #[test]
    fn monitor_takes_the_stages_chosen_for_it() {
        let x = noise(48000, 9);
        let mut p = Preset::built_in("talk-directional").unwrap();
        p.set_knob("denoise", 40.0);
        let mut c = Chain::new(p.clone()).unwrap();
        c.prepare(48000.0, 128);
        // by default: no denoise, no look-ahead, so no delay
        assert_eq!(c.monitor_latency(), 0);
        // everything chosen: the monitor is the send (and as late)
        p.monitor = p.on.clone();
        c.set_preset(p.clone());
        assert_eq!(c.monitor_latency(), c.send_latency());
        let (mut s, mut m) = (vec![0.0; x.len()], vec![0.0; x.len()]);
        for (i, b) in x.chunks(128).enumerate() {
            let r = i * 128..i * 128 + b.len();
            c.process(b, None, &mut s[r.clone()], Some(&mut m[r]));
        }
        assert!(s == m);
        // the limiter alone: its look-ahead
        p.monitor = On { lim: true, ..On::monitor_default() };
        c.set_preset(p);
        assert_eq!(c.monitor_latency(), 103);
    }

    #[test]
    fn bypass_is_the_raw_voice_in_both() {
        // 原音 is for the headphones too: the user hears what goes out (the headphones were still processed, found
        // by tools/func_test.py)
        let x = noise(48000, 11);
        let mut c = Chain::new(Preset::built_in("talk-directional").unwrap()).unwrap();
        c.prepare(48000.0, 128);
        c.set_bypass(true);
        let (mut s, mut m) = (vec![0.0; x.len()], vec![0.0; x.len()]);
        for (i, b) in x.chunks(128).enumerate() {
            let r = i * 128..i * 128 + b.len();
            c.process(b, None, &mut s[r.clone()], Some(&mut m[r]));
        }
        // after the 10 ms glide: each output is the input, as late as its path
        let (ls, lm) = (c.send_latency(), c.monitor_latency());
        for i in 2000..x.len() {
            assert_eq!(s[i], x[i - ls], "send at {i}");
            assert_eq!(m[i], x[i - lm], "monitor at {i}");
        }
    }

    #[test]
    fn latencies() {
        let mut c = Chain::new(Preset::built_in("talk-directional").unwrap()).unwrap();
        c.prepare(48000.0, 128);
        // denoise is off in every built-in preset (user 2026-11-07)
        assert_eq!(c.send_latency(), 103);
        // the monitor ends in the soft clipper: no delay (user 2026-11-07)
        assert_eq!(c.monitor_latency(), 0);
        let mut p = Preset::built_in("talk-directional").unwrap();
        p.set_knob("denoise", 50.0);
        c.set_preset(p);
        assert_eq!(c.send_latency(), 960 + 103);
        assert_eq!(c.monitor_latency(), 0);
        c.set_preset(Preset::built_in("sing").unwrap());
        assert_eq!(c.send_latency(), 103);
    }

    #[test]
    fn never_over_the_ceiling() {
        let x: Vec<f32> = noise(96000, 3).iter().map(|v| v * 6.0).collect();
        for name in ["talk-omni", "talk-directional", "sing"] {
            let (s, m) = run(name, &x, 128);
            let ceil = 10f32.powf(-1.0 / 20.0) * 1.0001;
            assert!(s.iter().all(|v| v.abs() <= ceil), "{name} send");
            // the monitor's clipper bounds the samples (not the true peak between them: it is for the ears)
            assert!(m.iter().all(|v| v.abs() <= ceil), "{name} monitor");
        }
    }
}
