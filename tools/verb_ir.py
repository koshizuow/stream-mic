"""Write the reference reverb's wet impulse response (stereo) for measure.py decay.

Usage: [VERB=fdn] uv run python verb_ir.py OUT.wav T60 [PRE_MS LOWCUT DAMP_HZ SECONDS]
"""
import sys

import numpy as np
import soundfile as sf

import os

if os.environ.get("VERB") == "fdn":
    from reffdn import reverb
else:
    from refverb import reverb

FS = 48000
t60 = float(sys.argv[2])
pre, lowcut, damp, secs = (float(v) for v in (sys.argv[3:7] + ["0", "0", "20000", "4"][len(sys.argv[3:7]):]))
x = np.zeros(int(secs * FS))
x[0] = 0.5
yl, yr = reverb(x, FS, t60, pre, lowcut, damp)
sf.write(sys.argv[1], np.stack([yl, yr], 1).astype(np.float32), FS, subtype="FLOAT")
