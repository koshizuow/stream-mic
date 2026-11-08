//! De-esser: a dynamic bell with a tilt detector (design/dsp/deesser.md).
//!
//!   s   = band pass at `freq` (Q 0.8, 0 dB at the centre)
//!   y   = x + (g - 1) * s                 g = 1 is exactly the input: nothing moves between s sounds
//!   det = level(s) - level(low pass 3 kHz of x)   (power, attack 1 ms, release 15 ms, in dB)
//!   g   = -(det - thr) * (1 - 1/ratio) dB above the threshold, at most `max_db`
//!
//! The detector compares the s band with the voice below 3 kHz, so it works the same whether the
//! person speaks loud or soft, and vowels (never louder up high than below 3 kHz) do not trigger
//! it. Matches tools/deess_bench.py `ref_deess(mode="tilt")`.

use crate::svf::{Coefs, Kind, Svf};
use crate::Stage;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeessParams {
    /// How much louder (dB) the s band may be than the voice below 3 kHz before it is pulled down.
    pub thr_db: f64,
    pub freq_hz: f64,
    pub q: f64,
    pub ratio: f64,
    pub max_db: f64,
    pub att_ms: f64,
    pub rel_ms: f64,
}

impl Default for DeessParams {
    fn default() -> Self {
        DeessParams { thr_db: 6.0, freq_hz: 7000.0, q: 0.8, ratio: 10.0, max_db: 12.0, att_ms: 1.0, rel_ms: 15.0 }
    }
}

pub const TILT_HZ: f64 = 3000.0;

#[derive(Clone, Debug)]
pub struct Deesser {
    p: DeessParams,
    fs: f64,
    prepared: bool,
    bp: Svf,
    lp: Svf,
    aa: f64,
    ar: f64,
    eb: f64,
    el: f64,
    gr: f64,
}

fn pole(ms: f64, fs: f64) -> f64 {
    (-1.0 / (ms.max(1e-3) * 1e-3 * fs)).exp()
}

impl Deesser {
    pub fn new(p: DeessParams) -> Self {
        Deesser { p, fs: 48000.0, prepared: false, bp: Svf::default(), lp: Svf::default(), aa: 0.0, ar: 0.0, eb: 0.0, el: 0.0, gr: 0.0 }
    }

    fn coefs(&self) -> (Coefs, Coefs) {
        (
            Coefs::design(Kind::BandPass, self.p.freq_hz, self.p.q, 0.0, self.fs),
            Coefs::design(Kind::LowPass, TILT_HZ, std::f64::consts::FRAC_1_SQRT_2, 0.0, self.fs),
        )
    }

    pub fn set_params(&mut self, p: DeessParams) {
        self.p = p;
        self.aa = pole(p.att_ms, self.fs);
        self.ar = pole(p.rel_ms, self.fs);
        if self.prepared {
            let (b, _) = self.coefs();
            self.bp.set_ramped(b, (0.010 * self.fs).round() as u32);
        }
    }

    /// Current reduction at the centre frequency, dB (<= 0).
    pub fn reduction_db(&self) -> f64 {
        -self.gr
    }
}

#[inline]
fn follow(e: f64, v: f64, aa: f64, ar: f64) -> f64 {
    if v > e {
        aa * e + (1.0 - aa) * v
    } else {
        ar * e + (1.0 - ar) * v
    }
}

impl Stage for Deesser {
    fn prepare(&mut self, fs: f64, _max_block: usize) {
        self.fs = fs;
        self.prepared = true;
        let (b, l) = self.coefs();
        self.bp.set_now(b);
        self.lp.set_now(l);
        self.set_params(self.p);
        self.reset();
    }

    fn process(&mut self, x: &mut [f32]) {
        let p = self.p;
        let k = 1.0 - 1.0 / p.ratio;
        for v in x.iter_mut() {
            let xv = *v as f64;
            let s = self.bp.tick(xv);
            let l = self.lp.tick(xv);
            self.eb = follow(self.eb, s * s, self.aa, self.ar);
            self.el = follow(self.el, l * l, self.aa, self.ar);
            let over = 10.0 * crate::m::log10(self.eb + 1e-20) - 10.0 * crate::m::log10(self.el + 1e-20) - p.thr_db;
            self.gr = (over.max(0.0) * k).min(p.max_db);
            let g = crate::m::undb(-self.gr);
            *v = (xv + (g - 1.0) * s) as f32;
        }
    }

    fn reset(&mut self) {
        self.bp.reset();
        self.lp.reset();
        self.eb = 0.0;
        self.el = 0.0;
        self.gr = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vowel_passes_and_an_s_is_pulled_down() {
        let fs = 48000.0;
        let tone = |f: f64, a: f64| -> Vec<f32> { (0..9600).map(|i| (a * (2.0 * std::f64::consts::PI * f * i as f64 / fs).sin()) as f32).collect() };
        // 300 Hz "vowel" at any level: untouched
        for a in [0.5, 0.01] {
            let mut d = Deesser::new(DeessParams::default());
            d.prepare(fs, 9600);
            let mut x = tone(300.0, a);
            let x0 = x.clone();
            d.process(&mut x);
            assert_eq!(d.reduction_db(), 0.0);
            // after the first 20 ms (at the very start the low pass level is still near zero)
            let err = x.iter().zip(&x0).skip(960).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(err < 1e-6);
        }
        // 7 kHz "s": pulled down by the same amount at any level. A pure 7 kHz tone is still about
        // -15 dB in the 3 kHz low pass, so it is 15 - 6 dB over: (9 dB) * 0.9 plus ripple.
        let mut got = Vec::new();
        for a in [0.5, 0.01] {
            let mut d = Deesser::new(DeessParams::default());
            d.prepare(fs, 9600);
            let mut x = tone(7000.0, a);
            d.process(&mut x);
            got.push(d.reduction_db());
        }
        assert!(got[0] < -6.0 && got[0] > -12.0, "{got:?}");
        assert!((got[0] - got[1]).abs() < 1e-6, "{got:?}");
    }
}
