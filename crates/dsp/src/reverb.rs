//! Reverb: an 8-channel FDN after Signalsmith's "Let's Write A Reverb" (2021), with Jot's per-line
//! decay (design/dsp/reverb.md). Chosen over Dattorro's plate because it is dense from the start:
//! echo density 0.94 over the first 50 ms against Dattorro's 0.38, which takes 250 to 350 ms to
//! become diffuse (grainy on a voice). Matches tools/reffdn.py.
//!
//!   x -> pre-delay -> low cut -> copied to 8 channels
//!     -> diffuser: 4 steps (4, 8, 16, 32 ms ranges), each: per-channel delay, Hadamard, sign flips
//!     -> loop: per-channel delay 45 ms * 2^(c/8), damping low pass, gain for T60, Householder,
//!        plus the diffused input; the wet is taken where the loop is written
//!     -> even channels left, odd right
//!
//! The diffuser centres the wet about 29 ms after the dry, so `pre_ms` is added on top of that.
//!
//! Early reflections (the styles' "initial reflections", TC-Helicon VoiceTone R1's table): twelve taps on the
//! pre-delayed, low-cut input spread over `er_ms`, alternately left and right, falling off, through the same
//! damping as the tail; `er` is their level (0 none). They come before the tail's diffuse onset, so a short pre-delay
//! with strong reflections reads as a small room. Without them (er 0) the output is the FDN alone, as tools/reffdn.py.
//!
//! Mix follows the backing track (singing, DECISIONS.md): `set_music` hands it the music for the
//! block, sample by sample (so the result does not depend on the block size); above MUSIC_ON_DB the mix glides to `mix`, below to `mix_dry`, over `follow_s`.

use crate::svf::{Coefs, Kind, Svf};
use crate::Stage;

pub const N: usize = 8;
const STEPS_MS: [f64; 4] = [4.0, 8.0, 16.0, 32.0];
const SIZE_MS: f64 = 45.0;
const FRACS: [f64; N] = [0.33, 0.71, 0.12, 0.58, 0.91, 0.27, 0.44, 0.86];
const FLIPS: [[f64; N]; 4] = [
    [1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0],
    [-1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0, 1.0],
    [1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0],
    [-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0],
];
/// Music louder than this (dBFS, 300 ms power average) counts as "playing".
pub const MUSIC_ON_DB: f64 = -50.0;
const MUSIC_TAU_S: f64 = 0.3;
/// The reflections' times as fractions of `er_ms`, and their gains (even taps left, odd right)
const ER_AT: [f64; 12] = [0.07, 0.13, 0.21, 0.27, 0.36, 0.43, 0.52, 0.6, 0.69, 0.78, 0.88, 1.0];
const ER_GAIN: [f64; 12] = [1.0, 0.93, 0.85, 0.79, 0.72, 0.66, 0.6, 0.55, 0.5, 0.46, 0.42, 0.38];
/// The longest reflection pattern, ms
pub const ER_MAX_MS: f64 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerbParams {
    pub t60_s: f64,
    /// Extra pre-delay on top of the diffuser's own (about 29 ms).
    pub pre_ms: f64,
    pub lowcut_hz: f64,
    pub damp_hz: f64,
    /// Wet amount with music playing (or always, without a music input), 0..1.
    pub mix: f64,
    /// Wet amount with no music.
    pub mix_dry: f64,
    /// Seconds to glide between the two.
    pub follow_s: f64,
    /// Early reflections' level, 0 none .. 1
    pub er: f64,
    /// Over how long the early reflections spread, ms (up to ER_MAX_MS)
    pub er_ms: f64,
    /// A low pass on the wet (the published presets' "LPF" or "high cut"), Hz; 0 or 20 kHz and above: off
    pub hc_hz: f64,
}

impl Default for VerbParams {
    /// The singing preset.
    fn default() -> Self {
        VerbParams { t60_s: 1.6, pre_ms: 10.0, lowcut_hz: 250.0, damp_hz: 7000.0, mix: 0.14, mix_dry: 0.04, follow_s: 1.5, er: 0.0, er_ms: 30.0, hc_hz: 0.0 }
    }
}

/// Multichannel delay line: `buf[c * len + i]`, one write position for all channels.
#[derive(Clone, Debug, Default)]
struct Lines {
    buf: Vec<f64>,
    len: usize,
    pos: usize,
    delay: [usize; N],
}

impl Lines {
    fn new(delay: [usize; N]) -> Lines {
        let len = *delay.iter().max().unwrap();
        Lines { buf: vec![0.0; len * N], len, pos: 0, delay }
    }
    #[inline]
    fn read(&self) -> [f64; N] {
        let mut o = [0.0; N];
        for (c, v) in o.iter_mut().enumerate() {
            // written delay[c] samples ago; delay == len reads the slot about to be overwritten
            let i = (self.pos + self.len - self.delay[c]) % self.len;
            *v = self.buf[c * self.len + i];
        }
        o
    }
    #[inline]
    fn write(&mut self, v: &[f64; N]) {
        for (c, x) in v.iter().enumerate() {
            self.buf[c * self.len + self.pos] = *x;
        }
        self.pos += 1;
        if self.pos == self.len {
            self.pos = 0;
        }
    }
    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
        self.pos = 0;
    }
}

#[inline]
fn hadamard(v: &mut [f64; N]) {
    let mut h = 1;
    while h < N {
        let mut i = 0;
        while i < N {
            for j in i..i + h {
                let (a, b) = (v[j], v[j + h]);
                v[j] = a + b;
                v[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
    let s = 1.0 / (N as f64).sqrt();
    v.iter_mut().for_each(|x| *x *= s);
}

#[derive(Clone, Debug)]
pub struct Reverb {
    p: VerbParams,
    fs: f64,
    pre: Vec<f64>,
    pre_len: usize,
    pre_pos: usize,
    lowcut: Svf,
    hc: [Svf; 2],
    diff: Vec<Lines>,
    lp_loop: Lines,
    gains: [f64; N],
    damp: f64,
    lp: [f64; N],
    // early reflections: the low-cut input's last ER_MAX_MS, the taps' delays in samples, the tone's state
    er_buf: Vec<f64>,
    er_pos: usize,
    er_tap: [usize; 12],
    er_lp: [f64; 2],
    // follow the music
    music: Vec<f32>,
    music_len: usize,
    music_pow: f64,
    music_a: f64,
    mix_now: f64,
}

impl Reverb {
    pub fn new(p: VerbParams) -> Self {
        let mut r = Reverb {
            p,
            fs: 48000.0,
            pre: Vec::new(),
            pre_len: 0,
            pre_pos: 0,
            lowcut: Svf::default(),
            hc: Default::default(),
            diff: Vec::new(),
            lp_loop: Lines::default(),
            gains: [0.0; N],
            damp: 0.0,
            lp: [0.0; N],
            er_buf: Vec::new(),
            er_pos: 0,
            er_tap: [1; 12],
            er_lp: [0.0; 2],
            music: Vec::new(),
            music_len: 0,
            music_pow: 0.0,
            music_a: 0.0,
            mix_now: p.mix_dry,
        };
        r.prepare(48000.0, 0);
        r
    }

    fn update(&mut self) {
        let fs = self.fs;
        for c in 0..N {
            let d = self.lp_loop.delay[c] as f64;
            self.gains[c] = 10f64.powf(-3.0 * d / (self.p.t60_s.max(0.05) * fs));
        }
        self.damp = (-2.0 * std::f64::consts::PI * self.p.damp_hz / fs).exp();
        let kind = if self.p.lowcut_hz > 0.0 { Kind::HighPass } else { Kind::Off };
        self.lowcut.set_ramped(Coefs::design(kind, self.p.lowcut_hz, std::f64::consts::FRAC_1_SQRT_2, 0.0, fs), (0.01 * fs) as u32);
        let hk = if self.p.hc_hz > 0.0 && self.p.hc_hz < 20000.0f64.min(fs * 0.45) { Kind::LowPass } else { Kind::Off };
        for f in self.hc.iter_mut() {
            f.set_ramped(Coefs::design(hk, self.p.hc_hz, std::f64::consts::FRAC_1_SQRT_2, 0.0, fs), (0.01 * fs) as u32);
        }
        self.pre_len = (self.p.pre_ms.max(0.0) * 1e-3 * fs).round() as usize;
        let span = self.p.er_ms.clamp(1.0, ER_MAX_MS) * 1e-3 * fs;
        for (t, f) in self.er_tap.iter_mut().zip(ER_AT) {
            *t = ((span * f).round() as usize).max(1);
        }
    }

    /// T60, damping, low cut and mixes change at once; a longer pre-delay than at `prepare` is capped.
    pub fn set_params(&mut self, p: VerbParams) {
        self.p = p;
        self.update();
        self.pre_len = self.pre_len.min(self.pre.len().saturating_sub(1));
    }

    /// The backing track for the next block (same length as it, up to `max_block`). Only its
    /// level is used. A block without it counts as silence; with no music at all the mix stays at
    /// `mix_dry` (or call `without_music`).
    pub fn set_music(&mut self, music: &[f32]) {
        let n = music.len().min(self.music.len());
        self.music[..n].copy_from_slice(&music[..n]);
        self.music_len = n;
    }

    /// Use `mix` all the time (no music input: a plain reverb).
    pub fn without_music(&mut self) {
        self.p.mix_dry = self.p.mix;
        self.mix_now = self.p.mix;
    }

    pub fn mix_now(&self) -> f64 {
        self.mix_now
    }

    /// One sample in, the wet (left, right) out.
    #[inline]
    fn tick(&mut self, x: f64) -> (f64, f64) {
        let v = if self.pre_len == 0 {
            x
        } else {
            let n = self.pre.len();
            self.pre[self.pre_pos] = x;
            self.pre_pos = (self.pre_pos + 1) % n;
            self.pre[(self.pre_pos + n - 1 - self.pre_len) % n]
        };
        let v = self.lowcut.tick(v);
        let (mut el, mut er) = (0.0, 0.0);
        if !self.er_buf.is_empty() {
            let n = self.er_buf.len();
            self.er_buf[self.er_pos] = v;
            if self.p.er > 0.0 {
                for (i, (t, g)) in self.er_tap.iter().zip(ER_GAIN).enumerate() {
                    let x = self.er_buf[(self.er_pos + n - t.min(&(n - 1))) % n] * g;
                    if i % 2 == 0 {
                        el += x;
                    } else {
                        er += x;
                    }
                }
                // the tail's tone, and a level that keeps er = 1 about as strong as the tap sum of one side
                let k = self.p.er * 0.45;
                self.er_lp[0] = (1.0 - self.damp) * el + self.damp * self.er_lp[0];
                self.er_lp[1] = (1.0 - self.damp) * er + self.damp * self.er_lp[1];
                el = self.er_lp[0] * k;
                er = self.er_lp[1] * k;
            }
            self.er_pos = (self.er_pos + 1) % n;
        }
        let mut u = [v; N];
        for (s, d) in self.diff.iter_mut().enumerate() {
            let mut o = d.read();
            d.write(&u);
            hadamard(&mut o);
            for c in 0..N {
                u[c] = o[c] * FLIPS[s][c];
            }
        }
        let out = self.lp_loop.read();
        let mut fb = [0.0; N];
        let mut sum = 0.0;
        for c in 0..N {
            self.lp[c] = (1.0 - self.damp) * out[c] + self.damp * self.lp[c];
            if self.lp[c].abs() < 1e-30 {
                self.lp[c] = 0.0;
            }
            fb[c] = self.lp[c] * self.gains[c];
            sum += fb[c];
        }
        let hh = 2.0 / N as f64 * sum;
        let mut w = [0.0; N];
        for c in 0..N {
            w[c] = fb[c] - hh + u[c];
        }
        self.lp_loop.write(&w);
        let norm = 1.0 / ((N / 2) as f64).sqrt();
        let (mut l, mut r) = (0.0, 0.0);
        for c in 0..N / 2 {
            l += w[2 * c];
            r += w[2 * c + 1];
        }
        (self.hc[0].tick(l * norm + el), self.hc[1].tick(r * norm + er))
    }

    #[inline]
    fn step_mix(&mut self, i: usize) -> f64 {
        let m = if i < self.music_len { self.music[i] as f64 } else { 0.0 };
        self.music_pow = self.music_a * self.music_pow + (1.0 - self.music_a) * m * m;
        let playing = 10.0 * crate::m::log10(self.music_pow + 1e-30) > MUSIC_ON_DB;
        let target = if playing { self.p.mix } else { self.p.mix_dry };
        let rate = (self.p.mix - self.p.mix_dry).abs() / (self.p.follow_s.max(0.01) * self.fs);
        if self.mix_now < target {
            self.mix_now = (self.mix_now + rate).min(target);
        } else if self.mix_now > target {
            self.mix_now = (self.mix_now - rate).max(target);
        }
        self.mix_now
    }

    /// Dry plus wet, in stereo (the dry is the same on both sides).
    pub fn process_stereo(&mut self, x: &[f32], left: &mut [f32], right: &mut [f32]) {
        for i in 0..x.len() {
            let v = x[i] as f64;
            let (l, r) = self.tick(v);
            let m = self.step_mix(i);
            left[i] = (v + m * l) as f32;
            right[i] = (v + m * r) as f32;
        }
        self.music_len = 0;
    }
}

impl Stage for Reverb {
    fn prepare(&mut self, fs: f64, max_block: usize) {
        self.fs = fs;
        self.music = vec![0.0; max_block.max(1)];
        self.music_len = 0;
        let mut diff = Vec::new();
        for (s, r) in STEPS_MS.iter().enumerate() {
            let rng = r * 1e-3 * fs;
            let mut d = [0usize; N];
            for (c, v) in d.iter_mut().enumerate() {
                *v = (rng * (c as f64 + FRACS[(c + 3 * s) % N]) / N as f64).round() as usize + 1;
            }
            diff.push(Lines::new(d));
        }
        self.diff = diff;
        let mut d = [0usize; N];
        for (c, v) in d.iter_mut().enumerate() {
            *v = (SIZE_MS * 1e-3 * fs * 2f64.powf(c as f64 / N as f64)).round() as usize;
        }
        self.lp_loop = Lines::new(d);
        // room for up to 100 ms of pre-delay without allocating later
        self.pre = vec![0.0; (0.1 * fs) as usize + 2];
        self.er_buf = vec![0.0; (ER_MAX_MS * 1e-3 * fs) as usize + 2];
        self.music_a = (-1.0 / (MUSIC_TAU_S * fs)).exp();
        let c = Coefs::design(
            if self.p.lowcut_hz > 0.0 { Kind::HighPass } else { Kind::Off },
            self.p.lowcut_hz,
            std::f64::consts::FRAC_1_SQRT_2,
            0.0,
            fs,
        );
        self.lowcut.set_now(c);
        self.update();
        self.pre_len = self.pre_len.min(self.pre.len() - 1);
        self.reset();
    }

    /// Mono: dry plus the mix times the average of the two wet sides.
    fn process(&mut self, x: &mut [f32]) {
        for (i, s) in x.iter_mut().enumerate() {
            let v = *s as f64;
            let (l, r) = self.tick(v);
            let m = self.step_mix(i);
            *s = (v + m * 0.5 * (l + r)) as f32;
        }
        self.music_len = 0;
    }

    fn reset(&mut self) {
        self.pre.iter_mut().for_each(|v| *v = 0.0);
        self.pre_pos = 0;
        self.lowcut.reset();
        self.hc.iter_mut().for_each(Svf::reset);
        self.diff.iter_mut().for_each(Lines::clear);
        self.lp_loop.clear();
        self.lp = [0.0; N];
        self.er_buf.iter_mut().for_each(|v| *v = 0.0);
        self.er_pos = 0;
        self.er_lp = [0.0; 2];
        self.music_pow = 0.0;
        self.mix_now = self.p.mix_dry;
    }
}

#[cfg(test)]
mod er_balance {
    use super::*;
    use crate::Stage;

    /// The early reflections' energy against the tail's (an impulse in), for turning a published preset's
    /// early/late balance into `er`: cargo test -p stream-mic-dsp er_balance -- --ignored --nocapture
    #[test]
    #[ignore]
    fn er_balance() {
        let energy = |er: f64, t60: f64| {
            let mut r = Reverb::new(VerbParams { t60_s: t60, pre_ms: 0.0, lowcut_hz: 0.0, damp_hz: 20000.0, mix: 1.0, mix_dry: 1.0, follow_s: 1.0, er, er_ms: 30.0, hc_hz: 0.0 });
            r.prepare(48000.0, 256);
            r.without_music();
            let n = 48000 * 6;
            let mut x = vec![0f32; n];
            x[0] = 1.0;
            let (mut l, mut rr) = (vec![0f32; n], vec![0f32; n]);
            for (i, c) in x.chunks(256).enumerate() {
                r.process_stereo(c, &mut l[i * 256..i * 256 + c.len()], &mut rr[i * 256..i * 256 + c.len()]);
            }
            l[0] -= 1.0;
            rr[0] -= 1.0;
            l.iter().chain(rr.iter()).map(|v| (*v as f64).powi(2)).sum::<f64>() / 2.0
        };
        for t60 in [0.5, 1.0, 2.0, 3.0] {
            let tail = energy(0.0, t60);
            let both = energy(1.0, t60);
            println!("t60 {t60}: tail {:.3}, early at er 1 {:.3} ({:.1} dB under the tail)", tail, both - tail, 10.0 * (tail / (both - tail)).log10());
        }
    }
}
