//! The math the per-sample paths use, as fast on Windows as on Linux. A mingw build takes exp and pow from Windows'
//! msvcrt, which took 39 and 67 ns where glibc takes 6 and 16 (a Windows and a Linux test PC, 2026-11-07); libm (pure Rust, a
//! port of musl's) takes 5 for exp there. On Linux the system's stay (libm's pow is three times glibc's).
//!
//! 10^(dB/20) is exp(dB · ln10/20): the same number, without pow, faster on both.

use std::f64::consts::LN_10;

#[inline]
pub fn exp(x: f64) -> f64 {
    #[cfg(windows)]
    {
        libm::exp(x)
    }
    #[cfg(not(windows))]
    {
        x.exp()
    }
}

#[inline]
pub fn log10(x: f64) -> f64 {
    #[cfg(windows)]
    {
        libm::log10(x)
    }
    #[cfg(not(windows))]
    {
        x.log10()
    }
}

/// dB to a gain: 10^(d/20)
#[inline]
pub fn undb(d: f64) -> f64 {
    exp(d * (LN_10 / 20.0))
}

#[cfg(test)]
mod tests {
    #[test]
    fn undb_is_ten_to_the_twentieth() {
        for d in [-120.0, -60.0, -6.0, 0.0, 3.5, 24.0] {
            let a = 10f64.powf(d / 20.0);
            assert!((super::undb(d) - a).abs() <= a * 1e-14, "{d}");
        }
    }
}
