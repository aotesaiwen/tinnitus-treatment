//! Notch → loudness → encode → re-measure, with a pass/fail report.
//!
//! Nothing nonlinear runs after the notch: a limiter or clipper would create
//! intermodulation products that refill the notch. Peak safety comes from
//! lowering the gain instead, and the report says when that happened.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::analysis::{self, NotchMeasurement, Psd};
use crate::conv;
use crate::loudness;
use crate::notch::{self, NotchDesign, NotchSpec};
use crate::wav::{self, Audio, SampleFormat, WriteStats};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Remove the band around the tinnitus frequency (treatment arm).
    Notch,
    /// Remove a band of the same shape somewhere else (sham control).
    Sham,
    /// Same loudness and peak processing, no notch (active control).
    Passthrough,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub mode: Mode,
    /// `centers_hz` holds the tinnitus frequency (or frequencies).
    pub spec: NotchSpec,
    pub sham_centers_hz: Vec<f64>,
    /// `None` keeps the input level (still subject to the true-peak ceiling).
    pub target_lufs: Option<f64>,
    pub true_peak_ceiling_db: f64,
    pub format: SampleFormat,
    pub dither_seed: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Notch,
            spec: NotchSpec::default(),
            sham_centers_hz: Vec::new(),
            target_lufs: Some(-23.0),
            true_peak_ceiling_db: -1.0,
            format: SampleFormat::F32,
            dither_seed: 0x5EED,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub id: String,
    pub description: String,
    pub value: f64,
    pub limit: f64,
    pub pass: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Levels {
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub tool: &'static str,
    pub version: &'static str,
    pub mode: Mode,
    pub fs: u32,
    pub channels: usize,
    pub duration_s: f64,
    pub input_path: Option<String>,
    pub input_sha256: Option<String>,
    pub input: Levels,
    /// Upper edge of the highest third-octave band within 60 dB of the loudest.
    pub input_bandwidth_hz: f64,
    pub design: Option<NotchDesign>,
    pub measurement: Option<NotchMeasurement>,
    pub target_lufs: Option<f64>,
    pub true_peak_ceiling_db: f64,
    pub gain_db: f64,
    pub gain_limited_by_true_peak: bool,
    pub output_format: SampleFormat,
    pub output_path: Option<String>,
    pub output_sha256: Option<String>,
    /// Measured on the encoded output, after quantisation.
    pub output: Levels,
    pub write: WriteStats,
    pub checks: Vec<Check>,
    pub warnings: Vec<String>,
    pub all_pass: bool,
    pub processing_s: f64,
    pub realtime_factor: f64,
}

fn levels(channels: &[Vec<f32>], fs: f64) -> Levels {
    Levels {
        integrated_lufs: loudness::integrated_loudness(channels, fs),
        true_peak_dbtp: loudness::true_peak_db(channels, fs),
    }
}

fn check(id: impl Into<String>, description: impl Into<String>, value: f64, limit: f64, pass: bool) -> Check {
    Check { id: id.into(), description: description.into(), value, limit, pass }
}

/// Runs the whole chain in memory. Returns the encoded WAV and its report.
pub fn run(input: &Audio, opts: &Options) -> Result<(Vec<u8>, Report)> {
    let t0 = Instant::now();
    let fs = input.fs as f64;
    if input.frames() == 0 {
        bail!("input has no samples");
    }
    let mut warnings = Vec::new();

    let design = match opts.mode {
        Mode::Passthrough => None,
        Mode::Notch => Some(notch::design(&opts.spec, fs).map_err(anyhow::Error::msg).context("designing notch")?),
        Mode::Sham => {
            if opts.sham_centers_hz.is_empty() {
                bail!("sham mode needs at least one --sham-center");
            }
            let sham = NotchSpec { centers_hz: opts.sham_centers_hz.clone(), ..opts.spec.clone() };
            let d = notch::design(&sham, fs).map_err(anyhow::Error::msg).context("designing sham notch")?;
            if let Ok(real) = notch::design(&NotchSpec { centers_hz: opts.spec.centers_hz.clone(), ..opts.spec.clone() }, fs) {
                let overlaps = d.stop_bands.iter().any(|s| {
                    real.stop_bands.iter().any(|r| s.lo_hz - d.transition_hz < r.hi_hz && r.lo_hz < s.hi_hz + d.transition_hz)
                });
                if overlaps {
                    warnings.push("sham notch overlaps the tinnitus band; the control is not a sham".into());
                }
            }
            Some(d)
        }
    };

    let filtered: Vec<Vec<f32>> = match &design {
        None => input.channels.clone(),
        Some(d) => {
            let taps = d.taps_f32();
            std::thread::scope(|s| {
                let handles: Vec<_> =
                    input.channels.iter().map(|ch| s.spawn(|| conv::filter_zero_phase(&taps, ch))).collect();
                handles.into_iter().map(|h| h.join().expect("filter thread")).collect()
            })
        }
    };

    let nfft = design.as_ref().map_or(16_384, |d| analysis::analysis_nfft(fs, d.transition_hz));
    let psd_in = Psd::welch(&input.channels, fs, nfft);
    let measurement = design.as_ref().map(|d| analysis::measure_notch(&psd_in, &Psd::welch(&filtered, fs, nfft), d));
    let input_levels = levels(&input.channels, fs);
    let pre = levels(&filtered, fs);

    let mut gain_db = match opts.target_lufs {
        Some(t) if pre.integrated_lufs.is_finite() => t - pre.integrated_lufs,
        _ => 0.0,
    };
    let limited = pre.true_peak_dbtp + gain_db > opts.true_peak_ceiling_db;
    if limited {
        gain_db = opts.true_peak_ceiling_db - pre.true_peak_dbtp;
    }
    let g = 10f32.powf(gain_db as f32 / 20.0);
    let out = Audio { fs: input.fs, channels: filtered.iter().map(|ch| ch.iter().map(|x| x * g).collect()).collect() };

    let (bytes, write) = wav::encode_wav(&out, opts.format, opts.dither_seed);
    let written = wav::parse_wav(&bytes).context("re-reading encoded output")?;
    let output = levels(&written.channels, fs);

    // --- checks ---
    let mut checks = Vec::new();
    if let (Some(d), Some(m)) = (&design, &measurement) {
        let spec = &d.spec;
        checks.push(check(
            "D1",
            "designed filter: minimum attenuation inside the notch band (dB)",
            d.achieved_stop_atten_db,
            spec.stop_atten_db,
            d.achieved_stop_atten_db >= spec.stop_atten_db,
        ));
        checks.push(check(
            "D2",
            "designed filter: maximum passband deviation (dB)",
            d.achieved_pass_ripple_db,
            spec.pass_ripple_db,
            d.achieved_pass_ripple_db <= spec.pass_ripple_db,
        ));
        for (i, b) in m.bands.iter().enumerate() {
            // Either the attenuation is measured at spec (3 dB allowance for
            // the window's own leakage), or what is left in the band is below
            // the measurement floor.
            let floor = b.output_level_db <= -100.0;
            checks.push(check(
                format!("M1.{i}"),
                format!("measured attenuation, {:.0}-{:.0} Hz (dB)", b.band.lo_hz, b.band.hi_hz),
                b.attenuation_db,
                spec.stop_atten_db - 3.0,
                b.attenuation_db >= spec.stop_atten_db - 3.0 || floor,
            ));
            let flank = b.lower_flank_change_db.abs().max(b.upper_flank_change_db.abs());
            checks.push(check(
                format!("M3.{i}"),
                "measured change in the half-octave flanks next to the notch (|dB|)",
                flank,
                0.2,
                flank <= 0.2,
            ));
            if b.input_level_db < -60.0 {
                warnings.push(format!(
                    "input has little energy in {:.0}-{:.0} Hz ({:.1} dB of total): the notch changes almost nothing",
                    b.band.lo_hz, b.band.hi_hz, b.input_level_db
                ));
            }
            if b.lower_flank_level_db.max(b.upper_flank_level_db) < -50.0 {
                warnings.push(format!(
                    "little energy beside the {:.0}-{:.0} Hz notch: the lateral-inhibition rationale needs sound in the flanks",
                    b.band.lo_hz, b.band.hi_hz
                ));
            }
            let upper_flank_top = ((b.band.hi_hz + d.transition_hz) * 2f64.sqrt()).min(fs / 2.0);
            let bw = psd_in.bandwidth_hz();
            if bw < upper_flank_top && b.band.hi_hz < fs / 2.0 {
                warnings.push(format!(
                    "input is band-limited to about {bw:.0} Hz, below the notch's upper flank ({upper_flank_top:.0} Hz); \
                     check the generator or codec"
                ));
            }
        }
        checks.push(check(
            "M2",
            "measured passband level change, 20 Hz-20 kHz outside the notch (|dB|)",
            m.passband_change_db.abs(),
            spec.pass_ripple_db,
            m.passband_change_db.abs() <= spec.pass_ripple_db,
        ));
    }
    if let Some(t) = opts.target_lufs {
        let err = output.integrated_lufs - t;
        if limited {
            warnings.push(format!(
                "loudness target {t} LUFS not reached: gain capped by the {} dBTP ceiling (output {:.2} LUFS)",
                opts.true_peak_ceiling_db, output.integrated_lufs
            ));
            checks.push(check("L1", "output loudness at or below target (LUFS)", output.integrated_lufs, t, err <= 0.1));
        } else {
            checks.push(check("L1", "output loudness error vs target (|LU|)", err.abs(), 0.1, err.abs() <= 0.1));
        }
    }
    checks.push(check(
        "L2",
        "output true peak (dBTP)",
        output.true_peak_dbtp,
        opts.true_peak_ceiling_db,
        output.true_peak_dbtp <= opts.true_peak_ceiling_db + 0.05,
    ));
    let bad = (write.clipped_samples + write.non_finite_samples) as f64;
    checks.push(check("Q1", "clipped or non-finite output samples", bad, 0.0, bad == 0.0));

    if input_levels.true_peak_dbtp > -0.1 {
        warnings.push(format!(
            "input true peak is {:.2} dBTP: the source may already be clipped",
            input_levels.true_peak_dbtp
        ));
    }
    if input.duration_s() < 3.0 {
        warnings.push("input is shorter than 3 s; loudness and spectra are less reliable".into());
    }

    let processing_s = t0.elapsed().as_secs_f64();
    let report = Report {
        tool: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        mode: opts.mode,
        fs: input.fs,
        channels: input.channels.len(),
        duration_s: input.duration_s(),
        input_path: None,
        input_sha256: None,
        input: input_levels,
        input_bandwidth_hz: psd_in.bandwidth_hz(),
        design,
        measurement,
        target_lufs: opts.target_lufs,
        true_peak_ceiling_db: opts.true_peak_ceiling_db,
        gain_db,
        gain_limited_by_true_peak: limited,
        output_format: opts.format,
        output_path: None,
        output_sha256: None,
        output,
        write,
        all_pass: checks.iter().all(|c| c.pass),
        checks,
        warnings,
        processing_s,
        realtime_factor: input.duration_s() / processing_s,
    };
    Ok((bytes, report))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads, processes and writes a file; hashes both ends for audit trails.
pub fn process_file(input_path: &Path, output_path: &Path, opts: &Options) -> Result<Report> {
    let in_bytes = std::fs::read(input_path).with_context(|| format!("reading {}", input_path.display()))?;
    let audio = wav::parse_wav(&in_bytes).with_context(|| format!("parsing {}", input_path.display()))?;
    let (out_bytes, mut report) = run(&audio, opts)?;
    std::fs::write(output_path, &out_bytes).with_context(|| format!("writing {}", output_path.display()))?;
    report.input_path = Some(input_path.display().to_string());
    report.input_sha256 = Some(sha256_hex(&in_bytes));
    report.output_path = Some(output_path.display().to_string());
    report.output_sha256 = Some(sha256_hex(&out_bytes));
    Ok(report)
}
