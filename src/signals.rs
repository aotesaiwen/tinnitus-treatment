//! Deterministic test signals (no external RNG, so results reproduce bit for bit).

/// xorshift64* generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub fn white_noise(n: usize, amplitude: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..n).map(|_| amplitude * (2.0 * rng.uniform() as f32 - 1.0)).collect()
}

/// Pink (1/f) noise via Paul Kellet's refined filter; accurate to ±0.05 dB
/// above ~10 Hz at 44.1/48 kHz.
pub fn pink_noise(n: usize, amplitude: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut b = [0.0f64; 7];
    (0..n)
        .map(|_| {
            let w = 2.0 * rng.uniform() - 1.0;
            b[0] = 0.99886 * b[0] + w * 0.0555179;
            b[1] = 0.99332 * b[1] + w * 0.0750759;
            b[2] = 0.96900 * b[2] + w * 0.1538520;
            b[3] = 0.86650 * b[3] + w * 0.3104856;
            b[4] = 0.55000 * b[4] + w * 0.5329522;
            b[5] = -0.7616 * b[5] - w * 0.0168980;
            let y = b[..6].iter().sum::<f64>() + b[6] + w * 0.5362;
            b[6] = w * 0.115926;
            amplitude * (y * 0.11) as f32
        })
        .collect()
}

pub fn sine(n: usize, fs: f64, freq: f64, amplitude: f32, phase: f64) -> Vec<f32> {
    (0..n)
        .map(|i| amplitude * (2.0 * std::f64::consts::PI * freq * i as f64 / fs + phase).sin() as f32)
        .collect()
}
