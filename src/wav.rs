//! Minimal WAV reader/writer: PCM 8/16/24/32-bit and IEEE float 32/64.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::signals::Rng;

#[derive(Clone, Debug)]
pub struct Audio {
    pub fs: u32,
    /// One Vec per channel, samples in [-1, 1).
    pub channels: Vec<Vec<f32>>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, |c| c.len())
    }

    pub fn duration_s(&self) -> f64 {
        self.frames() as f64 / self.fs as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, clap::ValueEnum)]
pub enum SampleFormat {
    /// 32-bit float: lossless, no dither, no clipping.
    F32,
    /// 24-bit PCM with TPDF dither.
    Pcm24,
    /// 16-bit PCM with TPDF dither.
    Pcm16,
}

pub fn read_wav(path: &Path) -> Result<Audio> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse_wav(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn parse_wav(b: &[u8]) -> Result<Audio> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file");
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, usize, u32, usize)> = None;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32_at(b, pos + 4) as usize;
        let body = pos + 8;
        let end = body.saturating_add(size).min(b.len());
        match id {
            b"fmt " => {
                if end - body < 16 {
                    bail!("fmt chunk too short");
                }
                let mut tag = u16_at(b, body);
                let channels = u16_at(b, body + 2) as usize;
                let fs = u32_at(b, body + 4);
                let bits = u16_at(b, body + 14) as usize;
                if tag == 0xFFFE {
                    if end - body < 26 {
                        bail!("WAVE_FORMAT_EXTENSIBLE chunk too short");
                    }
                    tag = u16_at(b, body + 24);
                }
                fmt = Some((tag, channels, fs, bits));
            }
            b"data" => data = Some(&b[body..end]),
            _ => {}
        }
        pos = body.saturating_add(size).saturating_add(size & 1);
    }
    let (tag, n_ch, fs, bits) = fmt.context("missing fmt chunk")?;
    let data = data.context("missing data chunk")?;
    if n_ch == 0 || fs == 0 {
        bail!("invalid format: {n_ch} channels at {fs} Hz");
    }
    let decode: fn(&[u8]) -> f32 = match (tag, bits) {
        (1, 8) => |s| (s[0] as f32 - 128.0) / 128.0,
        (1, 16) => |s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
        (1, 24) => |s| (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
        (1, 32) => {
            |s| (i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64 / 2_147_483_648.0) as f32
        }
        (3, 32) => |s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        (3, 64) => |s| f64::from_le_bytes(s[..8].try_into().unwrap()) as f32,
        _ => bail!("unsupported WAV encoding: format tag {tag}, {bits} bits"),
    };
    let width = bits / 8;
    let frames = data.len() / (width * n_ch);
    let mut channels = vec![Vec::with_capacity(frames); n_ch];
    for frame in data.chunks_exact(width * n_ch).take(frames) {
        for (c, s) in frame.chunks_exact(width).enumerate() {
            channels[c].push(decode(s));
        }
    }
    Ok(Audio { fs, channels })
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct WriteStats {
    /// Samples that had to be clamped to the PCM range.
    pub clipped_samples: usize,
    pub non_finite_samples: usize,
}

pub fn write_wav(
    path: &Path,
    audio: &Audio,
    format: SampleFormat,
    dither_seed: u64,
) -> Result<WriteStats> {
    let (bytes, stats) = encode_wav(audio, format, dither_seed);
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(stats)
}

pub fn encode_wav(audio: &Audio, format: SampleFormat, dither_seed: u64) -> (Vec<u8>, WriteStats) {
    let n_ch = audio.channels.len();
    let frames = audio.frames();
    let (tag, bits): (u16, u16) = match format {
        SampleFormat::F32 => (3, 32),
        SampleFormat::Pcm24 => (1, 24),
        SampleFormat::Pcm16 => (1, 16),
    };
    let width = bits as usize / 8;
    let data_len = frames * n_ch * width;
    let fmt_len: u32 = if tag == 3 { 18 } else { 16 };
    let fact_len: usize = if tag == 3 { 12 } else { 0 };

    let mut out = Vec::with_capacity(44 + 2 + fact_len + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(
        &((4 + 8 + fmt_len as usize + fact_len + 8 + data_len) as u32).to_le_bytes(),
    );
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&fmt_len.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&(n_ch as u16).to_le_bytes());
    out.extend_from_slice(&audio.fs.to_le_bytes());
    out.extend_from_slice(&(audio.fs * (n_ch * width) as u32).to_le_bytes());
    out.extend_from_slice(&((n_ch * width) as u16).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    if tag == 3 {
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(b"fact");
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&(frames as u32).to_le_bytes());
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());

    let mut stats = WriteStats::default();
    let mut rng = Rng::new(dither_seed);
    let full_scale = match format {
        SampleFormat::Pcm24 => 8_388_608.0,
        SampleFormat::Pcm16 => 32_768.0,
        SampleFormat::F32 => 1.0,
    };
    for i in 0..frames {
        for ch in &audio.channels {
            let mut x = ch[i];
            if !x.is_finite() {
                stats.non_finite_samples += 1;
                x = 0.0;
            }
            if format == SampleFormat::F32 {
                out.extend_from_slice(&x.to_le_bytes());
                continue;
            }
            // TPDF dither, ±1 LSB peak.
            let d = rng.uniform() + rng.uniform() - 1.0;
            let q = (x as f64 * full_scale + d).round();
            let clamped = q.clamp(-full_scale, full_scale - 1.0);
            if clamped != q {
                stats.clipped_samples += 1;
            }
            let v = clamped as i32;
            out.extend_from_slice(&v.to_le_bytes()[..width]);
        }
    }
    (out, stats)
}
