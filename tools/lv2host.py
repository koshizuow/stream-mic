"""Minimal offline LV2 host over the system liblilv (ctypes), for plugins lv2apply cannot run
(atom/event ports, plugins that need urid:map).

Usage: uv run python lv2host.py URI IN.wav OUT.wav [sym=value ...]
Input channels are mapped to the plugin's audio inputs in order (a mono file feeds every input).
"""
import ctypes as C
import sys
import time

import numpy as np
import soundfile as sf

lilv = C.CDLL("liblilv-0.so.0")
P = C.c_void_p
for name, res, args in [
    ("lilv_world_new", P, []),
    ("lilv_world_load_all", None, [P]),
    ("lilv_world_get_all_plugins", P, [P]),
    ("lilv_new_uri", P, [P, C.c_char_p]),
    ("lilv_plugins_get_by_uri", P, [P, P]),
    ("lilv_plugin_get_num_ports", C.c_uint32, [P]),
    ("lilv_plugin_get_port_by_index", P, [P, C.c_uint32]),
    ("lilv_port_is_a", C.c_bool, [P, P, P]),
    ("lilv_port_get_symbol", P, [P, P]),
    ("lilv_node_as_string", C.c_char_p, [P]),
    ("lilv_plugin_get_port_ranges_float", None, [P, P, P, P]),
    ("lilv_plugin_instantiate", P, [P, C.c_double, P]),
    ("lilv_instance_free", None, [P]),
]:
    f = getattr(lilv, name)
    f.restype = res
    f.argtypes = args


class Descriptor(C.Structure):
    _fields_ = [("URI", C.c_char_p), ("instantiate", P),
                ("connect_port", C.CFUNCTYPE(None, P, C.c_uint32, P)),
                ("activate", C.CFUNCTYPE(None, P)), ("run", C.CFUNCTYPE(None, P, C.c_uint32)),
                ("deactivate", C.CFUNCTYPE(None, P)), ("cleanup", P), ("extension_data", P)]


class Instance(C.Structure):
    _fields_ = [("lv2_descriptor", C.POINTER(Descriptor)), ("lv2_handle", P), ("pimpl", P)]


class Feature(C.Structure):
    _fields_ = [("URI", C.c_char_p), ("data", P)]


RESPOND_FN = C.CFUNCTYPE(C.c_int, P, C.c_uint32, P)
SCHEDULE_FN = C.CFUNCTYPE(C.c_int, P, C.c_uint32, P)


class WorkerSchedule(C.Structure):
    _fields_ = [("handle", P), ("schedule_work", SCHEDULE_FN)]


class WorkerInterface(C.Structure):
    _fields_ = [("work", C.CFUNCTYPE(C.c_int, P, RESPOND_FN, P, C.c_uint32, P)),
                ("work_response", C.CFUNCTYPE(C.c_int, P, C.c_uint32, P)),
                ("end_run", C.CFUNCTYPE(C.c_int, P))]


_work_queue = []
_responses = []


def _schedule(_h, size, data):
    _work_queue.append(C.string_at(data, size))
    return 0


def _respond(_h, size, data):
    _responses.append(C.string_at(data, size))
    return 0


class Option(C.Structure):
    _fields_ = [("context", C.c_uint32), ("subject", C.c_uint32), ("key", C.c_uint32),
                ("size", C.c_uint32), ("type", C.c_uint32), ("value", P)]


MAP_FN = C.CFUNCTYPE(C.c_uint32, P, C.c_char_p)
UNMAP_FN = C.CFUNCTYPE(C.c_char_p, P, C.c_uint32)


class URIDMap(C.Structure):
    _fields_ = [("handle", P), ("map", MAP_FN)]


class URIDUnmap(C.Structure):
    _fields_ = [("handle", P), ("unmap", UNMAP_FN)]


_uris = {}
_names = {}


def _map(_h, uri):
    if uri not in _uris:
        _uris[uri] = len(_uris) + 1
        _names[_uris[uri]] = C.create_string_buffer(uri)
    return _uris[uri]


def _unmap(_h, urid):
    buf = _names.get(urid)
    return C.cast(buf, C.c_char_p).value if buf else None


LV2 = "http://lv2plug.in/ns/"


class LV2Plugin:
    """One plugin instance that runs block by block (for chains). Mono in and out: the input feeds every audio
    input, the first audio output is returned. latency() reads the plugin's reported latency port."""

    def __init__(self, uri, fs, controls, block=512):
        controls = dict(controls)
        world = lilv.lilv_world_new()
        lilv.lilv_world_load_all(world)
        plugin = lilv.lilv_plugins_get_by_uri(lilv.lilv_world_get_all_plugins(world), lilv.lilv_new_uri(world, uri.encode()))
        if not plugin:
            raise SystemExit(f"plugin not found: {uri}")
        cls = {k: lilv.lilv_new_uri(world, (LV2 + v).encode()) for k, v in {
            "audio": "lv2core#AudioPort", "control": "lv2core#ControlPort", "input": "lv2core#InputPort",
            "output": "lv2core#OutputPort", "atom": "ext/atom#AtomPort", "cv": "lv2core#CVPort"}.items()}
        self.map_fn, self.unmap_fn = MAP_FN(_map), UNMAP_FN(_unmap)
        self.umap, self.uunmap = URIDMap(None, self.map_fn), URIDUnmap(None, self.unmap_fn)
        # Options (DPF plugins refuse to start without them): block lengths and sample rate.
        int_t = _map(None, b"http://lv2plug.in/ns/ext/atom#Int")
        float_t = _map(None, b"http://lv2plug.in/ns/ext/atom#Float")
        self.vals = [C.c_int32(block), C.c_int32(block), C.c_int32(block), C.c_float(fs)]
        keys = [b"http://lv2plug.in/ns/ext/buf-size#maxBlockLength", b"http://lv2plug.in/ns/ext/buf-size#minBlockLength",
                b"http://lv2plug.in/ns/ext/buf-size#nominalBlockLength", b"http://lv2plug.in/ns/lv2core#sampleRate"]
        self.opts = (Option * 5)()
        for k, (key, v) in enumerate(zip(keys, self.vals)):
            self.opts[k] = Option(0, 0, _map(None, key), 4, float_t if isinstance(v, C.c_float) else int_t,
                                  C.cast(C.pointer(v), P))
        self.opts[4] = Option(0, 0, 0, 0, 0, None)
        self.feats = [Feature(b"http://lv2plug.in/ns/ext/urid#map", C.cast(C.pointer(self.umap), P)),
                      Feature(b"http://lv2plug.in/ns/ext/urid#unmap", C.cast(C.pointer(self.uunmap), P)),
                      Feature(b"http://lv2plug.in/ns/ext/options#options", C.cast(self.opts, P)),
                      Feature(b"http://lv2plug.in/ns/ext/buf-size#boundedBlockLength", None)]
        self.schedule_fn, self.respond_fn = SCHEDULE_FN(_schedule), RESPOND_FN(_respond)
        self.sched = WorkerSchedule(None, self.schedule_fn)
        self.feats.append(Feature(b"http://lv2plug.in/ns/ext/worker#schedule", C.cast(C.pointer(self.sched), P)))
        self.feat_arr = (C.POINTER(Feature) * (len(self.feats) + 1))(*[C.pointer(f) for f in self.feats], None)

        self.inst_p = lilv.lilv_plugin_instantiate(plugin, C.c_double(fs), C.cast(self.feat_arr, P))
        if not self.inst_p:
            raise SystemExit("instantiate failed")
        inst = C.cast(self.inst_p, C.POINTER(Instance)).contents
        self.desc = desc = inst.lv2_descriptor.contents
        self.h = h = inst.lv2_handle
        self.worker = None
        if desc.extension_data:
            ext = C.CFUNCTYPE(P, C.c_char_p)(desc.extension_data)(b"http://lv2plug.in/ns/ext/worker#interface")
            if ext:
                self.worker = C.cast(ext, C.POINTER(WorkerInterface)).contents

        n = lilv.lilv_plugin_get_num_ports(plugin)
        mins, maxs, defs = (C.c_float * n)(), (C.c_float * n)(), (C.c_float * n)()
        lilv.lilv_plugin_get_port_ranges_float(plugin, mins, maxs, defs)
        self.keep, self.ins, self.outs, self.atom_in, self.atom_out = [], [], [], [], []
        self.lat_port = None
        self.seq_type = _map(None, b"http://lv2plug.in/ns/ext/atom#Sequence")
        self.chunk_type = _map(None, b"http://lv2plug.in/ns/ext/atom#Chunk")
        for i in range(n):
            port = lilv.lilv_plugin_get_port_by_index(plugin, i)
            sym = lilv.lilv_node_as_string(lilv.lilv_port_get_symbol(plugin, port)).decode()
            is_in = lilv.lilv_port_is_a(plugin, port, cls["input"])
            if lilv.lilv_port_is_a(plugin, port, cls["audio"]) or lilv.lilv_port_is_a(plugin, port, cls["cv"]):
                buf = np.zeros(block, dtype=np.float32)
                (self.ins if is_in else self.outs).append((sym, buf))
                self.keep.append(buf)
                desc.connect_port(h, i, buf.ctypes.data)
            elif lilv.lilv_port_is_a(plugin, port, cls["control"]):
                val = defs[i] if not np.isnan(defs[i]) else 0.0
                if is_in and sym in controls:
                    val = float(controls.pop(sym))
                buf = np.array([val], dtype=np.float32)
                self.keep.append(buf)
                if not is_in and "latency" in sym:
                    self.lat_port = buf
                desc.connect_port(h, i, buf.ctypes.data)
            elif lilv.lilv_port_is_a(plugin, port, cls["atom"]):
                buf = np.zeros(8192 // 4, dtype=np.uint32)
                self.keep.append(buf)
                (self.atom_in if is_in else self.atom_out).append(buf)
                desc.connect_port(h, i, buf.ctypes.data)
            else:
                desc.connect_port(h, i, None)
        if controls:
            raise SystemExit(f"unknown controls: {sorted(controls)}")
        desc.activate(h)

    def run_worker(self):
        # Synchronous worker: offline rendering has no real-time constraint.
        while _work_queue:
            data = _work_queue.pop(0)
            buf = C.create_string_buffer(data, len(data))
            self.worker.work(self.h, self.respond_fn, None, len(data), buf)
            while _responses:
                r = _responses.pop(0)
                rb = C.create_string_buffer(r, len(r))
                self.worker.work_response(self.h, len(r), rb)
        if self.worker and self.worker.end_run:
            self.worker.end_run(self.h)

    def run(self, xin, timer=None):
        """xin: (m, channels) float32. Returns (m, outputs). timer: list to add the run() seconds to."""
        m = len(xin)
        for k, (_, buf) in enumerate(self.ins):
            buf[:m] = xin[:, k % xin.shape[1]]
        for buf in self.atom_in:   # empty sequence: header size 8 (unit, pad), type Sequence
            buf[0], buf[1], buf[2], buf[3] = 8, self.seq_type, 0, 0
        for buf in self.atom_out:  # capacity for the plugin to write into
            buf[0], buf[1] = buf.nbytes - 8, self.chunk_type
        t = time.perf_counter()
        self.desc.run(self.h, m)
        if timer is not None:
            timer[0] += time.perf_counter() - t
        if self.worker:
            self.run_worker()
        return np.stack([buf[:m] for _, buf in self.outs], 1)

    def latency(self):
        return int(self.lat_port[0]) if self.lat_port is not None else 0

    def close(self):
        self.desc.deactivate(self.h)
        lilv.lilv_instance_free(self.inst_p)


def process(uri, x, fs, controls, block=512, tail=0.0):
    p = LV2Plugin(uri, fs, controls, block)
    if x.ndim == 1:
        x = x[:, None]
    total = len(x) + int(tail * fs)
    xin = np.zeros((total, x.shape[1]), dtype=np.float32)
    xin[: len(x)] = x
    y = np.zeros((total, len(p.outs)), dtype=np.float32)
    for pos in range(0, total, block):
        m = min(block, total - pos)
        y[pos:pos + m] = p.run(xin[pos:pos + m])
    p.close()
    return y


def main():
    uri, src, dst = sys.argv[1:4]
    controls = dict(a.split("=", 1) for a in sys.argv[4:])
    tail = float(controls.pop("_tail", 0))
    x, fs = sf.read(src, dtype="float32")
    y = process(uri, x, fs, controls, tail=tail)
    sf.write(dst, y, fs, subtype="FLOAT")


if __name__ == "__main__":
    main()
