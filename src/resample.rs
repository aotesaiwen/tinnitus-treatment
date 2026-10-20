//! Sample-rate conversion (FFT-based, via rubato), for playing tracks on a
//! device whose rate differs from the file's. The resampler is linear, so it
//! cannot refill a notch.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::wav::Audio;

pub fn resample(audio: &Audio, fs_out: u32) -> Result<Audio, String> {
    if audio.fs == fs_out || audio.frames() == 0 {
        return Ok(audio.clone());
    }
    let n_ch = audio.channels.len();
    let frames = audio.frames();
    let mut interleaved = Vec::with_capacity(frames * n_ch);
    for i in 0..frames {
        for ch in &audio.channels {
            interleaved.push(ch[i]);
        }
    }

    let mut r = Fft::<f32>::new(audio.fs as usize, fs_out as usize, 1024, n_ch, FixedSync::Input)
        .map_err(|e| format!("creating resampler: {e}"))?;
    let capacity = r.process_all_needed_output_len(frames);
    let mut out = vec![0.0f32; capacity * n_ch];
    let input = InterleavedSlice::new(&interleaved, n_ch, frames).map_err(|e| e.to_string())?;
    let mut output = InterleavedSlice::new_mut(&mut out, n_ch, capacity).map_err(|e| e.to_string())?;
    let (_, written) = r
        .process_all_into_buffer(&input, &mut output, frames, None)
        .map_err(|e| format!("resampling: {e}"))?;

    let channels = (0..n_ch).map(|c| (0..written).map(|i| out[i * n_ch + c]).collect()).collect();
    Ok(Audio { fs: fs_out, channels })
}
