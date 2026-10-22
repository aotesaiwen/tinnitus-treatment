"""Offline, full-track BS.1770 calibration of the Web Audio shelf settings.
Uses the project's existing NumPy/SciPy/SoundFile environment. No model or API.
The table is approximate between grid points and excludes background sound.
"""
import hashlib
import json
from pathlib import Path
import numpy as np
import soundfile as sf
from scipy.signal import lfilter

ROOT = Path(__file__).resolve().parents[2]
AUDIO = ROOT / 'listening-demo/dist/repertoire/chopin-nocturne-op9-no2.flac'


def shelf(rate, freq, gain, high=False):
    # RBJ shelves with S=1, matching Web Audio's fixed shelf slope.
    a = 10 ** (gain / 40)
    w = 2 * np.pi * freq / rate
    c, s = np.cos(w), np.sin(w)
    t = s * np.sqrt(2 * a)
    if high:
        b = a * np.array([(a+1)+(a-1)*c+t, -2*((a-1)+(a+1)*c), (a+1)+(a-1)*c-t])
        d = np.array([(a+1)-(a-1)*c+t, 2*((a-1)-(a+1)*c), (a+1)-(a-1)*c-t])
    else:
        b = a * np.array([(a+1)-(a-1)*c+t, 2*((a-1)-(a+1)*c), (a+1)-(a-1)*c-t])
        d = np.array([(a+1)+(a-1)*c+t, -2*((a-1)+(a+1)*c), (a+1)+(a-1)*c-t])
    return b/d[0], d/d[0]


def loudness(x, rate):
    # Same BS.1770 weighting and 400/100 ms gates as src/loudness.rs.
    k = np.tan(np.pi * 1681.974450955533 / rate)
    vh = 10 ** (3.999843853973347 / 20)
    vb, q = vh ** 0.4996667741545416, 0.7071752369554196
    a0 = 1 + k/q + k*k
    b = np.array([vh+vb*k/q+k*k, 2*(k*k-vh), vh-vb*k/q+k*k])/a0
    a = [1, 2*(k*k-1)/a0, (1-k/q+k*k)/a0]
    x = lfilter(b, a, x, axis=0)
    k = np.tan(np.pi * 38.13547087602444 / rate)
    a0 = 1 + k/0.5003270373238773 + k*k
    x = lfilter([1, -2, 1], [1, 2*(k*k-1)/a0, (1-k/0.5003270373238773+k*k)/a0], x, axis=0)
    energy = np.concatenate(([0], np.cumsum(np.sum(x*x, axis=1))))
    starts = np.arange(0, len(x)-round(.4*rate)+1, round(.1*rate))
    blocks = (energy[starts+round(.4*rate)]-energy[starts])/round(.4*rate)
    absolute = blocks[blocks > 10 ** ((-70+.691)/10)]
    gate = max(10 ** ((-70+.691)/10), np.mean(absolute)*.1)
    return -.691 + 10*np.log10(np.mean(blocks[blocks > gate]))


def main():
    audio, rate = sf.read(AUDIO)
    baseline = loudness(audio, rate)
    grid = [-6, -3, 0, 3, 6]
    gains = []
    for bass in grid:
        low = lfilter(*shelf(rate, 250, bass), audio, axis=0)
        row = []
        for bright in grid:
            processed = lfilter(*shelf(rate, 2500, bright, high=True), low, axis=0)
            row.append(round(baseline-loudness(processed, rate), 5))
        gains.append(row)
    record = dict(source_sha256=hashlib.sha256(AUDIO.read_bytes()).hexdigest(),
                  method='Full-track BS.1770 gated integrated loudness, before peak guard; bilinear interpolation in dB; music only.',
                  sample_rate=rate, original_lufs=baseline, grid_db=grid,
                  bass_frequency_hz=250, brightness_frequency_hz=2500,
                  compensation_db=gains)
    output = ROOT / 'listening-demo/dist/repertoire/compensation.js'
    output.write_text('export const calibration = '+json.dumps(record, indent=2)+';\n')
    print(json.dumps(record, indent=2))


if __name__ == '__main__':
    main()
