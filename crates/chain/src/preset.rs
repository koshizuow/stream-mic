//! Presets (build-plan.md step 5): a TOML file with the four knobs' positions and every stage's parameters, in the
//! units the settings page shows. The parameters as written are what plays; turning a knob rewrites the ones it
//! stands for (the mapping that was in ui/index.html as a demo lives here now).

use serde::{Deserialize, Serialize};
use stream_mic_denoise::DenoiseParams;
use stream_mic_dsp::deesser::DeessParams;
use stream_mic_dsp::dynamics::{CompParams, ExpParams, GrCurve, GR_POINTS};
use crate::opto_table as ot;
use stream_mic_dsp::eq::EqParams;
use stream_mic_dsp::limiter::LimParams;
use stream_mic_dsp::reverb::VerbParams;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Knobs {
    /// 0 off .. 100 strong
    pub denoise: f64,
    /// -100 thin .. 0 as is .. 100 full
    pub body: f64,
    /// 0 off .. 100 (with music, for singing)
    pub space: f64,
    /// 0 off .. 100 strong
    pub sib: f64,
    /// the expander's threshold, dBFS: below it the pauses are turned down. -80 off .. -20. The room's noise floor
    /// decides it (a fixed -54 sat under most rooms' noise and never closed), so the user sets it, auto setup later
    /// (user 2026-11-07)
    pub gate: f64,
}

impl Default for Knobs {
    fn default() -> Self {
        Knobs { denoise: 0.0, body: 0.0, space: 0.0, sib: 50.0, gate: -54.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct On {
    pub denoise: bool,
    pub lowcut: bool,
    pub eq: bool,
    pub deess: bool,
    pub exp: bool,
    pub comp: bool,
    pub verb: bool,
    pub lim: bool,
}

impl Default for On {
    fn default() -> Self {
        On { denoise: false, lowcut: true, eq: true, deess: true, exp: true, comp: true, verb: false, lim: true }
    }
}

impl On {
    /// What the monitor (the headphones) gets by default: every stage that works sample by sample, so no delay. Not
    /// denoise (20 ms and more) and not the limiter's look-ahead (2 ms): a soft clipper guards the ears instead.
    pub fn monitor_default() -> Self {
        On { denoise: false, lowcut: true, eq: true, deess: true, exp: true, comp: true, verb: true, lim: false }
    }

    /// on in both
    pub fn and(&self, o: &On) -> On {
        On {
            denoise: self.denoise && o.denoise,
            lowcut: self.lowcut && o.lowcut,
            eq: self.eq && o.eq,
            deess: self.deess && o.deess,
            exp: self.exp && o.exp,
            comp: self.comp && o.comp,
            verb: self.verb && o.verb,
            lim: self.lim && o.lim,
        }
    }
}

fn monitor_default() -> On {
    On::monitor_default()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DenoiseP {
    /// at most this much reduction, dB
    pub limit: f64,
}

impl Default for DenoiseP {
    fn default() -> Self {
        DenoiseP { limit: 6.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LowCutP {
    /// the corner, Hz
    pub freq: f64,
    /// the corner with the Body knob at 0 or above (thinner turns raise it)
    pub base: f64,
}

impl Default for LowCutP {
    fn default() -> Self {
        LowCutP { freq: 80.0, base: 80.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EqP {
    /// the style the values came from (EQ_STYLES), "" when they were changed by hand
    pub style: String,
    pub low: f64,
    /// the mid bell's gain, dB, and its centre, Hz (200..1000)
    pub mid: f64,
    pub mid_f: f64,
    /// the mid and presence bells' Q (0.5..4)
    pub mid_q: f64,
    pub pres: f64,
    /// the presence bell's centre, Hz (2000..6000)
    pub pres_f: f64,
    pub pres_q: f64,
    pub air: f64,
    /// the high cut, Hz; 20000 is off
    pub hc: f64,
    /// Low's and Presence's gain with the Body knob at 0 (the style's; the knob adds to them)
    pub low_base: f64,
    pub pres_base: f64,
}

impl Default for EqP {
    fn default() -> Self {
        EqP { style: String::new(), low: 0.0, mid: 0.0, mid_f: 400.0, mid_q: 1.4, pres: 1.5, pres_f: 3000.0, pres_q: 1.0, air: 1.0, hc: 20000.0, low_base: 0.0, pres_base: 1.5 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeessP {
    pub thr: f64,
    pub freq: f64,
    pub max: f64,
}

impl Default for DeessP {
    fn default() -> Self {
        DeessP { thr: 7.0, freq: 7000.0, max: 12.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExpP {
    pub thr: f64,
    pub ratio: f64,
    pub range: f64,
    pub hold: f64,
    pub rel: f64,
}

impl Default for ExpP {
    fn default() -> Self {
        ExpP { thr: -54.0, ratio: 2.5, range: 12.0, hold: 80.0, rel: 200.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompP {
    /// the style the values came from (COMP_STYLES), "" when they were changed by hand
    pub style: String,
    pub thr: f64,
    pub ratio: f64,
    pub knee: f64,
    pub att: f64,
    pub rel: f64,
    pub makeup: f64,
    /// a slow second release stage, ms; 0 = one stage only
    pub rel2: f64,
    /// the slow stage's share of the release, %
    pub slow: f64,
    /// after long reduction the slow stage gets up to this much slower, %
    pub memory: f64,
    /// the attack halves for every 5 dB of reduction (Attack is then its time at none)
    pub auto_att: bool,
    /// the two-knob optical mode: Amount and Makeup (with Compress/Limit) are all there is; the measured curves
    /// (opto_table.rs) replace threshold, ratio and knee, and Threshold only lines the curve up with the voice
    /// (auto setup sets it)
    pub two_knob: bool,
    /// 0..100: Peak Reduction 40..100 of the measured model
    pub amount: f64,
    /// Limit instead of Compress
    pub limit: bool,
    /// the two-knob mode: turning Amount (or Compress/Limit) moves Makeup by the change in reduction at the voice's
    /// level, so the loudness stays; off, it is as the original: the level drops and Makeup brings it back
    pub auto_makeup: bool,
}

/// where the measured curves sit when Threshold is at the default -24: the model's own levels
const TWO_KNOB_REF: f64 = -24.0;

impl CompP {
    fn pr(&self) -> f64 {
        40.0 + self.amount.clamp(0.0, 100.0) * 0.6
    }

    /// the measured curve for Amount and the mode, moved with Threshold
    pub fn two_knob_curve(&self) -> GrCurve {
        let rows = if self.limit { &ot::LIMIT } else { &ot::COMPRESS };
        let f = ((self.pr() - ot::PRS[0]) / 10.0).clamp(0.0, (ot::PRS.len() - 1) as f64);
        let i = (f as usize).min(ot::PRS.len() - 2);
        let t = f - i as f64;
        let mut gr = [0.0; GR_POINTS];
        for (k, g) in gr.iter_mut().enumerate() {
            *g = rows[i][k] + t * (rows[i + 1][k] - rows[i][k]);
        }
        GrCurve { lo: ot::LO + (self.thr - TWO_KNOB_REF), step: 2.0, gr }
    }

    /// the reduction at the voice's level: auto setup lines the curve up so that the voice's middle is reduced about
    /// 4 dB at Amount 50, which the model's curve gives 6 dB above Threshold (the model's -18 dBFS)
    pub fn voice_gr(&self) -> f64 {
        self.two_knob_curve().at(self.thr + 6.0)
    }

    /// Amount or Compress/Limit changed from `before`: with the automatic makeup on, Makeup follows the reduction
    pub fn follow_makeup(&mut self, before: &CompP) {
        if self.two_knob && self.auto_makeup {
            let b = CompP { thr: self.thr, ..before.clone() };
            self.makeup = (((self.makeup + self.voice_gr() - b.voice_gr()) * 2.0).round() / 2.0).clamp(-12.0, 36.0);
        }
    }

    /// the times that go with Amount and the mode (the model's), into the values the page shows
    pub fn two_knob_times(&mut self) {
        let rel = if self.limit { &ot::LIMIT_REL } else { &ot::COMPRESS_REL };
        let pr = self.pr();
        let at = |j: usize| rel[j][0] + rel[j][1] * pr;
        self.rel = at(0).round().clamp(30.0, 400.0);
        self.rel2 = (at(1) / 10.0).round() * 10.0;
        self.slow = at(2).round();
        self.memory = 50.0;
        self.att = 80.0;
        self.auto_att = true;
    }
}

impl Default for CompP {
    fn default() -> Self {
        CompP { style: String::new(), thr: -24.0, ratio: 3.0, knee: 6.0, att: 10.0, rel: 120.0, makeup: 6.0, rel2: 0.0, slow: 0.0, memory: 0.0, auto_att: false, two_knob: false, amount: 50.0, limit: false, auto_makeup: false }
    }
}

/// A compressor style: how it compresses (ratio, knee) and moves (attack, release); the threshold and makeup stay
/// (they are the level's, auto setup's)
#[derive(Clone, Copy, Debug)]
pub struct CompStyle {
    pub name: &'static str,
    pub ratio: f64,
    pub knee: f64,
    pub att: f64,
    pub rel: f64,
    pub rel2: f64,
    pub slow: f64,
    pub memory: f64,
    pub auto_att: bool,
}

/// "clean": the talk presets' own; "opto": an optical leveller's, from an LA-2A model trained on SignalTrain's
/// recordings and probed with test signals (stream-mic design notes comp-styles.md, 2026-11-11): ratio about 3 with
/// a wide knee at its stronger settings; half way in 30 ms at 5 dB, 12 at 12 dB, 5 at 20 dB (an 80 ms attack halving
/// every 5 dB); a release of 65 % in about 40 ms and 35 % in about 0.7 s, the slow part about 1.5 times as slow
/// after a long hold.
pub const COMP_STYLES: [CompStyle; 2] = [
    CompStyle { name: "clean", ratio: 3.0, knee: 6.0, att: 10.0, rel: 120.0, rel2: 0.0, slow: 0.0, memory: 0.0, auto_att: false },
    CompStyle { name: "opto", ratio: 3.0, knee: 12.0, att: 80.0, rel: 40.0, rel2: 700.0, slow: 35.0, memory: 50.0, auto_att: true },
];

pub fn comp_style(name: &str) -> Option<CompStyle> {
    COMP_STYLES.iter().copied().find(|s| s.name == name)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LimP {
    /// true peak ceiling, dBTP
    pub ceil: f64,
    pub rel: f64,
}

impl Default for LimP {
    fn default() -> Self {
        LimP { ceil: -1.0, rel: 50.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VerbP {
    /// the style the values below came from (VERB_STYLES), "" when they were changed by hand
    pub style: String,
    /// wet with music playing, %
    pub mix: f64,
    /// wet without music, %
    pub mix_dry: f64,
    /// seconds to glide between them
    pub follow: f64,
    /// T60 at low-mid frequencies, s
    pub size: f64,
    /// extra pre-delay on top of the reverb's own (about 29 ms), ms
    pub pre: f64,
    pub lowcut: f64,
    pub damp: f64,
    /// early reflections, 0 none .. 1
    pub er: f64,
    /// how long the early reflections spread, ms
    pub er_ms: f64,
    /// a low pass on the wet, Hz; 20000 is off
    pub hc: f64,
}

impl Default for VerbP {
    fn default() -> Self {
        VerbP { style: String::new(), mix: 0.0, mix_dry: 0.0, follow: 1.5, size: 0.4, pre: 0.0, lowcut: 250.0, damp: 7000.0, er: 0.0, er_ms: 30.0, hc: 20000.0 }
    }
}

/// An EQ style: the low cut, the four bands (Low a shelf at 200 Hz, Mid and Presence bells with their centre and Q, Air
/// a shelf at 10 kHz), the high cut (20000: none)
#[derive(Clone, Copy, Debug)]
pub struct EqStyle {
    pub name: &'static str,
    pub hp: f64,
    pub low: f64,
    pub mid: f64,
    pub mid_f: f64,
    pub mid_q: f64,
    pub pres: f64,
    pub pres_f: f64,
    pub pres_q: f64,
    pub air: f64,
    pub hc: f64,
}

/// Ours, and "f": Focusrite Vocaster's four Enhance presets as its user guide publishes them (v1.3, "Enhance
/// Presets' Parameters": a high pass at 80 Hz and three bells, frequency, Q and gain). Their bells go into our
/// bands: a bell at 100-135 Hz becomes the Low shelf, 450 Hz-1 kHz the Mid, 2-6 kHz the Presence, 7-11 kHz the Air
/// shelf (so those two are near, not the same); the rest are exact. Their compressor is not part of the EQ.
pub const EQ_STYLES: [EqStyle; 8] = [
    EqStyle { name: "natural", hp: 80.0, low: 0.0, mid: 0.0, mid_f: 400.0, mid_q: 1.4, pres: 0.0, pres_f: 3000.0, pres_q: 1.0, air: 0.0, hc: 20000.0 },
    EqStyle { name: "demud", hp: 100.0, low: 0.0, mid: -3.0, mid_f: 300.0, mid_q: 1.4, pres: 1.5, pres_f: 3000.0, pres_q: 1.0, air: 1.0, hc: 20000.0 },
    // what DAPS's engineer did to its studio speech (produced against clean, 20 speakers, 100 takes, the median;
    // stream-mic design notes eq-styles.md, 2026-11-11): the lows under 80-100 Hz down, about -2 dB around 315 Hz,
    // +1.5 dB broad at 2-3 kHz, and the top from 6 kHz down 2.5-3 dB
    EqStyle { name: "speech", hp: 90.0, low: 0.0, mid: -2.0, mid_f: 315.0, mid_q: 1.4, pres: 1.5, pres_f: 2500.0, pres_q: 0.7, air: -2.5, hc: 20000.0 },
    EqStyle { name: "phone", hp: 300.0, low: 0.0, mid: 3.0, mid_f: 1000.0, mid_q: 1.0, pres: 2.0, pres_f: 2500.0, pres_q: 1.0, air: 0.0, hc: 3400.0 },
    EqStyle { name: "f-clean", hp: 80.0, low: -2.0, mid: 0.0, mid_f: 400.0, mid_q: 1.4, pres: 2.0, pres_f: 2000.0, pres_q: 0.6, air: 2.0, hc: 20000.0 },
    EqStyle { name: "f-warm", hp: 80.0, low: -1.0, mid: 2.5, mid_f: 450.0, mid_q: 0.7, pres: -1.0, pres_f: 2500.0, pres_q: 0.5, air: 0.0, hc: 20000.0 },
    EqStyle { name: "f-bright", hp: 80.0, low: 0.0, mid: -2.0, mid_f: 500.0, mid_q: 0.6, pres: 3.0, pres_f: 2000.0, pres_q: 0.6, air: 3.0, hc: 20000.0 },
    EqStyle { name: "f-radio", hp: 80.0, low: 2.5, mid: -1.5, mid_f: 1000.0, mid_q: 1.5, pres: 1.5, pres_f: 6000.0, pres_q: 1.15, air: 0.0, hc: 20000.0 },
];

pub fn eq_style(name: &str) -> Option<EqStyle> {
    EQ_STYLES.iter().copied().find(|s| s.name == name)
}

/// A reverb style: decay T60 s, extra pre-delay ms, low cut Hz, damping Hz, early reflections 0..1, their spread ms,
/// and how much of the Space knob's mix it takes: the wet's energy (an impulse, test style_energy) evened out to
/// the hall's, within 1.2 dB without it; to be set by ear.
#[derive(Clone, Copy, Debug)]
pub struct VerbStyle {
    pub name: &'static str,
    pub size: f64,
    pub pre: f64,
    pub lowcut: f64,
    pub damp: f64,
    pub er: f64,
    pub er_ms: f64,
    /// a low pass on the wet, Hz (20000: none)
    pub hc: f64,
    pub gain: f64,
}

/// Our reverb styles: TC-Helicon VoiceTone R1's manual table (length, tone, initial reflections, pre-delay, words
/// only, no numbers) was the reference for what each is, and its order; the numbers are ours. Length: shortest 0.45 s, short 0.75-0.9, medium 1.5-1.9, longest 3; tone: bright 11 kHz damping, medium
/// 6.5-7, dark 3.5; reflections: low 0.12, medium 0.35, high 0.6-0.7; pre-delay (on top of the tail's own 29 ms,
/// which the reflections come before): none 0, short 10, medium 20-25, "yes" 30, long 60 ms. The vocal ranges and
/// low cuts first from mixing guides (lead 0.8-2 s; pre-delay room 5-20, plate 15-35, hall 20-60); checked 2026-11-10
/// against Yamaha SPX2000's and Dragonfly Reverb's published presets (design notes reverb-styles.md): the plate made
/// 2.0 s (theirs 1.8-2.44 s, R1 has it as long as the hall) and the low cuts brought to 100-200 Hz (theirs mostly
/// none or 50-140 Hz; the smallest space keeps 250).
pub const VERB_STYLES: [VerbStyle; 24] = [
    VerbStyle { name: "hall", size: 1.8, pre: 25.0, lowcut: 150.0, damp: 6500.0, er: 0.35, er_ms: 45.0, hc: 20000.0, gain: 1.00 },
    VerbStyle { name: "plate", size: 2.0, pre: 20.0, lowcut: 150.0, damp: 11000.0, er: 0.12, er_ms: 25.0, hc: 20000.0, gain: 0.90 },
    VerbStyle { name: "theatre", size: 1.9, pre: 30.0, lowcut: 100.0, damp: 3500.0, er: 0.12, er_ms: 50.0, hc: 20000.0, gain: 1.08 },
    VerbStyle { name: "club", size: 0.9, pre: 10.0, lowcut: 150.0, damp: 6500.0, er: 0.6, er_ms: 30.0, hc: 20000.0, gain: 1.07 },
    VerbStyle { name: "room", size: 0.75, pre: 0.0, lowcut: 200.0, damp: 11000.0, er: 0.6, er_ms: 20.0, hc: 20000.0, gain: 1.03 },
    VerbStyle { name: "studio", size: 0.85, pre: 0.0, lowcut: 150.0, damp: 7000.0, er: 0.35, er_ms: 25.0, hc: 20000.0, gain: 1.09 },
    VerbStyle { name: "ambience", size: 0.45, pre: 0.0, lowcut: 250.0, damp: 11000.0, er: 0.7, er_ms: 12.0, hc: 20000.0, gain: 1.07 },
    VerbStyle { name: "arena", size: 3.0, pre: 60.0, lowcut: 100.0, damp: 6000.0, er: 0.35, er_ms: 80.0, hc: 20000.0, gain: 0.95 },
    // Yamaha SPX2000 ("y") and Dragonfly Reverb ("d"): their published presets, turned into ours by
    // tools/verb_styles.py (how, and the sources, there)
    VerbStyle { name: "y-vocal-plate", size: 2.4, pre: 35.0, lowcut: 80.0, damp: 5050.0, er: 1.98, er_ms: 22.1, hc: 10600.0, gain: 1.03 },
    VerbStyle { name: "y-vocal-plate-x", size: 2.44, pre: 30.0, lowcut: 140.0, damp: 5100.0, er: 0.0, er_ms: 25.0, hc: 6300.0, gain: 1.55 },
    VerbStyle { name: "y-vocal", size: 2.5, pre: 25.0, lowcut: 90.0, damp: 3400.0, er: 0.0, er_ms: 25.0, hc: 8000.0, gain: 1.53 },
    VerbStyle { name: "y-vocal-chamber", size: 1.9, pre: 49.8, lowcut: 0.0, damp: 4200.0, er: 0.93, er_ms: 25.0, hc: 7500.0, gain: 1.46 },
    VerbStyle { name: "y-presence", size: 1.4, pre: 35.0, lowcut: 0.0, damp: 20000.0, er: 1.32, er_ms: 12.0, hc: 14000.0, gain: 0.91 },
    VerbStyle { name: "y-small-room", size: 0.68, pre: 0.0, lowcut: 0.0, damp: 6700.0, er: 0.0, er_ms: 25.0, hc: 10000.0, gain: 1.61 },
    VerbStyle { name: "y-medium-hall", size: 2.01, pre: 15.0, lowcut: 0.0, damp: 9550.0, er: 0.0, er_ms: 25.0, hc: 5000.0, gain: 1.57 },
    VerbStyle { name: "y-echo-room", size: 1.0, pre: 0.0, lowcut: 0.0, damp: 1050.0, er: 1.21, er_ms: 20.2, hc: 6700.0, gain: 1.91 },
    VerbStyle { name: "d-small-vocal-room", size: 0.3, pre: 0.0, lowcut: 0.0, damp: 6400.0, er: 0.73, er_ms: 20.0, hc: 20000.0, gain: 1.12 },
    VerbStyle { name: "d-medium-vocal-room", size: 0.6, pre: 2.0, lowcut: 0.0, damp: 6000.0, er: 0.81, er_ms: 20.0, hc: 20000.0, gain: 1.08 },
    VerbStyle { name: "d-large-vocal-room", size: 1.0, pre: 4.0, lowcut: 0.0, damp: 5000.0, er: 0.91, er_ms: 20.0, hc: 20000.0, gain: 1.04 },
    VerbStyle { name: "d-vocal-studio", size: 0.8, pre: 0.0, lowcut: 0.0, damp: 2550.0, er: 0.86, er_ms: 20.0, hc: 5800.0, gain: 1.96 },
    VerbStyle { name: "d-small-vocal-hall", size: 1.2, pre: 4.0, lowcut: 0.0, damp: 3050.0, er: 0.95, er_ms: 20.0, hc: 6250.0, gain: 1.76 },
    VerbStyle { name: "d-medium-vocal-hall", size: 1.3, pre: 8.0, lowcut: 0.0, damp: 3900.0, er: 0.97, er_ms: 20.0, hc: 5800.0, gain: 1.69 },
    VerbStyle { name: "d-large-vocal-hall", size: 2.4, pre: 12.0, lowcut: 0.0, damp: 3900.0, er: 1.16, er_ms: 20.0, hc: 6250.0, gain: 1.45 },
    VerbStyle { name: "d-clear-plate", size: 0.6, pre: 0.0, lowcut: 100.0, damp: 7000.0, er: 0.0, er_ms: 20.0, hc: 13000.0, gain: 1.48 },
];

pub fn verb_style(name: &str) -> Option<VerbStyle> {
    VERB_STYLES.iter().copied().find(|s| s.name == name)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preset {
    /// file name without .toml
    pub name: String,
    /// what the settings page shows
    pub label: String,
    pub about: String,
    /// the reverb follows a backing track (singing): without music it is a third of the knob's amount
    pub follow_music: bool,
    pub knobs: Knobs,
    pub on: On,
    /// which of the stages that are on also go into the monitor (user 2026-11-07: chosen per stage); denoise there
    /// is the send's own result (no second model), the limiter there its look-ahead (else the soft clipper)
    #[serde(default = "monitor_default")]
    pub monitor: On,
    pub denoise: DenoiseP,
    pub lowcut: LowCutP,
    pub eq: EqP,
    pub deess: DeessP,
    pub exp: ExpP,
    pub comp: CompP,
    pub verb: VerbP,
    pub lim: LimP,
}

impl Default for Preset {
    fn default() -> Self {
        Preset {
            name: String::new(),
            label: String::new(),
            about: String::new(),
            follow_music: false,
            knobs: Knobs::default(),
            on: On::default(),
            monitor: On::monitor_default(),
            denoise: DenoiseP::default(),
            lowcut: LowCutP::default(),
            eq: EqP::default(),
            deess: DeessP::default(),
            exp: ExpP::default(),
            comp: CompP::default(),
            verb: VerbP::default(),
            lim: LimP::default(),
        }
    }
}

/// The presets that come with the program (presets/*.toml)
pub const BUILT_IN: [(&str, &str); 3] = [
    ("talk-omni", include_str!("../../../presets/talk-omni.toml")),
    ("talk-directional", include_str!("../../../presets/talk-directional.toml")),
    ("sing", include_str!("../../../presets/sing.toml")),
];

fn half(v: f64) -> f64 {
    (v * 2.0).round() / 2.0 + 0.0 // (+ 0.0: no "-0")
}

impl Preset {
    pub fn parse(text: &str) -> Result<Preset, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn built_in(name: &str) -> Option<Preset> {
        BUILT_IN.iter().find(|(n, _)| *n == name).map(|(_, t)| Preset::parse(t).expect("built-in preset"))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Turn one knob: it rewrites the parameters it stands for (the settings page's mapping)
    pub fn set_knob(&mut self, knob: &str, v: f64) {
        match knob {
            "denoise" => {
                let v = v.clamp(0.0, 100.0);
                self.knobs.denoise = v;
                self.on.denoise = v > 0.0;
                self.denoise.limit = (6.0 + v / 100.0 * 34.0).round();
            }
            "body" => {
                let v = v.clamp(-100.0, 100.0);
                self.knobs.body = v;
                // on top of the style's (or the preset's) own Low and Presence
                self.eq.low = half(self.eq.low_base + v / 100.0 * 4.0).clamp(-6.0, 6.0);
                self.eq.pres = half(self.eq.pres_base - v / 100.0 * 2.0).clamp(-6.0, 6.0);
                self.lowcut.freq = if v < 0.0 { ((self.lowcut.base - v * 0.5) / 5.0).round() * 5.0 } else { self.lowcut.base };
            }
            "space" => {
                // the knob is the reverb with the music; without it (a cappella, talking between songs) a third,
                // since a level right under a backing track is far too wet on the voice alone (user 2026-11-06)
                let v = v.clamp(0.0, 100.0);
                self.knobs.space = v;
                self.on.verb = v > 0.0;
                // how wet only: the style decides how long and how bright (2026-11-10, styles as in io44-control)
                let gain = verb_style(&self.verb.style).map_or(1.0, |s| s.gain);
                self.verb.mix = (v / 100.0 * 30.0 * gain).round();
                self.verb.mix_dry = (self.verb.mix * if self.follow_music { 0.3 } else { 1.0 }).round();
            }
            "sib" => {
                let v = v.clamp(0.0, 100.0);
                self.knobs.sib = v;
                self.on.deess = v > 0.0;
                self.deess.thr = half(14.0 - v / 100.0 * 14.0);
            }
            "gate" => {
                let v = v.clamp(-80.0, -20.0);
                self.knobs.gate = v;
                self.on.exp = v > -80.0;
                self.exp.thr = v;
            }
            _ => {}
        }
    }

    /// A reverb style: its decay, pre-delay, low cut, damping and reflections, and the Space knob again (its mix)
    pub fn set_verb_style(&mut self, name: &str) -> bool {
        let Some(st) = verb_style(name) else { return false };
        let v = &mut self.verb;
        v.style = st.name.to_string();
        v.size = st.size;
        v.pre = st.pre;
        v.lowcut = st.lowcut;
        v.damp = st.damp;
        v.er = st.er;
        v.er_ms = st.er_ms;
        v.hc = st.hc;
        let k = self.knobs.space;
        self.set_knob("space", k);
        true
    }

    /// A compressor style: its ratio, knee, attack and release; the threshold and makeup stay
    pub fn set_comp_style(&mut self, name: &str) -> bool {
        if name == "two-knob" {
            let c = &mut self.comp;
            c.style = name.to_string();
            c.two_knob = true;
            c.two_knob_times();
            return true;
        }
        let Some(st) = comp_style(name) else { return false };
        let c = &mut self.comp;
        c.style = st.name.to_string();
        c.two_knob = false;
        (c.ratio, c.knee, c.att, c.rel, c.rel2, c.slow, c.memory, c.auto_att) = (st.ratio, st.knee, st.att, st.rel, st.rel2, st.slow, st.memory, st.auto_att);
        true
    }

    /// An EQ style: its values, Low and Presence as the Body knob's base, and the Body knob again on top
    pub fn set_eq_style(&mut self, name: &str) -> bool {
        let Some(st) = eq_style(name) else { return false };
        let e = &mut self.eq;
        e.style = st.name.to_string();
        self.lowcut.freq = st.hp;
        self.lowcut.base = st.hp;
        self.on.lowcut = true;
        let e = &mut self.eq;
        e.low_base = st.low;
        e.mid = st.mid;
        e.mid_f = st.mid_f;
        e.mid_q = st.mid_q;
        e.pres_base = st.pres;
        e.pres_f = st.pres_f;
        e.pres_q = st.pres_q;
        e.air = st.air;
        e.hc = st.hc;
        let k = self.knobs.body;
        self.set_knob("body", k);
        true
    }

    /// Every knob applied again (what a preset's parameters would be if only the knobs had set them)
    pub fn from_knobs(&self) -> Preset {
        let mut p = self.clone();
        let k = self.knobs.clone();
        p.set_knob("denoise", k.denoise);
        p.set_knob("body", k.body);
        p.set_knob("space", k.space);
        p.set_knob("sib", k.sib);
        p.set_knob("gate", k.gate);
        p
    }

    pub fn denoise_params(&self) -> DenoiseParams {
        DenoiseParams { limit_db: self.denoise.limit }
    }

    pub fn eq_params(&self) -> EqParams {
        // the low cut is its own stage now (Preset::lowcut)
        EqParams { hp_hz: 0.0, low_db: self.eq.low, mid_hz: self.eq.mid_f, mid_db: self.eq.mid, mid_q: self.eq.mid_q, pres_hz: self.eq.pres_f, pres_db: self.eq.pres, pres_q: self.eq.pres_q, air_db: self.eq.air, hc_hz: self.eq.hc }
    }

    pub fn deess_params(&self) -> DeessParams {
        DeessParams { thr_db: self.deess.thr, freq_hz: self.deess.freq, max_db: self.deess.max, ..DeessParams::default() }
    }

    pub fn exp_params(&self) -> ExpParams {
        let e = &self.exp;
        ExpParams { thr_db: e.thr, ratio: e.ratio, range_db: e.range, hold_ms: e.hold, rel_ms: e.rel, ..ExpParams::default() }
    }

    pub fn comp_params(&self) -> CompParams {
        let c = &self.comp;
        CompParams {
            thr_db: c.thr,
            ratio: c.ratio,
            knee_db: c.knee,
            att_ms: c.att,
            rel_ms: c.rel,
            makeup_db: c.makeup,
            rel2_ms: c.rel2,
            slow: c.slow / 100.0,
            memory: c.memory / 100.0,
            att_auto: c.auto_att,
            curve: if c.two_knob { Some(c.two_knob_curve()) } else { None },
            ..CompParams::default()
        }
    }

    pub fn verb_params(&self) -> VerbParams {
        let v = &self.verb;
        VerbParams {
            t60_s: v.size,
            pre_ms: v.pre,
            lowcut_hz: v.lowcut,
            damp_hz: v.damp,
            mix: v.mix / 100.0,
            mix_dry: v.mix_dry / 100.0,
            follow_s: v.follow,
            er: v.er,
            er_ms: v.er_ms,
            hc_hz: v.hc,
        }
    }

    pub fn lim_params(&self) -> LimParams {
        LimParams { ceil_db: self.lim.ceil, rel_ms: self.lim.rel, ..LimParams::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_presets_load_and_agree_with_their_knobs() {
        for (name, _) in BUILT_IN {
            let p = Preset::built_in(name).unwrap();
            assert_eq!(p.name, name);
            // what the file says is what the knobs at those positions give: turning a knob back to where it was
            // changes nothing
            assert_eq!(p.from_knobs(), p, "{name}");
        }
    }

    #[test]
    fn comp_styles() {
        let mut p = Preset::built_in("sing").unwrap();
        let (thr, mk) = (p.comp.thr, p.comp.makeup);
        assert!(p.set_comp_style("opto"));
        assert_eq!((p.comp.style.as_str(), p.comp.rel2, p.comp.thr, p.comp.makeup), ("opto", 700.0, thr, mk));
        assert!(p.set_comp_style("clean"));
        // the plain compressor the talk presets had before the two-knob mode
        assert_eq!((p.comp.ratio, p.comp.knee, p.comp.att, p.comp.rel, p.comp.rel2, p.comp.auto_att, p.comp.two_knob), (3.0, 6.0, 10.0, 120.0, 0.0, false, false));
        assert!(!p.set_comp_style("nope"));
        assert!(Preset::parse(&p.to_toml()).unwrap() == p);
    }

    #[test]
    fn two_knob_mode() {
        let mut p = Preset::built_in("talk-omni").unwrap();
        assert!(p.set_comp_style("two-knob"));
        assert!(p.comp.two_knob && p.comp.auto_att);
        // more Amount: more reduction at the same level, and a faster release
        let at = |p: &Preset, x: f64| p.comp_params().curve.unwrap().at(x);
        p.comp.amount = 0.0;
        p.comp.two_knob_times();
        let (g0, r0) = (at(&p, -20.0), p.comp.rel2);
        p.comp.amount = 100.0;
        p.comp.two_knob_times();
        let (g1, r1) = (at(&p, -20.0), p.comp.rel2);
        assert!(g1 > g0 + 8.0 && r1 < r0, "{g0} {g1} {r0} {r1}");
        // Threshold moves the whole curve: 6 dB lower, the same reduction 6 dB lower
        let g = at(&p, -20.0);
        p.comp.thr -= 6.0;
        assert!((at(&p, -26.0) - g).abs() < 1e-9);
        // Limit is harder than Compress at the top
        p.comp.amount = 83.0;
        let c = at(&p, -14.0);
        p.comp.limit = true;
        assert!(at(&p, -6.0) - at(&p, -14.0) >= 0.0 && c > 0.0);
        // another style leaves the mode
        assert!(p.set_comp_style("opto"));
        assert!(!p.comp.two_knob && p.comp_params().curve.is_none());
    }

    #[test]
    fn built_in_presets_use_the_two_knob_mode_with_its_times() {
        for (name, _) in BUILT_IN {
            let p = Preset::built_in(name).unwrap();
            assert!(p.comp.two_knob && p.comp.style == "two-knob", "{name}");
            let mut q = p.comp.clone();
            q.two_knob_times();
            assert_eq!(q, p.comp, "{name}: the times are the model's for its Amount");
        }
    }

    #[test]
    fn automatic_makeup_keeps_the_voice_level() {
        let mut p = Preset::built_in("talk-omni").unwrap();
        let before = p.comp.clone();
        p.comp.amount = 90.0;
        p.comp.follow_makeup(&before);
        assert_eq!(p.comp.makeup, before.makeup, "off: Makeup stays");
        p.comp.auto_makeup = true;
        p.comp.follow_makeup(&before);
        let d = p.comp.voice_gr() - CompP { thr: p.comp.thr, ..before.clone() }.voice_gr();
        assert!(d > 3.0 && (p.comp.makeup - before.makeup - d).abs() <= 0.25, "{d} {}", p.comp.makeup);
    }

    #[test]
    fn a_preset_survives_toml() {
        let p = Preset::built_in("sing").unwrap();
        assert_eq!(Preset::parse(&p.to_toml()).unwrap(), p);
    }
}

#[cfg(test)]
mod style_energy {
    use super::*;
    use stream_mic_dsp::reverb::Reverb;
    use stream_mic_dsp::Stage;

    /// The wet's energy (an impulse in, 6 s) for each style at mix 1: what the styles' gains even out.
    /// cargo test -p stream-mic-chain style_energy -- --ignored --nocapture
    #[test]
    #[ignore]
    fn style_energy() {
        for st in VERB_STYLES {
            let mut p = Preset::default();
            p.set_verb_style(st.name);
            p.verb.mix = 100.0;
            p.verb.mix_dry = 100.0;
            let mut r = Reverb::new(p.verb_params());
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
            let e: f64 = l.iter().chain(rr.iter()).map(|v| (*v as f64).powi(2)).sum::<f64>() / 2.0;
            let early: f64 = l[..4800].iter().chain(rr[..4800].iter()).map(|v| (*v as f64).powi(2)).sum::<f64>() / 2.0;
            println!("{:9} energy {:7.2} dB  first 100 ms {:7.2} dB", st.name, 10.0 * e.log10(), 10.0 * early.log10());
        }
    }
}
