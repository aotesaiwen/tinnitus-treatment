//! Linear-phase FIR design by the Kaiser window method.
//!
//! Frequencies in this module are normalised: cycles per sample, 0 to 0.5.

use std::f64::consts::PI;

use realfft::RealFftPlanner;

/// Zeroth-order modified Bessel function of the first kind, by power series.
pub fn bessel_i0(x: f64) -> f64 {
    let q = 0.25 * x * x;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut k = 1.0;
    loop {
        term *= q / (k * k);
        sum += term;
        if term < sum * 1e-17 {
            return sum;
        }
        k += 1.0;
    }
}

/// Kaiser's empirical beta for a given stopband attenuation (dB).
pub fn kaiser_beta(atten_db: f64) -> f64 {
    if atten_db > 50.0 {
        0.1102 * (atten_db - 8.7)
    } else if atten_db >= 21.0 {
        0.5842 * (atten_db - 21.0).powf(0.4) + 0.07886 * (atten_db - 21.0)
    } else {
        0.0
    }
}

/// Kaiser's length estimate for attenuation `atten_db` and a transition band
/// `transition` wide (normalised). Always odd, so the filter is type I and can
/// pass both DC and Nyquist.
pub fn kaiser_len(atten_db: f64, transition: f64) -> usize {
    let n = ((atten_db - 7.95) / (2.285 * 2.0 * PI * transition)).ceil().max(2.0) as usize + 1;
    n | 1
}

pub fn kaiser_window(n: usize, beta: f64) -> Vec<f64> {
    if n == 1 {
        return vec![1.0];
    }
    let m = (n - 1) as f64;
    let norm = bessel_i0(beta);
    (0..n)
        .map(|i| {
            let r = 2.0 * i as f64 / m - 1.0;
            bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / norm
        })
        .collect()
}

fn lowpass_tap(fc: f64, k: f64) -> f64 {
    if k == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * k).sin() / (PI * k) }
}

/// Ideal (unwindowed) band-stop impulse response of odd length `n` that
/// removes each interval `(lo, hi)`. An interval reaching 0.5 removes
/// everything above `lo`.
pub fn ideal_bandstop(n: usize, stops: &[(f64, f64)]) -> Vec<f64> {
    assert!(n % 2 == 1, "type I filter needs an odd length");
    let m = (n / 2) as isize;
    (0..n as isize)
        .map(|i| {
            let k = (i - m) as f64;
            let mut h = if i == m { 1.0 } else { 0.0 };
            for &(lo, hi) in stops {
                h -= lowpass_tap(hi.min(0.5), k) - lowpass_tap(lo.max(0.0), k);
            }
            h
        })
        .collect()
}

/// Exact zero-phase amplitude of a symmetric odd-length filter at frequency `f`.
/// |H(f)| is the absolute value of this.
pub fn amplitude_at(h: &[f64], f: f64) -> f64 {
    let m = h.len() / 2;
    let w = 2.0 * PI * f;
    let mut a = h[m];
    for k in 1..=m {
        a += 2.0 * h[m + k] * (w * k as f64).cos();
    }
    a
}

/// |H| on an `nfft`-point grid, bins 0..=nfft/2 (bin k is k/nfft cycles/sample).
pub fn magnitude_grid(h: &[f64], nfft: usize) -> Vec<f64> {
    assert!(nfft >= h.len());
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(nfft);
    let mut buf = fft.make_input_vec();
    buf[..h.len()].copy_from_slice(h);
    let mut spec = fft.make_output_vec();
    fft.process(&mut buf, &mut spec).expect("fft sizes match");
    spec.iter().map(|c| c.norm()).collect()
}
