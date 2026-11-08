//! Compressor and downward expander (design/dsp/dynamics.md, measurements.md).
//!
//! Compressor: Giannoulis, Massberg, Reiss 2012, the design they recommend: feed-forward, level in
//! dB, gain computer first, then a smooth decoupled peak detector on the gain reduction. Matches
//! tools/refcomp.py (and Calf MonoCompressor). Optional sidechain high pass so a 100 Hz voice
//! fundamental does not make the gain follow the waveform.
//!
//! Expander: the detector only says how loud it is; how fast the gain opens and closes is set on
//! the gain itself, in dB per time, with a hold. Plugin expanders close later the louder you spoke
//! (the detector falls from your level to the threshold), measured 170 to 480 ms for "100 ms".
//! Matches tools/refexp.py.

use crate::svf::{Coefs, Kind, Svf};
use crate::Stage;

const LOG_EPS: f64 = 1e-12;

#[inline]
fn db(x: f64) -> f64 {
    20.0 * crate::m::log10(x.abs() + LOG_EPS)
}

#[inline]
fn undb(d: f64) -> f64 {
    crate::m::undb(d)
}

/// One-pole coefficient for time constant `ms` (reaches 1 - 1/e after `ms`).
fn alpha(ms: f64, fs: f64) -> f64 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 1e-3 * fs)).exp()
    }
}

/// Giannoulis eq. 4: output level for input level `x` (dB), threshold `t`, ratio `r`, knee width `w`.
pub fn compress_curve(x: f64, t: f64, r: f64, w: f64) -> f64 {
    let over = 2.0 * (x - t);
    if over < -w {
        x
    } else if over <= w && w > 0.0 {
        x + (1.0 / r - 1.0) * (x - t + w / 2.0).powi(2) / (2.0 * w)
    } else {
        t + (x - t) / r
    }
}

/// Expander gain (dB, <= 0) for level `x`: below `t` the gain falls (r - 1) dB per dB, with a
/// quadratic knee of width `w` centred on `t`, and never below -`range`.
pub fn expand_gain(x: f64, t: f64, r: f64, w: f64, range: f64) -> f64 {
    let d = x - t;
    let g = if 2.0 * d > w {
        0.0
    } else if 2.0 * d >= -w && w > 0.0 {
        -(r - 1.0) * (d - w / 2.0).powi(2) / (2.0 * w)
    } else {
        (r - 1.0) * d
    };
    g.max(-range)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompParams {
    pub thr_db: f64,
    pub ratio: f64,
    pub knee_db: f64,
    pub att_ms: f64,
    pub rel_ms: f64,
    pub makeup_db: f64,
    /// Sidechain high pass corner, 0 = off.
    pub sc_hp_hz: f64,
}

impl Default for CompParams {
    /// The talk preset.
    fn default() -> Self {
        CompParams { thr_db: -24.0, ratio: 3.0, knee_db: 6.0, att_ms: 10.0, rel_ms: 120.0, makeup_db: 6.0, sc_hp_hz: 100.0 }
    }
}

#[derive(Clone, Debug)]
pub struct Compressor {
    p: CompParams,
    fs: f64,
    aa: f64,
    ar: f64,
    sc: Svf,
    y1: f64,
    yl: f64,
}

impl Compressor {
    pub fn new(p: CompParams) -> Self {
        let mut c = Compressor { p, fs: 48000.0, aa: 0.0, ar: 0.0, sc: Svf::default(), y1: 0.0, yl: 0.0 };
        c.update();
        c
    }

    fn update(&mut self) {
        self.aa = alpha(self.p.att_ms, self.fs);
        self.ar = alpha(self.p.rel_ms, self.fs);
        let kind = if self.p.sc_hp_hz > 0.0 { Kind::HighPass } else { Kind::Off };
        // The detector state carries over, so a change takes effect smoothly on its own.
        self.sc.set_now(Coefs::design(kind, self.p.sc_hp_hz, std::f64::consts::FRAC_1_SQRT_2, 0.0, self.fs));
    }

    pub fn set_params(&mut self, p: CompParams) {
        self.p = p;
        self.update();
    }

    /// Current gain reduction in dB (<= 0), for the meter.
    pub fn reduction_db(&self) -> f64 {
        -self.yl
    }
}

impl Stage for Compressor {
    fn prepare(&mut self, fs: f64, _max_block: usize) {
        self.fs = fs;
        self.update();
        self.reset();
    }

    fn process(&mut self, x: &mut [f32]) {
        let p = self.p;
        for s in x.iter_mut() {
            let v = *s as f64;
            let xg = db(self.sc.tick(v));
            let xl = xg - compress_curve(xg, p.thr_db, p.ratio, p.knee_db);
            self.y1 = xl.max(self.ar * self.y1 + (1.0 - self.ar) * xl);
            self.yl = self.aa * self.yl + (1.0 - self.aa) * self.y1;
            *s = (v * undb(p.makeup_db - self.yl)) as f32;
        }
    }

    fn reset(&mut self) {
        self.sc.reset();
        self.y1 = 0.0;
        self.yl = 0.0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpParams {
    pub thr_db: f64,
    /// Slope below the threshold is 1:ratio (gain falls ratio - 1 dB per dB).
    pub ratio: f64,
    pub range_db: f64,
    pub knee_db: f64,
    /// Time to open across the whole range.
    pub att_ms: f64,
    /// How long the gain stays open after the level last needed it.
    pub hold_ms: f64,
    /// Time to close across the whole range.
    pub rel_ms: f64,
    /// Level detector's peak decay.
    pub det_ms: f64,
    pub sc_hp_hz: f64,
}

impl Default for ExpParams {
    /// The talk preset.
    fn default() -> Self {
        ExpParams { thr_db: -54.0, ratio: 2.5, range_db: 12.0, knee_db: 10.0, att_ms: 3.0, hold_ms: 80.0, rel_ms: 200.0, det_ms: 10.0, sc_hp_hz: 100.0 }
    }
}

#[derive(Clone, Debug)]
pub struct Expander {
    p: ExpParams,
    fs: f64,
    ad: f64,
    up: f64,
    down: f64,
    hold_n: u32,
    sc: Svf,
    env: f64,
    g: f64,
    hold: u32,
}

impl Expander {
    pub fn new(p: ExpParams) -> Self {
        let mut e = Expander { p, fs: 48000.0, ad: 0.0, up: 0.0, down: 0.0, hold_n: 0, sc: Svf::default(), env: 0.0, g: 0.0, hold: 0 };
        e.update();
        e
    }

    fn update(&mut self) {
        let per_ms = 1e-3 * self.fs;
        self.ad = alpha(self.p.det_ms, self.fs);
        self.up = self.p.range_db / (self.p.att_ms.max(0.01) * per_ms);
        self.down = self.p.range_db / (self.p.rel_ms.max(0.01) * per_ms);
        self.hold_n = (self.p.hold_ms * per_ms).round() as u32;
        let kind = if self.p.sc_hp_hz > 0.0 { Kind::HighPass } else { Kind::Off };
        self.sc.set_now(Coefs::design(kind, self.p.sc_hp_hz, std::f64::consts::FRAC_1_SQRT_2, 0.0, self.fs));
    }

    pub fn set_params(&mut self, p: ExpParams) {
        self.p = p;
        self.update();
    }

    pub fn reduction_db(&self) -> f64 {
        self.g
    }
}

impl Stage for Expander {
    fn prepare(&mut self, fs: f64, _max_block: usize) {
        self.fs = fs;
        self.update();
        self.reset();
    }

    fn process(&mut self, x: &mut [f32]) {
        let p = self.p;
        for s in x.iter_mut() {
            let v = *s as f64;
            self.env = self.sc.tick(v).abs().max(self.ad * self.env);
            let target = expand_gain(db(self.env), p.thr_db, p.ratio, p.knee_db, p.range_db);
            if target >= self.g {
                self.g = target.min(self.g + self.up);
                self.hold = self.hold_n;
            } else if self.hold > 0 {
                self.hold -= 1;
            } else {
                self.g = target.max(self.g - self.down);
            }
            *s = (v * undb(self.g)) as f32;
        }
    }

    fn reset(&mut self) {
        self.sc.reset();
        self.env = 0.0;
        // Start open: the first words must not fade in.
        self.g = 0.0;
        self.hold = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speechish(n: usize) -> Vec<f32> {
        let mut s: u32 = 0x1234_5678;
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let amp = [0.002, 0.5, 0.05, 0.9, 0.001][(i / 7000) % 5];
                (s as f32 / u32::MAX as f32 - 0.5) * amp
            })
            .collect()
    }

    fn run<S: Stage>(mut st: S, x: &[f32], block: usize) -> Vec<f32> {
        st.prepare(48000.0, block);
        let mut y = x.to_vec();
        for c in y.chunks_mut(block) {
            st.process(c);
        }
        y
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = speechish(96000);
        let c = run(Compressor::new(CompParams::default()), &x, 1);
        let e = run(Expander::new(ExpParams::default()), &x, 1);
        for b in [64, 128, 480, 4096] {
            assert_eq!(run(Compressor::new(CompParams::default()), &x, b), c, "comp block {b}");
            assert_eq!(run(Expander::new(ExpParams::default()), &x, b), e, "exp block {b}");
        }
    }

    #[test]
    fn curves() {
        assert_eq!(compress_curve(-40.0, -30.0, 4.0, 0.0), -40.0);
        assert_eq!(compress_curve(-10.0, -30.0, 4.0, 0.0), -25.0);
        assert_eq!(expand_gain(-20.0, -50.0, 2.0, 10.0, 12.0), 0.0);
        assert_eq!(expand_gain(-60.0, -50.0, 2.0, 0.0, 12.0), -10.0);
        assert_eq!(expand_gain(-90.0, -50.0, 2.0, 10.0, 12.0), -12.0);
        // the knee meets both straight parts
        assert!((expand_gain(-55.0, -50.0, 2.0, 10.0, 40.0) + 5.0).abs() < 1e-12);
        assert!(expand_gain(-45.0, -50.0, 2.0, 10.0, 40.0).abs() < 1e-12);
    }

    #[test]
    fn expander_closes_at_the_set_speed_whatever_the_level() {
        // Loud or quiet speech, then silence: the gain reaches -range after hold + rel either way.
        let p = ExpParams { thr_db: -50.0, range_db: 12.0, hold_ms: 50.0, rel_ms: 200.0, ..Default::default() };
        for amp in [0.9f32, 0.02] {
            let mut x = vec![0.0f32; 48000];
            for (i, v) in x.iter_mut().take(9600).enumerate() {
                *v = amp * ((i as f32) * 0.13).sin();
            }
            let mut e = Expander::new(p);
            e.prepare(48000.0, 48000);
            e.process(&mut x[..9600]);
            assert!(e.reduction_db() > -0.01);
            // detector fall to the knee + 50 ms hold + 200 ms close. The detector falls 0.87 dB/ms
            // (10 ms peak decay), so -1 dBFS speech takes about 50 ms to reach the knee and
            // -34 dBFS about 13 ms: loud speech closes about 40 ms later. Plugins: 170 to 480 ms.
            e.process(&mut x[9600..9600 + 48 * 255]);
            assert!(e.reduction_db() > -12.0, "amp {amp}: closed too early");
            e.process(&mut x[9600 + 48 * 255..9600 + 48 * 310]);
            assert!((e.reduction_db() + 12.0).abs() < 1e-9, "amp {amp}: {}", e.reduction_db());
        }
    }
}
