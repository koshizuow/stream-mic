//! Presets (build-plan.md step 5): a TOML file with the four knobs' positions and every stage's parameters, in the
//! units the settings page shows. The parameters as written are what plays; turning a knob rewrites the ones it
//! stands for (the mapping that was in ui/index.html as a demo lives here now).

use serde::{Deserialize, Serialize};
use stream_mic_denoise::DenoiseParams;
use stream_mic_dsp::deesser::DeessParams;
use stream_mic_dsp::dynamics::{CompParams, ExpParams};
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
    pub eq: bool,
    pub deess: bool,
    pub exp: bool,
    pub comp: bool,
    pub verb: bool,
    pub lim: bool,
}

impl Default for On {
    fn default() -> Self {
        On { denoise: false, eq: true, deess: true, exp: true, comp: true, verb: false, lim: true }
    }
}

impl On {
    /// What the monitor (the headphones) gets by default: every stage that works sample by sample, so no delay. Not
    /// denoise (20 ms and more) and not the limiter's look-ahead (2 ms): a soft clipper guards the ears instead.
    pub fn monitor_default() -> Self {
        On { denoise: false, eq: true, deess: true, exp: true, comp: true, verb: true, lim: false }
    }

    /// on in both
    pub fn and(&self, o: &On) -> On {
        On {
            denoise: self.denoise && o.denoise,
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
pub struct EqP {
    pub hp: f64,
    /// the low cut with the body knob at 0 or above (thinner turns raise it)
    pub hp_base: f64,
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
}

impl Default for EqP {
    fn default() -> Self {
        EqP { hp: 80.0, hp_base: 80.0, low: 0.0, mid: 0.0, mid_f: 400.0, mid_q: 1.4, pres: 1.5, pres_f: 3000.0, pres_q: 1.0, air: 1.0, hc: 20000.0 }
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
    pub thr: f64,
    pub ratio: f64,
    pub knee: f64,
    pub att: f64,
    pub rel: f64,
    pub makeup: f64,
}

impl Default for CompP {
    fn default() -> Self {
        CompP { thr: -24.0, ratio: 3.0, knee: 6.0, att: 10.0, rel: 120.0, makeup: 6.0 }
    }
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
}

impl Default for VerbP {
    fn default() -> Self {
        VerbP { style: String::new(), mix: 0.0, mix_dry: 0.0, follow: 1.5, size: 0.4, pre: 0.0, lowcut: 250.0, damp: 7000.0, er: 0.0, er_ms: 30.0 }
    }
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
    pub gain: f64,
}

/// The styles of TC-Helicon VoiceTone R1 (its manual's table: length, tone, initial reflections, pre-delay), in its
/// order. Length: shortest 0.45 s, short 0.75-0.9, medium 1.5-1.9, longest 3; tone: bright 11 kHz damping, medium
/// 6.5-7, dark 3.5; reflections: low 0.12, medium 0.35, high 0.6-0.7; pre-delay (on top of the tail's own 29 ms,
/// which the reflections come before): none 0, short 10, medium 20-25, "yes" 30, long 60 ms. The vocal ranges and
/// low cuts as in io44-control's table (mixing guides: lead 0.8-2 s; pre-delay room 5-20, plate 15-35, hall 20-60).
pub const VERB_STYLES: [VerbStyle; 8] = [
    VerbStyle { name: "hall", size: 1.8, pre: 25.0, lowcut: 180.0, damp: 6500.0, er: 0.35, er_ms: 45.0, gain: 1.00 },
    VerbStyle { name: "plate", size: 1.5, pre: 20.0, lowcut: 350.0, damp: 11000.0, er: 0.12, er_ms: 25.0, gain: 0.96 },
    VerbStyle { name: "theatre", size: 1.9, pre: 30.0, lowcut: 120.0, damp: 3500.0, er: 0.12, er_ms: 50.0, gain: 1.08 },
    VerbStyle { name: "club", size: 0.9, pre: 10.0, lowcut: 200.0, damp: 6500.0, er: 0.6, er_ms: 30.0, gain: 1.07 },
    VerbStyle { name: "room", size: 0.75, pre: 0.0, lowcut: 300.0, damp: 11000.0, er: 0.6, er_ms: 20.0, gain: 1.03 },
    VerbStyle { name: "studio", size: 0.85, pre: 0.0, lowcut: 200.0, damp: 7000.0, er: 0.35, er_ms: 25.0, gain: 1.09 },
    VerbStyle { name: "ambience", size: 0.45, pre: 0.0, lowcut: 400.0, damp: 11000.0, er: 0.7, er_ms: 12.0, gain: 1.07 },
    VerbStyle { name: "arena", size: 3.0, pre: 60.0, lowcut: 150.0, damp: 6000.0, er: 0.35, er_ms: 80.0, gain: 0.95 },
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
    (v * 2.0).round() / 2.0
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
                self.eq.low = half(v / 100.0 * 4.0);
                self.eq.pres = half(1.5 - v / 100.0 * 2.0);
                self.eq.hp = if v < 0.0 { ((self.eq.hp_base - v * 0.5) / 5.0).round() * 5.0 } else { self.eq.hp_base };
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
        let k = self.knobs.space;
        self.set_knob("space", k);
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
        EqParams { hp_hz: self.eq.hp, low_db: self.eq.low, mid_hz: self.eq.mid_f, mid_db: self.eq.mid, mid_q: self.eq.mid_q, pres_hz: self.eq.pres_f, pres_db: self.eq.pres, pres_q: self.eq.pres_q, air_db: self.eq.air, hc_hz: self.eq.hc }
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
        CompParams { thr_db: c.thr, ratio: c.ratio, knee_db: c.knee, att_ms: c.att, rel_ms: c.rel, makeup_db: c.makeup, ..CompParams::default() }
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
