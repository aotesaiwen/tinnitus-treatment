//! Notch specification and verified linear-phase FIR design.
//!
//! The notch follows tailor-made notched music training (TMNMT): a band
//! `width_oct` octaves wide, centred geometrically on the tinnitus frequency,
//! is removed. The whole nominal band is stopband; the transition bands sit
//! *outside* it, so no part of the specified octave survives.
//!
//! The design loop measures the actual frequency response and lengthens the
//! filter until the specification is met, rather than trusting Kaiser's
//! length formula.

use serde::Serialize;

use crate::fir;

#[derive(Clone, Debug, Serialize)]
pub struct NotchSpec {
    /// Notch centres in Hz (usually one: the matched tinnitus pitch).
    pub centers_hz: Vec<f64>,
    /// Width of each removed band in octaves (TMNMT uses 1.0).
    pub width_oct: f64,
    /// Width of each transition band in octaves, measured at the lower edge.
    pub transition_oct: f64,
    /// Minimum attenuation anywhere inside the nominal band(s), dB.
    pub stop_atten_db: f64,
    /// Maximum gain deviation anywhere in the passband(s), dB.
    pub pass_ripple_db: f64,
}

impl Default for NotchSpec {
    fn default() -> Self {
        Self {
            centers_hz: Vec::new(),
            width_oct: 1.0,
            transition_oct: 1.0 / 24.0,
            stop_atten_db: 60.0,
            pass_ripple_db: 0.1,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct Band {
    pub lo_hz: f64,
    pub hi_hz: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct NotchDesign {
    pub fs: f64,
    pub spec: NotchSpec,
    /// Nominal removed bands after merging overlaps, Hz.
    pub stop_bands: Vec<Band>,
    /// Bands where the gain must be unity within the ripple spec, Hz.
    pub pass_bands: Vec<Band>,
    pub transition_hz: f64,
    pub n_taps: usize,
    /// Group delay of the linear-phase filter, samples. Offline processing
    /// removes it, so output stays sample-aligned with input.
    pub delay_samples: usize,
    pub kaiser_beta: f64,
    pub design_atten_db: f64,
    /// Measured: minimum attenuation inside the stop bands, dB.
    pub achieved_stop_atten_db: f64,
    /// Measured: largest |gain| in the pass bands, dB.
    pub achieved_pass_ripple_db: f64,
    pub iterations: usize,
    #[serde(skip)]
    pub taps: Vec<f64>,
}

impl NotchDesign {
    pub fn taps_f32(&self) -> Vec<f32> {
        self.taps.iter().map(|&t| t as f32).collect()
    }

    pub fn meets_spec(&self) -> bool {
        self.achieved_stop_atten_db >= self.spec.stop_atten_db
            && self.achieved_pass_ripple_db <= self.spec.pass_ripple_db
    }

    /// (frequency Hz, gain dB) on a dense grid, for plotting.
    pub fn response_db(&self) -> Vec<(f64, f64)> {
        let nfft = grid_len(self.taps.len());
        fir::magnitude_grid(&self.taps, nfft)
            .iter()
            .enumerate()
            .map(|(k, &m)| (k as f64 * self.fs / nfft as f64, 20.0 * m.max(1e-300).log10()))
            .collect()
    }
}

/// About 5.5 s of delay at 48 kHz; far beyond any sensible notch.
const MAX_TAPS: usize = 1 << 18;

fn grid_len(n_taps: usize) -> usize {
    (8 * n_taps).next_power_of_two().max(1 << 16)
}

pub fn design(spec: &NotchSpec, fs: f64) -> Result<NotchDesign, String> {
    let nyq = fs / 2.0;
    if spec.centers_hz.is_empty() {
        return Err("at least one notch centre is required".into());
    }
    if !(spec.width_oct > 0.0 && spec.width_oct <= 4.0) {
        return Err(format!("width_oct must be in (0, 4], got {}", spec.width_oct));
    }
    if !(spec.transition_oct > 0.0 && spec.transition_oct <= 0.5) {
        return Err(format!("transition_oct must be in (0, 0.5], got {}", spec.transition_oct));
    }
    if !(20.0..=150.0).contains(&spec.stop_atten_db) {
        return Err(format!("stop_atten_db must be in [20, 150], got {}", spec.stop_atten_db));
    }
    if !(spec.pass_ripple_db > 0.0) {
        return Err("pass_ripple_db must be positive".into());
    }

    let half = 2f64.powf(spec.width_oct / 2.0);
    let mut bands: Vec<Band> = Vec::new();
    for &c in &spec.centers_hz {
        let b = Band { lo_hz: c / half, hi_hz: (c * half).min(nyq) };
        if !(c.is_finite() && c > 0.0) || c >= nyq {
            return Err(format!("notch centre {c} Hz must be between 0 and Nyquist ({nyq} Hz)"));
        }
        bands.push(b);
    }
    bands.sort_by(|a, b| a.lo_hz.total_cmp(&b.lo_hz));

    let tr = bands
        .iter()
        .map(|b| b.lo_hz * (1.0 - 2f64.powf(-spec.transition_oct)))
        .fold(f64::INFINITY, f64::min);
    if bands[0].lo_hz < 20.0 {
        return Err(format!("notch lower edge {:.1} Hz is below the audible range (20 Hz)", bands[0].lo_hz));
    }

    // Merge bands whose transitions would collide: the gap could not hold a
    // valid passband anyway.
    let mut stop: Vec<Band> = Vec::new();
    for b in bands {
        match stop.last_mut() {
            Some(prev) if prev.hi_hz + tr >= b.lo_hz - tr => prev.hi_hz = prev.hi_hz.max(b.hi_hz),
            _ => stop.push(b),
        }
    }

    let mut pass = Vec::new();
    let mut start = 0.0;
    for b in &stop {
        pass.push(Band { lo_hz: start, hi_hz: b.lo_hz - tr });
        start = b.hi_hz + tr;
    }
    if start < nyq {
        pass.push(Band { lo_hz: start, hi_hz: nyq });
    }

    let cutoffs: Vec<(f64, f64)> = stop
        .iter()
        .map(|b| {
            let hi = if b.hi_hz + tr / 2.0 >= nyq { 0.5 } else { (b.hi_hz + tr / 2.0) / fs };
            ((b.lo_hz - tr / 2.0) / fs, hi)
        })
        .collect();

    let mut a_design = spec.stop_atten_db + 1.0;
    let mut n = fir::kaiser_len(a_design, tr / fs);
    for iteration in 1..=40 {
        if n > MAX_TAPS {
            return Err(format!(
                "spec needs more than {MAX_TAPS} taps; widen transition_oct or lower stop_atten_db"
            ));
        }
        let beta = fir::kaiser_beta(a_design);
        let window = fir::kaiser_window(n, beta);
        let taps: Vec<f64> = fir::ideal_bandstop(n, &cutoffs)
            .iter()
            .zip(&window)
            .map(|(h, w)| h * w)
            .collect();
        let (stop_att, ripple) = measure(&taps, fs, &stop, &pass);

        let design = NotchDesign {
            fs,
            spec: spec.clone(),
            stop_bands: stop.clone(),
            pass_bands: pass.clone(),
            transition_hz: tr,
            n_taps: n,
            delay_samples: n / 2,
            kaiser_beta: beta,
            design_atten_db: a_design,
            achieved_stop_atten_db: stop_att,
            achieved_pass_ripple_db: ripple,
            iterations: iteration,
            taps,
        };
        if design.meets_spec() {
            return Ok(design);
        }
        // Kaiser ripple is equal in both bands, so one knob fixes either
        // failure: ask for more attenuation, which also lengthens the filter.
        let short_db = (spec.stop_atten_db - stop_att).max(0.0);
        let ripple_needed_db = -20.0 * (10f64.powf(spec.pass_ripple_db / 20.0) - 1.0).log10();
        a_design = (a_design + short_db.max(1.0)).max(ripple_needed_db + 1.0);
        n = fir::kaiser_len(a_design, tr / fs).max(n + n / 20 + 2) | 1;
    }
    Err("filter design did not converge in 40 iterations".into())
}

/// Minimum stopband attenuation and maximum passband deviation (both dB),
/// from a dense FFT grid plus exact evaluation at every band edge.
fn measure(taps: &[f64], fs: f64, stop: &[Band], pass: &[Band]) -> (f64, f64) {
    let nfft = grid_len(taps.len());
    let mag = fir::magnitude_grid(taps, nfft);
    let hz = |k: usize| k as f64 * fs / nfft as f64;
    let db = |m: f64| 20.0 * m.max(1e-300).log10();

    let mut min_att = f64::INFINITY;
    for b in stop {
        for (k, &m) in mag.iter().enumerate() {
            let f = hz(k);
            if f >= b.lo_hz && f <= b.hi_hz {
                min_att = min_att.min(-db(m));
            }
        }
        for f in [b.lo_hz, b.hi_hz] {
            min_att = min_att.min(-db(fir::amplitude_at(taps, f / fs).abs()));
        }
    }

    let mut max_dev: f64 = 0.0;
    for b in pass {
        for (k, &m) in mag.iter().enumerate() {
            let f = hz(k);
            if f >= b.lo_hz && f <= b.hi_hz {
                max_dev = max_dev.max(db(m).abs());
            }
        }
        for f in [b.lo_hz, b.hi_hz] {
            max_dev = max_dev.max(db(fir::amplitude_at(taps, f / fs).abs()).abs());
        }
    }
    (min_att, max_dev)
}
