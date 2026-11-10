#!/usr/bin/env python3
"""The published reverb presets the style families are made from, and how their numbers become ours.

  python3 tools/verb_styles.py          # stream-mic's rows (crates/chain/src/preset.rs VERB_STYLES), then io44-control's

Sources (design notes: working-memory stream-mic/design/dsp/reverb-styles.md):
- "y": Yamaha SPX2000 Owner's Manual, the preset tables (pages 27-31): REV TIME (1 kHz T60), HI.RATIO (the highs'
  T60 against it), INI.DLY (the early reflections' delay), E/R DLY (the tail after them), E/R BAL. (0 tail only ..
  100 % early only), HPF and LPF on the effect.
- "d": Dragonfly Reverb (GPL-3.0, github.com/michaelwillis/dragonfly-reverb, commit 440ec7b), the presets in
  plugins/*/DistrhoPluginInfo.h: decay, predelay, damping, high cut, the hall's high crossover and multiplier, the
  room's early level against its late level (10 % : 20 % in every preset).

How (stream-mic's reverb, crates/dsp/src/reverb.rs):
- t60 = the preset's decay; pre = its first sound's delay (Yamaha INI.DLY, Dragonfly predelay); our tail itself comes
  about 29 ms later, the early reflections right after pre.
- damp (the loop's one-pole low pass) from a high-frequency ratio r at a frequency f: the T60 at f is r times the
  one at 1 kHz, solved for the mean loop delay (62 ms). Yamaha's HI.RATIO at 6 kHz (the manual names no frequency);
  Dragonfly's hall high multiplier at its crossover; Dragonfly's room and plate damping as given.
- er from the early/late balance b taken as amplitudes: the early energy (0.481 er^2, test er_balance) is
  (b/(1-b))^2 times the tail's (measured: 1.18 at 0.5 s, 1.59 at 1, 2.34 at 2, 2.98 at 3 s). Presets with no
  early reflections (Yamaha REV-X and classic, Dragonfly's plate) get none. er_ms: Yamaha E/R DLY (25 ms without
  one), 20 ms for Dragonfly.
- lowcut = HPF / low cut (Thru, or Dragonfly's 4 Hz: off, 0); hc = LPF / high cut (Thru or 16 kHz: off, 20 kHz).
How (io44's reverb: size, pre-delay, low cut only; measured 2026-11-09):
- size from the decay through the measured 1 kHz T30 per size; pre-delay so the tail starts when the preset's does
  (Yamaha INI.DLY + E/R DLY, Dragonfly predelay) less the io44's own 24 ms; low cut = HPF (Thru: 20 Hz, its lowest).
"""
import math

FS = 48000.0
LOOP_S = 0.045 * sum(2 ** (c / 8) for c in range(8)) / 8   # the FDN's mean loop delay

# name, decay s, hi ratio (or None), its frequency, first delay ms, tail after it ms (or None), early balance 0..1
# (or None: none), hpf Hz (0 = Thru), lpf Hz (0 = Thru), damp Hz given (or None), label
Y = [
    ("y-vocal-plate", 2.4, 0.3, 6000, 35.0, 22.1, 0.46, 80, 10600, None, "Vocal Plate"),
    ("y-vocal-plate-x", 2.44, 0.3, 6000, 30.0, None, None, 140, 6300, None, "Vocal Plate X"),
    ("y-vocal", 2.5, 0.2, 6000, 25.0, None, None, 90, 8000, None, "Vocal"),
    ("y-vocal-chamber", 1.9, 0.3, 6000, 49.8, None, 0.30, 0, 7500, None, "Vocal Chamber"),
    ("y-presence", 1.4, 1.0, 6000, 35.0, 12.0, 0.40, 0, 14000, None, "Presence"),
    ("y-small-room", 0.68, 0.7, 6000, 0.0, None, None, 0, 10000, None, "Small Room"),
    ("y-medium-hall", 2.01, 0.6, 6000, 15.0, None, None, 0, 5000, None, "Medium Hall"),
    ("y-echo-room", 1.0, 0.2, 6000, 0.0, 20.2, 0.40, 0, 6700, None, "Echo Room"),
]
D = [
    ("d-small-vocal-room", 0.3, None, 0, 0.0, None, 1 / 3, 0, 0, 6400, "Small Vocal Room"),
    ("d-medium-vocal-room", 0.6, None, 0, 2.0, None, 1 / 3, 0, 0, 6000, "Medium Vocal Room"),
    ("d-large-vocal-room", 1.0, None, 0, 4.0, None, 1 / 3, 0, 0, 5000, "Large Vocal Room"),
    ("d-vocal-studio", 0.8, 0.40, 5200, 0.0, None, 1 / 3, 0, 5800, None, "Vocal Studio"),
    ("d-small-vocal-hall", 1.2, 0.35, 5200, 4.0, None, 1 / 3, 0, 6250, None, "Small Vocal Hall"),
    ("d-medium-vocal-hall", 1.3, 0.40, 5200, 8.0, None, 1 / 3, 0, 5800, None, "Medium Vocal Hall"),
    ("d-large-vocal-hall", 2.4, 0.30, 4600, 12.0, None, 1 / 3, 0, 6250, None, "Large Vocal Hall"),
    ("d-clear-plate", 0.6, None, 0, 0.0, None, None, 100, 13000, 7000, "Clear Plate"),
]

TAIL = [(0.5, 1.184), (1.0, 1.590), (2.0, 2.340), (3.0, 2.980)]
IO_SIZE = [(0, .66), (.1, .73), (.2, .86), (.3, .96), (.4, 1.10), (.5, 1.27), (.6, 1.50), (.7, 1.86), (.8, 2.54),
           (.9, 3.77), (1, 7.2)]


def interp(pts, x):
    if x <= pts[0][0]:
        (x0, y0), (x1, y1) = pts[0], pts[1]
    elif x >= pts[-1][0]:
        (x0, y0), (x1, y1) = pts[-2], pts[-1]
    else:
        i = next(i for i in range(1, len(pts)) if x <= pts[i][0])
        (x0, y0), (x1, y1) = pts[i - 1], pts[i]
    return y0 + (y1 - y0) * (x - x0) / (x1 - x0)


def onepole_db(fc, f):
    a = math.exp(-2 * math.pi * fc / FS)
    w = 2 * math.pi * f / FS
    return 20 * math.log10((1 - a) / math.sqrt(1 - 2 * a * math.cos(w) + a * a))


def damp_for(t60, ratio, f):
    """the loop's low pass that makes the T60 at f ratio times the 1 kHz one"""
    if ratio >= 0.99:
        return 20000.0
    want = -(60 * LOOP_S / t60) * (1 / ratio - 1)   # extra dB a loop at f
    lo, hi = 50.0, 20000.0
    for _ in range(60):
        mid = math.sqrt(lo * hi)
        if onepole_db(mid, f) < want:
            lo = mid
        else:
            hi = mid
    return round(math.sqrt(lo * hi) / 50) * 50


def ours(row):
    name, t60, ratio, f, first, after, bal, hpf, lpf, damp, label = row
    d = damp if damp else damp_for(t60, ratio, f) if ratio else 20000.0
    er = 0.0
    if bal:
        er = math.sqrt((bal / (1 - bal)) ** 2 * interp(TAIL, t60) / 0.481)
    er_ms = after if after else 25.0 if name.startswith("y") else 20.0
    hc = lpf if lpf and lpf < 16000 else 20000.0
    return dict(name=name, size=t60, pre=first, lowcut=float(hpf), damp=d, er=round(min(er, 2.0), 2), er_ms=er_ms, hc=hc, label=label)


def io44(row):
    name, t60, _, _, first, after, _, hpf, _, _, label = row
    pts = [(t, s) for s, t in IO_SIZE]
    size = 0.0 if t60 <= IO_SIZE[0][1] else round(interp([(math.log(t), s) for t, s in pts], math.log(t60)), 2)
    tail = first + (after or 0.0)
    pre = max(0.0, round(tail - 24.4, 1)) / 1000
    return dict(name=name, size=min(size, 1.0), pre=pre, hp=float(hpf or 20), label=label)


if __name__ == "__main__":
    for r in Y + D:
        o = ours(r)
        print('    VerbStyle {{ name: "{name}", size: {size}, pre: {pre:.1f}, lowcut: {lowcut:.1f}, damp: {damp:.1f}, er: {er}, er_ms: {er_ms:.1f}, hc: {hc:.1f}, gain: 1.00 }},'.format(**o))
    print()
    for r in Y + D:
        o = io44(r)
        print('    "{name}": ({size}, {pre:.4f}, {hp:.1f}, {send}),'.format(send=round(-15 + 3.5 * o["size"], 1), **o))
