//! Spectral measurements that verify a notch on real signals.

use realfft::RealFftPlanner;
use serde::Serialize;

use crate::notch::{Band, NotchDesign};

/// One-sided power spectrum, summed over channels (Welch, 4-term
/// Blackman-Harris window, 50 % overlap). Sidelobes sit at -92 dB, so loud
/// passband content does not leak into and hide a deep notch.
pub struct Psd {
    pub fs: f64,
    pub nfft: usize,
    pub power: Vec<f64>,
}

impl Psd {
    pub fn welch(channels: &[Vec<f32>], fs: f64, nfft: usize) -> Self {
        let fft = RealFftPlanner::<f64>::new().plan_fft_forward(nfft);
        let window: Vec<f64> = (0..nfft)
            .map(|i| {
                let x = 2.0 * std::f64::consts::PI * i as f64 / nfft as f64;
                0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()
            })
            .collect();
        let mut power = vec![0.0; nfft / 2 + 1];
        let mut buf = fft.make_input_vec();
        let mut spec = fft.make_output_vec();
        let hop = nfft / 2;
        for ch in channels {
            let mut start = 0;
            loop {
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = ch.get(start + i).map_or(0.0, |&v| v as f64) * window[i];
                }
                fft.process(&mut buf, &mut spec).expect("fft sizes match");
                for (p, c) in power.iter_mut().zip(&spec) {
                    *p += c.norm_sqr();
                }
                start += hop;
                if start + nfft > ch.len() {
                    break;
                }
            }
        }
        Self { fs, nfft, power }
    }

    pub fn bin_hz(&self) -> f64 {
        self.fs / self.nfft as f64
    }

    /// Power in bins whose centre lies in [lo, hi].
    pub fn band_power(&self, lo: f64, hi: f64) -> f64 {
        let df = self.bin_hz();
        let k0 = (lo / df).ceil().max(0.0) as usize;
        let k1 = ((hi / df).floor() as usize).min(self.power.len() - 1);
        if k0 > k1 { 0.0 } else { self.power[k0..=k1].iter().sum() }
    }

    pub fn total(&self) -> f64 {
        self.power.iter().sum()
    }

    /// Upper edge of the highest third-octave band within 60 dB of the
    /// loudest one. Flags band-limited sources (e.g. a generator or codec
    /// that rolls off below the notch).
    pub fn bandwidth_hz(&self) -> f64 {
        let edge = 2f64.powf(1.0 / 6.0);
        let bands: Vec<(f64, f64)> = (-16..=13)
            .map(|k| 1000.0 * 2f64.powf(k as f64 / 3.0))
            .filter(|c| c * edge <= self.fs / 2.0)
            .map(|c| (c * edge, self.band_power(c / edge, c * edge)))
            .collect();
        let max = bands.iter().map(|b| b.1).fold(0.0, f64::max);
        bands.iter().rev().find(|b| b.1 >= max * 1e-6).map_or(0.0, |b| b.0)
    }
}

/// FFT size whose Blackman-Harris main lobe (±4 bins) fits well inside the
/// transition band, so passband energy cannot leak into the measured notch.
pub fn analysis_nfft(fs: f64, transition_hz: f64) -> usize {
    ((8.0 * fs / transition_hz).ceil() as usize).next_power_of_two().clamp(4096, 1 << 18)
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.max(1e-300).log10()
}

#[derive(Clone, Debug, Serialize)]
pub struct BandMeasurement {
    pub band: Band,
    /// Input power in the band relative to total input power, dB.
    pub input_level_db: f64,
    /// Filtered power in the band relative to total *input* power, dB.
    pub output_level_db: f64,
    /// Measured attenuation inside the band, dB.
    pub attenuation_db: f64,
    /// Power change in the half octave just below / above the notch, dB.
    /// Should be ~0: the TMNMT rationale relies on energy in these flanks.
    pub lower_flank_change_db: f64,
    pub upper_flank_change_db: f64,
    pub lower_flank_level_db: f64,
    pub upper_flank_level_db: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct NotchMeasurement {
    pub nfft: usize,
    pub bin_hz: f64,
    pub bands: Vec<BandMeasurement>,
    /// Filtered / input power over all passbands between 20 Hz and 20 kHz, dB.
    pub passband_change_db: f64,
}

/// Compares input and filtered (before any gain) spectra.
pub fn measure_notch(input: &Psd, filtered: &Psd, design: &NotchDesign) -> NotchMeasurement {
    let total = input.total();
    let tr = design.transition_hz;
    let nyq = design.fs / 2.0;
    let bands = design
        .stop_bands
        .iter()
        .map(|b| {
            let p_in = input.band_power(b.lo_hz, b.hi_hz);
            let p_out = filtered.band_power(b.lo_hz, b.hi_hz);
            let (l0, l1) = ((b.lo_hz - tr) / 2f64.sqrt(), b.lo_hz - tr);
            let (u0, u1) = ((b.hi_hz + tr).min(nyq), ((b.hi_hz + tr) * 2f64.sqrt()).min(nyq));
            BandMeasurement {
                band: *b,
                input_level_db: db(p_in / total),
                output_level_db: db(p_out / total),
                attenuation_db: db(p_in / p_out),
                lower_flank_change_db: db(filtered.band_power(l0, l1) / input.band_power(l0, l1)),
                upper_flank_change_db: if u1 > u0 {
                    db(filtered.band_power(u0, u1) / input.band_power(u0, u1))
                } else {
                    0.0
                },
                lower_flank_level_db: db(input.band_power(l0, l1) / total),
                upper_flank_level_db: if u1 > u0 { db(input.band_power(u0, u1) / total) } else { f64::NEG_INFINITY },
            }
        })
        .collect();

    let (mut p_in, mut p_out) = (0.0, 0.0);
    for b in &design.pass_bands {
        let (lo, hi) = (b.lo_hz.max(20.0), b.hi_hz.min(20_000.0));
        if hi > lo {
            p_in += input.band_power(lo, hi);
            p_out += filtered.band_power(lo, hi);
        }
    }
    NotchMeasurement {
        nfft: input.nfft,
        bin_hz: input.bin_hz(),
        bands,
        passband_change_db: db(p_out / p_in),
    }
}
