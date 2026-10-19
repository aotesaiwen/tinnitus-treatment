//! Calibrated test sounds for tinnitus matching.

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::signals::Rng;

/// Maps presentation levels to digital levels for one transducer.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Calibration {
    /// (Hz, dB SPL measured in a coupler for a full-scale sine). With no
    /// points the system is uncalibrated and levels are dB re full scale.
    pub points: Vec<(f64, f64)>,
}

impl Calibration {
    pub fn is_calibrated(&self) -> bool {
        !self.points.is_empty()
    }

    pub fn unit(&self) -> &'static str {
        if self.is_calibrated() { "dB SPL" } else { "dB FS (uncalibrated)" }
    }

    /// Level a full-scale sine produces at `hz`, interpolated in log frequency.
    pub fn offset_db(&self, hz: f64) -> f64 {
        let mut p = self.points.clone();
        p.sort_by(|a, b| a.0.total_cmp(&b.0));
        match p.as_slice() {
            [] => 0.0,
            [first, ..] if hz <= first.0 => first.1,
            [.., last] if hz >= last.0 => last.1,
            _ => {
                let i = p.windows(2).position(|w| hz >= w[0].0 && hz < w[1].0).expect("hz inside range");
                let (a, b) = (p[i], p[i + 1]);
                let t = (hz / a.0).log2() / (b.0 / a.0).log2();
                a.1 + t * (b.1 - a.1)
            }
        }
    }

    pub fn to_dbfs(&self, level_db: f64, hz: f64) -> f64 {
        level_db - self.offset_db(hz)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sound {
    Tone { hz: f64 },
    /// Three 250 ms pulses. Pulsed tones are easier to tell apart from
    /// tinnitus when measuring thresholds.
    PulsedTone { hz: f64 },
    NarrowNoise { hz: f64, width_oct: f64 },
    /// White noise, 125 Hz to 16 kHz (or 0.45 fs).
    BroadNoise,
}

impl Sound {
    /// Frequency used for the calibration lookup.
    pub fn reference_hz(&self) -> f64 {
        match *self {
            Sound::Tone { hz } | Sound::PulsedTone { hz } | Sound::NarrowNoise { hz, .. } => hz,
            Sound::BroadNoise => 1000.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stimulus {
    pub sound: Sound,
    /// Calibration units. Noise is scaled to the RMS of a sine at this level.
    pub level_db: f64,
}

const RAMP_S: f64 = 0.02;
const NOISE_RAMP_S: f64 = 0.05;
const TONE_S: f64 = 1.0;
const NOISE_S: f64 = 1.5;
const PULSE_ON_S: f64 = 0.25;
const PULSE_OFF_S: f64 = 0.15;
const PULSES: usize = 3;
pub const PAIR_GAP_S: f64 = 0.5;

/// Turns stimulus specs into samples, refusing anything above the ceiling.
pub struct Renderer {
    pub fs: f64,
    pub calibration: Calibration,
    pub max_level_db: f64,
    rng: Rng,
}

impl Renderer {
    pub fn new(fs: f64, calibration: Calibration, max_level_db: f64, seed: u64) -> Self {
        Self { fs, calibration, max_level_db, rng: Rng::new(seed) }
    }

    pub fn render(&mut self, s: &Stimulus) -> Result<Vec<f32>, String> {
        if !(s.level_db <= self.max_level_db + 1e-9) {
            return Err(format!("{:.1} dB exceeds the {:.1} dB ceiling", s.level_db, self.max_level_db));
        }
        let fs = self.fs;
        let amp = 10f64.powf(self.calibration.to_dbfs(s.level_db, s.sound.reference_hz()) / 20.0);
        let x = match s.sound {
            Sound::Tone { hz } => tone(fs, hz, TONE_S, amp),
            Sound::PulsedTone { hz } => {
                let gap = vec![0.0; (PULSE_OFF_S * fs).round() as usize];
                let pulse = tone(fs, hz, PULSE_ON_S, amp);
                let mut x = Vec::new();
                for i in 0..PULSES {
                    if i > 0 {
                        x.extend_from_slice(&gap);
                    }
                    x.extend_from_slice(&pulse);
                }
                x
            }
            Sound::NarrowNoise { hz, width_oct } => {
                let half = 2f64.powf(width_oct / 2.0);
                band_noise(&mut self.rng, fs, hz / half, (hz * half).min(0.49 * fs), NOISE_S, amp / 2f64.sqrt())
            }
            Sound::BroadNoise => band_noise(&mut self.rng, fs, 125.0, (16_000f64).min(0.45 * fs), NOISE_S, amp / 2f64.sqrt()),
        };
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if peak > 1.0 {
            return Err(format!("stimulus would clip (peak {peak:.2} of full scale)"));
        }
        Ok(x)
    }

    /// One stimulus, or two separated by a short gap (for 2AFC).
    pub fn render_all(&mut self, stimuli: &[Stimulus]) -> Result<Vec<f32>, String> {
        let mut out = Vec::new();
        for (i, s) in stimuli.iter().enumerate() {
            if i > 0 {
                out.extend(std::iter::repeat_n(0.0, (PAIR_GAP_S * self.fs).round() as usize));
            }
            out.extend(self.render(s)?);
        }
        Ok(out)
    }
}

/// Raised-cosine onset and offset, so no click spreads energy across the spectrum.
fn ramp(x: &mut [f32], n: usize) {
    let n = n.min(x.len() / 2);
    let len = x.len();
    for i in 0..n {
        let g = (0.5 - 0.5 * (std::f64::consts::PI * i as f64 / n as f64).cos()) as f32;
        x[i] *= g;
        x[len - 1 - i] *= g;
    }
}

fn tone(fs: f64, hz: f64, secs: f64, amp: f64) -> Vec<f32> {
    let n = (secs * fs).round() as usize;
    let w = 2.0 * std::f64::consts::PI * hz / fs;
    let mut x: Vec<f32> = (0..n).map(|i| (amp * (w * i as f64).sin()) as f32).collect();
    ramp(&mut x, (RAMP_S * fs).round() as usize);
    x
}

fn gaussian(rng: &mut Rng) -> f64 {
    let u1 = 1.0 - rng.uniform();
    let u2 = rng.uniform();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// Gaussian noise with energy only between `lo` and `hi`, by inverse FFT of a
/// band-limited random spectrum. The FFT length equals the signal length,
/// so the noise is periodic and the band edges are exact.
fn band_noise(rng: &mut Rng, fs: f64, lo: f64, hi: f64, secs: f64, rms: f64) -> Vec<f32> {
    let n = ((secs * fs).round() as usize) & !1;
    let inv = RealFftPlanner::<f64>::new().plan_fft_inverse(n);
    let mut spec = inv.make_input_vec();
    let df = fs / n as f64;
    for (k, c) in spec.iter_mut().enumerate() {
        let f = k as f64 * df;
        if f >= lo && f <= hi && k > 0 && k < n / 2 {
            *c = Complex::new(gaussian(rng), gaussian(rng));
        }
    }
    let mut y = inv.make_output_vec();
    inv.process(&mut spec, &mut y).expect("fft sizes match");
    let cur = (y.iter().map(|v| v * v).sum::<f64>() / n as f64).sqrt();
    let g = if cur > 0.0 { rms / cur } else { 0.0 };
    let mut x: Vec<f32> = y.iter().map(|v| (v * g) as f32).collect();
    ramp(&mut x, (NOISE_RAMP_S * fs).round() as usize);
    x
}
