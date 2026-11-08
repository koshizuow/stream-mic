//! Simper's state variable filter (trapezoidal integration, Cytomic 2013/2016; design/dsp/eq.md).
//! Same frequency response as the RBJ cookbook biquads, but coefficients can move every sample
//! without clicks, so parameter changes ramp linearly over a fixed time.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Pass through (the stage is off).
    Off,
    HighPass,
    LowPass,
    /// Band pass with 0 dB at the centre (RBJ "constant 0 dB peak gain").
    BandPass,
    LowShelf,
    HighShelf,
    Bell,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coefs {
    a1: f64,
    a2: f64,
    a3: f64,
    m0: f64,
    m1: f64,
    m2: f64,
}

impl Coefs {
    pub const IDENTITY: Coefs = Coefs { a1: 0.0, a2: 0.0, a3: 0.0, m0: 1.0, m1: 0.0, m2: 0.0 };

    /// `freq` in Hz, `q` the cookbook Q, `gain_db` for shelves and bells.
    pub fn design(kind: Kind, freq: f64, q: f64, gain_db: f64, fs: f64) -> Coefs {
        if kind == Kind::Off || freq <= 0.0 {
            return Coefs::IDENTITY;
        }
        // Keep the corner below Nyquist; tan() blows up at fs/2.
        let w = PI * freq.min(0.49 * fs) / fs;
        let a = 10f64.powf(gain_db / 40.0);
        let mut k = 1.0 / q;
        let mut g = w.tan();
        let (m0, m1, m2);
        match kind {
            Kind::Off => unreachable!(),
            Kind::HighPass => {
                (m0, m1, m2) = (1.0, -k, -1.0);
            }
            Kind::LowPass => {
                (m0, m1, m2) = (0.0, 0.0, 1.0);
            }
            Kind::BandPass => {
                (m0, m1, m2) = (0.0, k, 0.0);
            }
            Kind::Bell => {
                k = 1.0 / (q * a);
                (m0, m1, m2) = (1.0, k * (a * a - 1.0), 0.0);
            }
            Kind::LowShelf => {
                g /= a.sqrt();
                (m0, m1, m2) = (1.0, k * (a - 1.0), a * a - 1.0);
            }
            Kind::HighShelf => {
                g *= a.sqrt();
                (m0, m1, m2) = (a * a, k * (1.0 - a) * a, 1.0 - a * a);
            }
        }
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Coefs { a1, a2, a3, m0, m1, m2 }
    }

    fn lerp_step(from: &Coefs, to: &Coefs, n: u32) -> Coefs {
        let d = 1.0 / n as f64;
        Coefs {
            a1: (to.a1 - from.a1) * d,
            a2: (to.a2 - from.a2) * d,
            a3: (to.a3 - from.a3) * d,
            m0: (to.m0 - from.m0) * d,
            m1: (to.m1 - from.m1) * d,
            m2: (to.m2 - from.m2) * d,
        }
    }

    fn add(&mut self, s: &Coefs) {
        self.a1 += s.a1;
        self.a2 += s.a2;
        self.a3 += s.a3;
        self.m0 += s.m0;
        self.m1 += s.m1;
        self.m2 += s.m2;
    }
}

#[derive(Clone, Debug)]
pub struct Svf {
    c: Coefs,
    target: Coefs,
    step: Coefs,
    remain: u32,
    ic1: f64,
    ic2: f64,
}

impl Default for Svf {
    fn default() -> Self {
        Svf { c: Coefs::IDENTITY, target: Coefs::IDENTITY, step: Coefs::default(), remain: 0, ic1: 0.0, ic2: 0.0 }
    }
}

impl Svf {
    /// Jump to `c` at once (before the first block, or after a reset).
    pub fn set_now(&mut self, c: Coefs) {
        self.c = c;
        self.target = c;
        self.remain = 0;
    }

    /// Move to `c` linearly over `ramp` samples.
    pub fn set_ramped(&mut self, c: Coefs, ramp: u32) {
        if c == self.target {
            return;
        }
        if ramp == 0 {
            return self.set_now(c);
        }
        self.target = c;
        self.step = Coefs::lerp_step(&self.c, &c, ramp);
        self.remain = ramp;
    }

    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, v0: f64) -> f64 {
        if self.remain > 0 {
            self.remain -= 1;
            if self.remain == 0 {
                self.c = self.target;
            } else {
                self.c.add(&self.step);
            }
        }
        let c = &self.c;
        let v3 = v0 - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        // Silence decays the state into denormals, which are slow on x86.
        if self.ic1.abs() < 1e-30 {
            self.ic1 = 0.0;
        }
        if self.ic2.abs() < 1e-30 {
            self.ic2 = 0.0;
        }
        c.m0 * v0 + c.m1 * v1 + c.m2 * v2
    }
}
