//! Low cut and four-band EQ: the "低切與 EQ" stage of the UI.
//!
//! - low cut: 2nd order Butterworth high pass (12 dB/oct)
//! - 低頻: low shelf at 200 Hz
//! - 中頻: bell, Q 1.4, moved between 200 Hz and 1 kHz (the low mids' "mud" at 250-350 Hz and "box" at 500-800 Hz;
//!   2026-11-10, ahead of the EQ styles)
//! - 清晰: bell, Q 1, moved between 2 and 6 kHz (3 kHz by default; 2026-11-10)
//! - 空氣感: high shelf at 10 kHz
//! - high cut: 2nd order Butterworth low pass (12 dB/oct), off at 20 kHz and above; stream-mic's own (the io44 has
//!   no band left for it), for styles such as a telephone or an old radio (2026-11-10)
//!
//! All four are Simper SVFs (design/dsp/eq.md). The high shelf has the bilinear transform's
//! squeeze near Nyquist; the matched (Vicanek) alternative is a listening A/B for later.

use crate::svf::{Coefs, Kind, Svf};
use crate::Stage;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqParams {
    /// Low cut corner in Hz, 0 = off.
    pub hp_hz: f64,
    pub low_db: f64,
    /// The mid bell's centre, Hz, and its gain
    pub mid_hz: f64,
    pub mid_db: f64,
    /// The presence bell's centre, Hz
    pub pres_hz: f64,
    pub pres_db: f64,
    pub air_db: f64,
    /// High cut corner in Hz; at HC_OFF_HZ or above (or 0) it is off.
    pub hc_hz: f64,
}

impl Default for EqParams {
    /// The talk preset's values.
    fn default() -> Self {
        EqParams { hp_hz: 80.0, low_db: 0.0, mid_hz: MID_HZ, mid_db: 0.0, pres_hz: PRES_HZ, pres_db: 1.5, air_db: 1.0, hc_hz: HC_OFF_HZ }
    }
}

pub const LOW_HZ: f64 = 200.0;
/// The mid bell's default centre and its Q
pub const MID_HZ: f64 = 400.0;
pub const MID_Q: f64 = 1.4;
pub const PRES_HZ: f64 = 3000.0;
pub const PRES_Q: f64 = 1.0;
/// The high cut is off at this corner and above
pub const HC_OFF_HZ: f64 = 20000.0;
pub const AIR_HZ: f64 = 10000.0;
pub const SHELF_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
pub const HP_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Parameter changes glide over this long.
pub const RAMP_S: f64 = 0.010;

#[derive(Clone, Debug)]
pub struct Eq {
    p: EqParams,
    fs: f64,
    prepared: bool,
    f: [Svf; 6],
}

impl Eq {
    pub fn new(p: EqParams) -> Self {
        Eq { p, fs: 48000.0, prepared: false, f: Default::default() }
    }

    pub fn params(&self) -> EqParams {
        self.p
    }

    fn coefs(p: &EqParams, fs: f64) -> [Coefs; 6] {
        let hc = p.hc_hz > 0.0 && p.hc_hz < HC_OFF_HZ.min(fs * 0.45);
        [
            Coefs::design(if p.hp_hz > 0.0 { Kind::HighPass } else { Kind::Off }, p.hp_hz, HP_Q, 0.0, fs),
            Coefs::design(Kind::LowShelf, LOW_HZ, SHELF_Q, p.low_db, fs),
            Coefs::design(Kind::Bell, p.mid_hz.clamp(20.0, fs * 0.45), MID_Q, p.mid_db, fs),
            Coefs::design(Kind::Bell, p.pres_hz.clamp(20.0, fs * 0.45), PRES_Q, p.pres_db, fs),
            Coefs::design(Kind::HighShelf, AIR_HZ, SHELF_Q, p.air_db, fs),
            Coefs::design(if hc { Kind::LowPass } else { Kind::Off }, p.hc_hz, HP_Q, 0.0, fs),
        ]
    }

    /// New settings take effect from the next sample processed, gliding over RAMP_S.
    pub fn set_params(&mut self, p: EqParams) {
        self.p = p;
        if !self.prepared {
            return;
        }
        let ramp = (RAMP_S * self.fs).round() as u32;
        for (f, c) in self.f.iter_mut().zip(Self::coefs(&p, self.fs)) {
            f.set_ramped(c, ramp);
        }
    }
}

impl Stage for Eq {
    fn prepare(&mut self, fs: f64, _max_block: usize) {
        self.fs = fs;
        self.prepared = true;
        for (f, c) in self.f.iter_mut().zip(Self::coefs(&self.p, fs)) {
            f.set_now(c);
            f.reset();
        }
    }

    fn process(&mut self, x: &mut [f32]) {
        for s in x.iter_mut() {
            let mut v = *s as f64;
            for f in self.f.iter_mut() {
                v = f.tick(v);
            }
            *s = v as f32;
        }
    }

    fn reset(&mut self) {
        for f in self.f.iter_mut() {
            f.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize) -> Vec<f32> {
        // xorshift, fixed seed
        let mut s: u32 = 0x1234_5678;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * 0.5
            })
            .collect()
    }

    fn run(p: EqParams, x: &[f32], block: usize, change_at: Option<(usize, EqParams)>) -> Vec<f32> {
        let mut eq = Eq::new(p);
        eq.prepare(48000.0, block);
        let mut y = x.to_vec();
        let mut pos = 0;
        for chunk in y.chunks_mut(block) {
            if let Some((at, q)) = change_at {
                if pos <= at && at < pos + chunk.len() {
                    // Split the block so the change lands on the same sample for every block size.
                    let (a, b) = chunk.split_at_mut(at - pos);
                    eq.process(a);
                    eq.set_params(q);
                    eq.process(b);
                    pos += chunk.len();
                    continue;
                }
            }
            eq.process(chunk);
            pos += chunk.len();
        }
        y
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = noise(48000);
        let p = EqParams { hp_hz: 100.0, low_db: 3.0, mid_hz: 350.0, mid_db: -3.0, pres_hz: 2500.0, pres_db: -2.0, air_db: 4.5, hc_hz: 8000.0 };
        let q = EqParams { hp_hz: 60.0, low_db: -4.0, mid_hz: 700.0, mid_db: 2.0, pres_hz: 4500.0, pres_db: 5.0, air_db: -3.0, hc_hz: 12000.0 };
        let want = run(p, &x, 1, Some((12345, q)));
        for block in [64, 128, 480, 4096] {
            assert_eq!(run(p, &x, block, Some((12345, q))), want, "block {block}");
        }
    }

    #[test]
    fn flat_settings_pass_through() {
        let x = noise(4800);
        let p = EqParams { hp_hz: 0.0, low_db: 0.0, mid_hz: MID_HZ, mid_db: 0.0, pres_hz: PRES_HZ, pres_db: 0.0, air_db: 0.0, hc_hz: HC_OFF_HZ };
        let y = run(p, &x, 128, None);
        let err = x.iter().zip(&y).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-6, "max error {err}");
    }

    #[test]
    fn shelves_reach_their_gain() {
        // DC through the low shelf = low_db; a tone near Nyquist through the high shelf = air_db.
        let fs = 48000.0;
        let c = Coefs::design(Kind::LowShelf, LOW_HZ, SHELF_Q, 6.0, fs);
        let mut f = Svf::default();
        f.set_now(c);
        let mut y = 0.0;
        for _ in 0..48000 {
            y = f.tick(1.0);
        }
        assert!((20.0 * y.log10() - 6.0).abs() < 1e-6);
        let c = Coefs::design(Kind::HighShelf, AIR_HZ, SHELF_Q, -4.0, fs);
        let mut f = Svf::default();
        f.set_now(c);
        let mut y = 0.0;
        for i in 0..48000 {
            y = f.tick(if i % 2 == 0 { 1.0 } else { -1.0 });
        }
        assert!((20.0 * y.abs().log10() + 4.0).abs() < 1e-6);
    }

    #[test]
    fn the_high_cut_cuts_and_is_off_at_its_top() {
        // a 10 kHz tone through a 2 kHz high cut: about -28 dB (12 dB/oct, 2.3 octaves); at 20 kHz it passes as is
        let fs = 48000.0;
        let x: Vec<f32> = (0..9600).map(|i| (2.0 * std::f64::consts::PI * 10000.0 * i as f64 / fs).sin() as f32 * 0.5).collect();
        let flat = EqParams { hp_hz: 0.0, low_db: 0.0, mid_hz: MID_HZ, mid_db: 0.0, pres_hz: PRES_HZ, pres_db: 0.0, air_db: 0.0, hc_hz: HC_OFF_HZ };
        let rms = |y: &[f32]| (y[4800..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 4800.0).sqrt();
        let a = rms(&run(flat, &x, 128, None));
        let b = rms(&run(EqParams { hc_hz: 2000.0, ..flat }, &x, 128, None));
        let db = 20.0 * (b / a).log10();
        assert!((-31.0..-25.0).contains(&db), "high cut at 10 kHz: {db} dB");
        assert!((a - 0.5 / 2f64.sqrt()).abs() < 1e-3, "off: {a}");
    }

    #[test]
    fn a_parameter_jump_has_no_click() {
        // A steady 1 kHz tone; switching the bell from -6 to +6 dB must not jump the waveform more
        // than the tone itself moves from one sample to the next.
        let fs = 48000.0;
        let x: Vec<f32> = (0..9600).map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / fs).sin() as f32 * 0.5).collect();
        let p = EqParams { hp_hz: 80.0, low_db: 0.0, mid_hz: MID_HZ, mid_db: 0.0, pres_hz: PRES_HZ, pres_db: -6.0, air_db: 0.0, hc_hz: HC_OFF_HZ };
        let q = EqParams { pres_db: 6.0, ..p };
        let y = run(p, &x, 128, Some((4800, q)));
        let max_step = y.windows(2).skip(2400).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        // 1 kHz at amplitude 1.0 (after +6 dB) moves at most 2π·1000/48000 ≈ 0.131 per sample.
        assert!(max_step < 0.14, "max step {max_step}");
    }
}
