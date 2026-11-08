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
}

impl Default for VerbParams {
    /// The singing preset.
    fn default() -> Self {
        VerbParams { t60_s: 1.6, pre_ms: 10.0, lowcut_hz: 250.0, damp_hz: 7000.0, mix: 0.14, mix_dry: 0.04, follow_s: 1.5 }
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
    diff: Vec<Lines>,
    lp_loop: Lines,
    gains: [f64; N],
    damp: f64,
    lp: [f64; N],
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
            diff: Vec::new(),
            lp_loop: Lines::default(),
            gains: [0.0; N],
            damp: 0.0,
            lp: [0.0; N],
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
        self.pre_len = (self.p.pre_ms.max(0.0) * 1e-3 * fs).round() as usize;
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
        (l * norm, r * norm)
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
        self.diff.iter_mut().for_each(Lines::clear);
        self.lp_loop.clear();
        self.lp = [0.0; N];
        self.music_pow = 0.0;
        self.mix_now = self.p.mix_dry;
    }
}
