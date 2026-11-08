//! Noise reduction: DeepFilterNet3's low latency model run through libDF (tract), embedded instead
//! of the LADSPA plugin, which measured 30 ms (design/measurements.md).
//!
//! The model works on 480-sample hops (10 ms at 48 kHz) and its output lags its input by
//! fft_size - hop = 480 samples. Feeding it a stream of any block size needs one more hop of
//! buffering, so the stage's latency is 960 samples (20 ms). Samples go in and out one at a time,
//! so the output does not depend on the block size.
//!
//! `process` runs the model inline when a hop is complete, which takes milliseconds and allocates
//! inside tract: fine offline (cli), but the real-time engine must run it on its own thread.

use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;
use stream_mic_dsp::Stage;

pub const FS: f64 = 48000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenoiseParams {
    /// At most this much reduction (dB); 0 = off (the signal is only delayed, the model is not run).
    pub limit_db: f64,
}

impl Default for DenoiseParams {
    fn default() -> Self {
        DenoiseParams { limit_db: 24.0 }
    }
}

pub struct Denoise {
    p: DenoiseParams,
    dfp: DfParams,
    df: DfTract,
    /// No frame processed since the model was built (so `reset` need not rebuild it).
    fresh: bool,
    hop: usize,
    model_delay: usize,
    inbuf: Array2<f32>,
    outbuf: Array2<f32>,
    fill: usize,
    /// Output FIFO: one hop of buffering, then the model's frames.
    fifo: Vec<f32>,
    rd: usize,
    wr: usize,
    /// Plain delay used while off, so switching does not change the latency.
    bypass: Vec<f32>,
    bpos: usize,
    /// Last frame's local SNR estimate from the model (dB).
    pub lsnr: f32,
}

impl Denoise {
    pub fn new(p: DenoiseParams) -> Result<Self, String> {
        let dfp = DfParams::default();
        let df = Self::build(&dfp, p.limit_db)?;
        let hop = df.hop_size;
        let model_delay = df.fft_size - df.hop_size + df.lookahead * df.hop_size;
        let lat = hop + model_delay;
        Ok(Denoise {
            p,
            dfp,
            df,
            fresh: true,
            hop,
            model_delay,
            inbuf: Array2::zeros((1, hop)),
            outbuf: Array2::zeros((1, hop)),
            fill: 0,
            fifo: vec![0.0; 2 * hop],
            rd: 0,
            wr: hop,
            bypass: vec![0.0; lat],
            bpos: 0,
            lsnr: 0.0,
        })
    }

    fn build(dfp: &DfParams, limit_db: f64) -> Result<DfTract, String> {
        // The deep-filter program's thresholds (local SNR in dB: below -15 the decoders are skipped,
        // above 35 the ERB and DF stages are). The quality in measurements.md was measured with these;
        // libDF's defaults (-10, 30, 20), which the LADSPA plugin uses, skip the DF stage sooner.
        // STREAM_MIC_DF_THRESH="min,erb,df" overrides them (measuring CPU against quality)
        let (a, b, c) = std::env::var("STREAM_MIC_DF_THRESH")
            .ok()
            .and_then(|v| {
                let t: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (t.len() == 3).then(|| (t[0], t[1], t[2]))
            })
            .unwrap_or((-15.0, 35.0, 35.0));
        let rp = RuntimeParams::default_with_ch(1).with_atten_lim(Self::lim(limit_db)).with_thresholds(a, b, c);
        DfTract::new(dfp.clone(), &rp).map_err(|e| format!("DeepFilterNet: {e}"))
    }

    fn lim(db: f64) -> f32 {
        // libDF: >= 100 means no limit, < 0.01 means pass through (undelayed, which we never want)
        db.clamp(0.5, 100.0) as f32
    }

    pub fn set_params(&mut self, p: DenoiseParams) {
        if p.limit_db > 0.0 {
            self.df.set_atten_lim(Self::lim(p.limit_db));
        }
        self.p = p;
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    pub fn model_delay(&self) -> usize {
        self.model_delay
    }
}

impl Stage for Denoise {
    fn prepare(&mut self, fs: f64, _max_block: usize) {
        assert_eq!(fs, FS, "DeepFilterNet3 runs at 48 kHz only");
        self.reset();
    }

    fn process(&mut self, x: &mut [f32]) {
        let n = self.fifo.len();
        let lat = self.bypass.len();
        for s in x.iter_mut() {
            let v = *s;
            // the bypass delay runs all the time, so turning denoise off mid-stream keeps the timing
            let old = self.bypass[self.bpos];
            self.bypass[self.bpos] = v;
            self.bpos = (self.bpos + 1) % lat;
            self.inbuf[[0, self.fill]] = v;
            self.fill += 1;
            if self.fill == self.hop {
                self.fill = 0;
                if self.p.limit_db > 0.0 {
                    self.fresh = false;
                    match self.df.process(self.inbuf.view(), self.outbuf.view_mut()) {
                        Ok(l) => self.lsnr = l,
                        Err(_) => self.outbuf.fill(0.0),
                    }
                }
                for i in 0..self.hop {
                    self.fifo[self.wr] = self.outbuf[[0, i]];
                    self.wr = (self.wr + 1) % n;
                }
            }
            let y = self.fifo[self.rd];
            self.rd = (self.rd + 1) % n;
            *s = if self.p.limit_db > 0.0 { y } else { old };
        }
    }

    fn reset(&mut self) {
        self.fill = 0;
        self.fifo.iter_mut().for_each(|v| *v = 0.0);
        self.rd = 0;
        self.wr = self.hop;
        self.bypass.iter_mut().for_each(|v| *v = 0.0);
        self.bpos = 0;
        // libDF's init() may only run once (it refills one of its rolling buffers without clearing
        // it first, v0.5.6), so a fresh state means a new model. This allocates: not for the audio thread.
        if !self.fresh {
            if let Ok(df) = Self::build(&self.dfp, self.p.limit_db) {
                self.df = df;
                self.fresh = true;
            }
        }
    }

    fn latency(&self) -> usize {
        self.hop + self.model_delay
    }
}
