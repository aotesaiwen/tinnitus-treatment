//! Times each verification stage on 10 min of stereo pink noise.

use std::time::Instant;

use tmdsp::analysis::{self, Psd};
use tmdsp::{loudness, signals};

fn main() {
    let fs = 48_000.0;
    let n = 10 * 60 * 48_000;
    let ch = vec![signals::pink_noise(n, 0.3, 1), signals::pink_noise(n, 0.3, 2)];
    let t = Instant::now();
    let l = loudness::integrated_loudness(&ch, fs);
    println!("loudness       {:.3} s ({l:.2} LUFS)", t.elapsed().as_secs_f64());
    let t = Instant::now();
    let tp = loudness::true_peak_db(&ch, fs);
    println!("true peak      {:.3} s ({tp:.2} dBTP)", t.elapsed().as_secs_f64());
    for tr in [5.0, 30.0, 120.0, 250.0] {
        let nfft = analysis::analysis_nfft(fs, tr);
        let t = Instant::now();
        let p = Psd::welch(&ch, fs, nfft);
        println!("welch nfft {nfft:>6} {:.3} s ({} bins)", t.elapsed().as_secs_f64(), p.power.len());
    }
}
