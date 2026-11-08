"""Reference peaking EQ designs, as frequency responses (no audio needed).

analog : the prototype H(s) = (s^2 + s w0 sqrt(G)/Q + w0^2) / (s^2 + s w0/(sqrt(G) Q) + w0^2), G linear peak gain
rbj    : RBJ cookbook (bilinear transform, prewarped)
matched: Vicanek 2016 section 4.4 (impulse-invariant poles, magnitude matched at DC and f0)

Usage: uv run python refeq.py F0 GAIN_DB Q [FS]
"""
import sys

import numpy as np
from scipy import signal


def analog(f, f0, gain_db, q):
    G = 10 ** (gain_db / 20)
    s = 1j * f / f0
    h = (s ** 2 + s * np.sqrt(G) / q + 1) / (s ** 2 + s / (np.sqrt(G) * q) + 1)
    return 20 * np.log10(np.abs(h))


def rbj(f0, gain_db, q, fs):
    A = 10 ** (gain_db / 40)
    w0 = 2 * np.pi * f0 / fs
    alpha = np.sin(w0) / (2 * q)
    b = [1 + alpha * A, -2 * np.cos(w0), 1 - alpha * A]
    a = [1 + alpha / A, -2 * np.cos(w0), 1 - alpha / A]
    return np.array(b) / a[0], np.array(a) / a[0]


def matched(f0, gain_db, q, fs):
    G = 10 ** (gain_db / 20)
    w0 = 2 * np.pi * f0 / fs
    # The prototype's poles have q_pole = 1/(2 sqrt(G) Q).
    qq = 1 / (2 * np.sqrt(G) * q)
    if qq <= 1:
        a1 = -2 * np.exp(-qq * w0) * np.cos(np.sqrt(1 - qq * qq) * w0)
    else:
        a1 = -2 * np.exp(-qq * w0) * np.cosh(np.sqrt(qq * qq - 1) * w0)
    a2 = np.exp(-2 * qq * w0)
    phi1 = np.sin(w0 / 2) ** 2
    phi0 = 1 - phi1
    phi2 = 4 * phi0 * phi1
    A0 = (1 + a1 + a2) ** 2
    A1 = (1 - a1 + a2) ** 2
    A2 = -4 * a2
    B0 = A0
    R1 = (A0 * phi0 + A1 * phi1 + A2 * phi2) * G * G
    R2 = (-A0 + A1 + 4 * (phi0 - phi1) * A2) * G * G
    B2 = (R1 - R2 * phi1 - B0) / (4 * phi1 * phi1)
    B1 = R2 + B0 + 4 * (phi1 - phi0) * B2
    W = 0.5 * (np.sqrt(B0) + np.sqrt(B1))
    b0 = 0.5 * (W + np.sqrt(W * W + B2))
    b1 = 0.5 * (np.sqrt(B0) - np.sqrt(B1))
    b2 = -B2 / (4 * b0)
    return np.array([b0, b1, b2]), np.array([1, a1, a2])


def response_db(ba, f, fs):
    _, h = signal.freqz(*ba, worN=f, fs=fs)
    return 20 * np.log10(np.abs(h))


def main():
    f0, g, q = (float(v) for v in sys.argv[1:4])
    fs = float(sys.argv[4]) if len(sys.argv) > 4 else 48000.0
    f = np.array([1000, 2000, 4000, 6000, 8000, 10000, 12000, 14000, 16000, 18000, 20000, 22000, 23500], float)
    an, rb, ma = analog(f, f0, g, q), response_db(rbj(f0, g, q, fs), f, fs), response_db(matched(f0, g, q, fs), f, fs)
    print("    Hz  analog     rbj  matched")
    for i, fr in enumerate(f):
        print(f"{fr:6.0f} {an[i]:7.2f} {rb[i]:7.2f} {ma[i]:7.2f}")


if __name__ == "__main__":
    main()
