//! ITU-R BS.1770-4 integrated loudness (LUFS) and true peak (dBTP).

use crate::fir;

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    pub b: [f64; 3],
    /// `a[0]` is 1.
    pub a: [f64; 3],
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn new(b: [f64; 3], a: [f64; 3]) -> Self {
        Self { b, a, z1: 0.0, z2: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z1;
        self.z1 = self.b[1] * x - self.a[1] * y + self.z2;
        self.z2 = self.b[2] * x - self.a[2] * y;
        y
    }
}

/// The two K-weighting stages for any sample rate. The analog parameters
/// reproduce the BS.1770 48 kHz coefficient table (checked in the tests).
pub fn k_weighting(fs: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    // Stage 1: high shelf (head acoustics).
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad::new(
        [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    );
    // Stage 2: RLB high-pass. BS.1770 leaves the numerator unnormalised.
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let hp = Biquad::new([1.0, -2.0, 1.0], [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0]);
    [shelf, hp]
}

/// BS.1770 channel weight; 5.1 order is L R C LFE Ls Rs.
fn channel_weight(index: usize, n_channels: usize) -> f64 {
    match (n_channels, index) {
        (6, 3) => 0.0,
        (6, 4) | (6, 5) => 1.41,
        _ => 1.0,
    }
}

/// Gated integrated loudness in LUFS. Returns -inf for silence or for
/// signals shorter than one 400 ms block.
pub fn integrated_loudness(channels: &[Vec<f32>], fs: f64) -> f64 {
    let len = channels.first().map_or(0, |c| c.len());
    let block = (0.4 * fs).round() as usize;
    let step = (0.1 * fs).round() as usize;
    if len < block || block == 0 {
        return f64::NEG_INFINITY;
    }

    // Prefix sums of K-weighted energy, one per channel.
    let prefix: Vec<Vec<f64>> = channels
        .iter()
        .map(|ch| {
            let [mut s1, mut s2] = k_weighting(fs);
            let mut acc = 0.0;
            let mut p = Vec::with_capacity(ch.len() + 1);
            p.push(0.0);
            for &x in ch {
                let y = s2.process(s1.process(x as f64));
                acc += y * y;
                p.push(acc);
            }
            p
        })
        .collect();

    let n_ch = channels.len();
    let blocks: Vec<f64> = (0..=(len - block) / step)
        .map(|j| {
            let s = j * step;
            (0..n_ch)
                .map(|c| channel_weight(c, n_ch) * (prefix[c][s + block] - prefix[c][s]) / block as f64)
                .sum()
        })
        .collect();

    let lufs = |z: f64| -0.691 + 10.0 * z.log10();
    let mean_above = |gate: f64| {
        let (sum, n) = blocks
            .iter()
            .filter(|&&z| lufs(z) > gate)
            .fold((0.0, 0usize), |(s, n), &z| (s + z, n + 1));
        if n == 0 { 0.0 } else { sum / n as f64 }
    };

    let abs_mean = mean_above(-70.0);
    if abs_mean == 0.0 {
        return f64::NEG_INFINITY;
    }
    let rel_gate = lufs(abs_mean) - 10.0;
    lufs(mean_above(rel_gate.max(-70.0)))
}

/// Oversampling interpolator for true-peak metering (BS.1770 Annex 2).
pub struct TruePeakMeter {
    factor: usize,
    /// Per phase, taps reversed so each output is a forward dot product.
    phases: Vec<Vec<f32>>,
}

const TP_TAPS_PER_PHASE: usize = 32;

impl TruePeakMeter {
    pub fn new(fs: f64) -> Self {
        let factor = if fs <= 48_500.0 { 4 } else if fs <= 97_000.0 { 2 } else { 1 };
        let n = factor * TP_TAPS_PER_PHASE;
        let window = fir::kaiser_window(n + 1, 9.0);
        let m = n as f64 / 2.0;
        let fc = 0.5 / factor as f64;
        let proto: Vec<f64> = (0..n)
            .map(|i| {
                let k = i as f64 - m;
                let sinc = if k == 0.0 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * k).sin() / (std::f64::consts::PI * k) };
                sinc * window[i] * factor as f64
            })
            .collect();
        let phases = (0..factor)
            .map(|p| {
                let mut taps: Vec<f32> = (0..TP_TAPS_PER_PHASE).map(|k| proto[p + k * factor] as f32).collect();
                taps.reverse();
                taps
            })
            .collect();
        Self { factor, phases }
    }

    pub fn factor(&self) -> usize {
        self.factor
    }

    /// Largest absolute interpolated sample value (linear).
    pub fn peak(&self, x: &[f32]) -> f32 {
        let sample_peak = x.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        if self.factor == 1 {
            return sample_peak;
        }
        let t = TP_TAPS_PER_PHASE;
        let mut padded = vec![0.0f32; x.len() + 2 * t];
        padded[t..t + x.len()].copy_from_slice(x);
        let mut peak = sample_peak;
        for win in padded.windows(t) {
            for ph in &self.phases {
                peak = peak.max(dot(win, ph).abs());
            }
        }
        peak
    }
}

/// Dot product with eight independent accumulators, so it vectorises.
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let ((ca, ra), (cb, rb)) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    let tail: f32 = ra.iter().zip(rb).map(|(x, y)| x * y).sum();
    for (xa, xb) in ca.iter().zip(cb) {
        for i in 0..8 {
            acc[i] += xa[i] * xb[i];
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// True peak over all channels, dBTP.
pub fn true_peak_db(channels: &[Vec<f32>], fs: f64) -> f64 {
    let meter = TruePeakMeter::new(fs);
    let peak = std::thread::scope(|s| {
        let handles: Vec<_> = channels.iter().map(|ch| s.spawn(|| meter.peak(ch))).collect();
        handles.into_iter().map(|h| h.join().expect("meter thread")).fold(0.0f32, f32::max)
    });
    20.0 * (peak as f64).max(1e-30).log10()
}
