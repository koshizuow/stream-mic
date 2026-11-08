"""Minimal LADSPA host over ctypes (DeepFilterNet ships its real-time plugin only as LADSPA).

  python ladspa.py LIB.so            list the plugins and ports in a library
"""
import ctypes as C
import sys
import time

import numpy as np

P = C.c_void_p
INPUT, OUTPUT, CONTROL, AUDIO = 1, 2, 4, 8


class Hint(C.Structure):
    _fields_ = [("hints", C.c_int), ("lower", C.c_float), ("upper", C.c_float)]


class Desc(C.Structure):
    _fields_ = [("unique_id", C.c_ulong), ("label", C.c_char_p), ("properties", C.c_int), ("name", C.c_char_p),
                ("maker", C.c_char_p), ("copyright", C.c_char_p), ("port_count", C.c_ulong),
                ("port_desc", C.POINTER(C.c_int)), ("port_names", C.POINTER(C.c_char_p)),
                ("hints", C.POINTER(Hint)), ("impl", P),
                ("instantiate", C.CFUNCTYPE(P, P, C.c_ulong)),
                ("connect_port", C.CFUNCTYPE(None, P, C.c_ulong, P)),
                ("activate", C.CFUNCTYPE(None, P)),
                ("run", C.CFUNCTYPE(None, P, C.c_ulong)),
                ("run_adding", P), ("set_gain", P),
                ("deactivate", C.CFUNCTYPE(None, P)),
                ("cleanup", C.CFUNCTYPE(None, P))]


def descriptors(lib):
    so = C.CDLL(lib)
    so.ladspa_descriptor.restype = C.POINTER(Desc)
    so.ladspa_descriptor.argtypes = [C.c_ulong]
    out, i = [], 0
    while True:
        d = so.ladspa_descriptor(i)
        if not d:
            return so, out
        out.append(d)
        i += 1


def default_value(h, fs):
    """LADSPA_HINT_DEFAULT_* (ladspa.h): the default a host should use for a control port."""
    lo, hi = h.lower, h.upper
    if h.hints & 0x8:  # SAMPLE_RATE: bounds are fractions of the sample rate
        lo, hi = lo * fs, hi * fs
    log = h.hints & 0x10
    mix = lambda a, b, t: (np.exp(np.log(a) * (1 - t) + np.log(b) * t) if log and a > 0 and b > 0 else a * (1 - t) + b * t)
    d = h.hints & 0x3C0
    return {0x40: lo, 0x80: mix(lo, hi, .25), 0xC0: mix(lo, hi, .5), 0x100: mix(lo, hi, .75), 0x140: hi,
            0x200: 0.0, 0x240: 1.0, 0x280: 100.0, 0x2C0: 440.0}.get(d, lo if h.hints & 0x2 else 0.0)


class LadspaPlugin:
    """Mono in, mono out, block by block. controls: {port name substring: value}."""

    def __init__(self, lib, label, fs, controls, block=512):
        self.so, ds = descriptors(lib)
        d = next(x.contents for x in ds if x.contents.label.decode() == label)
        # keep the Desc alive through the library handle
        self.d = d
        self.h = d.instantiate(C.cast(C.pointer(d), P), fs)
        self.keep, self.ins, self.outs = [], [], []
        controls = dict(controls)
        for i in range(d.port_count):
            pd, name = d.port_desc[i], d.port_names[i].decode()
            if pd & AUDIO:
                buf = np.zeros(block, dtype=np.float32)
                (self.ins if pd & INPUT else self.outs).append(buf)
            else:
                val = None
                for k in list(controls):
                    if k.lower() in name.lower():
                        val = float(controls.pop(k))
                if val is None:
                    # the plugin's own default (2026-11-06: this used to give the port's lower bound once the given
                    # controls were used up, and 0 before that; DeepFilterNet's thresholds at their minimum made it
                    # pass almost every frame through untouched)
                    val = default_value(d.hints[i], fs)
                buf = np.array([val], dtype=np.float32)
            self.keep.append(buf)
            d.connect_port(self.h, i, buf.ctypes.data)
        if controls:
            raise SystemExit(f"unknown controls: {sorted(controls)}")
        d.activate(self.h)

    def run(self, xin, timer=None):
        m = len(xin)
        for k, buf in enumerate(self.ins):
            buf[:m] = xin[:, k % xin.shape[1]]
        t = time.perf_counter()
        self.d.run(self.h, m)
        if timer is not None:
            timer[0] += time.perf_counter() - t
        return np.stack([buf[:m] for buf in self.outs], 1)

    def latency(self):
        return 0

    def close(self):
        self.d.deactivate(self.h)
        self.d.cleanup(self.h)


if __name__ == "__main__":
    _, ds = descriptors(sys.argv[1])
    for p in ds:
        d = p.contents
        print(d.label.decode(), "|", d.name.decode())
        for i in range(d.port_count):
            h = d.hints[i]
            print("  ", d.port_desc[i], d.port_names[i].decode(), h.hints, h.lower, h.upper)
