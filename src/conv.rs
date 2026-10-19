//! FFT overlap-save convolution, offline (zero-phase) and streaming.

use std::collections::VecDeque;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Filters fixed-size blocks with a long FIR in O(log N) work per sample.
pub struct OverlapSave {
    n_taps: usize,
    block: usize,
    h_spec: Vec<Complex<f32>>,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    /// `[last n_taps-1 inputs | current block]`
    frame: Vec<f32>,
    time: Vec<f32>,
    spec: Vec<Complex<f32>>,
    scratch_fwd: Vec<Complex<f32>>,
    scratch_inv: Vec<Complex<f32>>,
}

impl OverlapSave {
    pub fn new(taps: &[f32]) -> Self {
        let n = taps.len();
        assert!(n > 0);
        let fft_len = (4 * n).next_power_of_two().max(1024);
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(fft_len);
        let inv = planner.plan_fft_inverse(fft_len);

        let mut padded = fwd.make_input_vec();
        padded[..n].copy_from_slice(taps);
        let mut h_spec = fwd.make_output_vec();
        fwd.process(&mut padded, &mut h_spec).expect("fft sizes match");
        // Fold the inverse FFT's 1/N scaling into the filter once.
        let scale = 1.0 / fft_len as f32;
        h_spec.iter_mut().for_each(|c| *c *= scale);

        Self {
            n_taps: n,
            block: fft_len - n + 1,
            h_spec,
            frame: vec![0.0; fft_len],
            time: fwd.make_input_vec(),
            spec: fwd.make_output_vec(),
            scratch_fwd: fwd.make_scratch_vec(),
            scratch_inv: inv.make_scratch_vec(),
            fwd,
            inv,
        }
    }

    pub fn block_len(&self) -> usize {
        self.block
    }

    /// Group delay of a linear-phase (symmetric) filter, samples.
    pub fn group_delay(&self) -> usize {
        (self.n_taps - 1) / 2
    }

    /// Consumes exactly `block_len()` samples and produces as many.
    pub fn process_block(&mut self, input: &[f32], output: &mut [f32]) {
        assert_eq!(input.len(), self.block);
        assert_eq!(output.len(), self.block);
        let hist = self.n_taps - 1;
        self.frame.copy_within(self.block.., 0);
        self.frame[hist..].copy_from_slice(input);

        self.time.copy_from_slice(&self.frame);
        self.fwd
            .process_with_scratch(&mut self.time, &mut self.spec, &mut self.scratch_fwd)
            .expect("fft sizes match");
        for (s, h) in self.spec.iter_mut().zip(&self.h_spec) {
            *s *= h;
        }
        // A real signal has real DC and Nyquist bins; clear rounding residue.
        let last = self.spec.len() - 1;
        self.spec[0].im = 0.0;
        self.spec[last].im = 0.0;
        self.inv
            .process_with_scratch(&mut self.spec, &mut self.time, &mut self.scratch_inv)
            .expect("fft sizes match");
        output.copy_from_slice(&self.time[hist..]);
    }
}

/// Filters a whole signal with the group delay removed: output sample `i`
/// lines up with input sample `i`, and the length is unchanged.
pub fn filter_zero_phase(taps: &[f32], x: &[f32]) -> Vec<f32> {
    let mut os = OverlapSave::new(taps);
    let delay = os.group_delay();
    let block = os.block_len();
    let total = x.len() + delay;
    let mut out = Vec::with_capacity(total + block);
    let mut inbuf = vec![0.0f32; block];
    let mut outbuf = vec![0.0f32; block];
    let mut pos = 0;
    while pos < total {
        let avail = x.len().saturating_sub(pos).min(block);
        if avail > 0 {
            inbuf[..avail].copy_from_slice(&x[pos..pos + avail]);
        }
        inbuf[avail..].fill(0.0);
        os.process_block(&inbuf, &mut outbuf);
        out.extend_from_slice(&outbuf);
        pos += block;
    }
    out.drain(..delay);
    out.truncate(x.len());
    out
}

/// Accepts any buffer size (for an audio callback) at a fixed latency of
/// `latency()` samples. Suited to music playback, where latency does not
/// matter but the notch has to be applied after decoding.
pub struct StreamingFilter {
    os: OverlapSave,
    inq: Vec<f32>,
    outq: VecDeque<f32>,
    tmp: Vec<f32>,
}

impl StreamingFilter {
    pub fn new(taps: &[f32]) -> Self {
        let os = OverlapSave::new(taps);
        let block = os.block_len();
        Self { os, inq: Vec::with_capacity(2 * block), outq: VecDeque::from(vec![0.0; block]), tmp: vec![0.0; block] }
    }

    /// Input-to-output delay in samples (buffering plus group delay).
    pub fn latency(&self) -> usize {
        self.os.block_len() + self.os.group_delay()
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        assert_eq!(input.len(), output.len());
        let block = self.os.block_len();
        self.inq.extend_from_slice(input);
        let mut consumed = 0;
        while self.inq.len() - consumed >= block {
            self.os.process_block(&self.inq[consumed..consumed + block], &mut self.tmp);
            self.outq.extend(self.tmp.iter().copied());
            consumed += block;
        }
        self.inq.drain(..consumed);
        for o in output.iter_mut() {
            *o = self.outq.pop_front().expect("queue primed with one block");
        }
    }
}
