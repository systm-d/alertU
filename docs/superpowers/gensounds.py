#!/usr/bin/env python3
"""Synthesize AlertU's warning tick and looping siren as 16-bit mono WAVs.

Prototype for the pure-Rust generator planned as lot B task B1. The siren is
built so it can be restarted back-to-back without an audible click: its
frequency modulation completes exactly one cycle over the file, its mid
frequency times the duration is an integer number of cycles (so the phase at
the end matches the phase at the start), and both ends are faded to silence.
"""
import math
import struct
import sys
import wave

RATE = 44100


def write_wav(path, samples):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1.0, min(1.0, s)) * 32767)) for s in samples))


def fade(samples, ms):
    """Fade both ends to zero, so no waveform starts or ends on a step."""
    n = int(RATE * ms / 1000)
    out = list(samples)
    for i in range(min(n, len(out) // 2)):
        g = i / n
        out[i] *= g
        out[-1 - i] *= g
    return out


def tick(duration=0.05, freq=1200.0, amp=0.18):
    """A short, dry countdown tick — present but not alarming."""
    n = int(RATE * duration)
    return fade([amp * math.sin(2 * math.pi * freq * i / RATE) for i in range(n)], 6)


def siren(duration=2.0, f_mid=1000.0, f_dev=400.0, amp=0.75):
    """A 600<->1400 Hz sweep that loops seamlessly.

    Instantaneous frequency f(t) = f_mid + f_dev*sin(2*pi*t/T) completes exactly
    one modulation cycle over T. Integrating gives the phase below; with
    f_mid*T an integer, phase(T) - phase(0) is a whole number of cycles, so
    restarting the file continues the waveform smoothly.
    """
    assert abs(f_mid * duration - round(f_mid * duration)) < 1e-9, "f_mid*T must be integral"
    n = int(RATE * duration)
    out = []
    for i in range(n):
        t = i / RATE
        phase = 2 * math.pi * f_mid * t - (f_dev * duration) * math.cos(2 * math.pi * t / duration)
        out.append(amp * math.sin(phase))
    return fade(out, 8)


if __name__ == "__main__":
    dest = sys.argv[1]
    write_wav(f"{dest}/warning.wav", tick())
    write_wav(f"{dest}/siren.wav", siren())
    print(f"wrote {dest}/warning.wav and {dest}/siren.wav")
