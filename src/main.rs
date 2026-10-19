mod match_cli;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use tmdsp::analysis::Psd;
use tmdsp::notch::{self, NotchSpec};
use tmdsp::pipeline::{self, Mode, Options};
use tmdsp::wav::{self, Audio, SampleFormat};
use tmdsp::{conv, loudness, signals};

#[derive(Parser)]
#[command(name = "tmusic", version, about = "Verified notch processing for tinnitus sound-therapy research")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct NotchArgs {
    /// Tinnitus frequency in Hz (repeat for several).
    #[arg(long = "center", required = true)]
    centers: Vec<f64>,
    /// Width of the removed band, octaves.
    #[arg(long, default_value_t = 1.0)]
    width_oct: f64,
    /// Transition band width, octaves.
    #[arg(long, default_value_t = 1.0 / 24.0)]
    transition_oct: f64,
    /// Minimum attenuation inside the band, dB.
    #[arg(long, default_value_t = 60.0)]
    atten_db: f64,
    /// Maximum passband deviation, dB.
    #[arg(long, default_value_t = 0.1)]
    ripple_db: f64,
}

impl NotchArgs {
    fn spec(&self) -> NotchSpec {
        NotchSpec {
            centers_hz: self.centers.clone(),
            width_oct: self.width_oct,
            transition_oct: self.transition_oct,
            stop_atten_db: self.atten_db,
            pass_ripple_db: self.ripple_db,
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Notch a WAV file, normalise loudness, and write a verification report.
    Process {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[command(flatten)]
        notch: NotchArgs,
        #[arg(long, value_enum, default_value_t = Mode::Notch)]
        mode: Mode,
        /// Centre of the sham notch (sham mode).
        #[arg(long = "sham-center")]
        sham_centers: Vec<f64>,
        /// Integrated loudness target, LUFS.
        #[arg(long, default_value_t = -23.0, allow_negative_numbers = true)]
        target_lufs: f64,
        /// Keep the input loudness instead of normalising.
        #[arg(long)]
        no_normalize: bool,
        /// True-peak ceiling, dBTP.
        #[arg(long, default_value_t = -1.0, allow_negative_numbers = true)]
        tp_ceiling: f64,
        #[arg(long, value_enum, default_value_t = SampleFormat::F32)]
        format: SampleFormat,
        /// Report path (default: <output>.report.json).
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Design a notch and print its verified response.
    Design {
        #[arg(long, default_value_t = 48_000.0)]
        fs: f64,
        #[command(flatten)]
        notch: NotchArgs,
        /// Write the magnitude response (Hz, dB) as CSV.
        #[arg(long)]
        csv: Option<PathBuf>,
    },
    /// Loudness, true peak, bandwidth and band levels of a WAV file.
    Analyze {
        #[arg(short, long)]
        input: PathBuf,
        /// Report the level of a one-octave band around these frequencies.
        #[arg(long = "center")]
        centers: Vec<f64>,
    },
    /// Interactive tinnitus matching (pitch, octave check, loudness, masking level).
    Match(match_cli::MatchArgs),
    /// Validate the matching procedures against simulated listeners.
    MatchSim(match_cli::SimArgs),
    /// Write one calibrated matching stimulus to a WAV file.
    Stimulus(match_cli::StimulusArgs),
    /// Measure throughput on synthetic stereo pink noise.
    Bench {
        #[arg(long, default_value_t = 10.0)]
        minutes: f64,
        #[arg(long, default_value_t = 48_000)]
        fs: u32,
        #[arg(long = "center", default_values_t = [4000.0, 250.0])]
        centers: Vec<f64>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Returns whether every check passed.
fn run(cli: Cli) -> Result<bool> {
    match cli.cmd {
        Cmd::Match(a) => match_cli::interactive(a),
        Cmd::MatchSim(a) => match_cli::simulate(a),
        Cmd::Stimulus(a) => match_cli::stimulus(a),
        Cmd::Process { input, output, notch, mode, sham_centers, target_lufs, no_normalize, tp_ceiling, format, report } => {
            let opts = Options {
                mode,
                spec: notch.spec(),
                sham_centers_hz: sham_centers,
                target_lufs: (!no_normalize).then_some(target_lufs),
                true_peak_ceiling_db: tp_ceiling,
                format,
                ..Options::default()
            };
            let r = pipeline::process_file(&input, &output, &opts)?;
            let report_path = report.unwrap_or_else(|| {
                let mut p = output.clone().into_os_string();
                p.push(".report.json");
                p.into()
            });
            std::fs::write(&report_path, serde_json::to_string_pretty(&r)?)
                .with_context(|| format!("writing {}", report_path.display()))?;

            println!("{} -> {}  ({:?}, {:.1} s audio in {:.2} s, {:.0}x real time)",
                input.display(), output.display(), r.mode, r.duration_s, r.processing_s, r.realtime_factor);
            if let Some(d) = &r.design {
                println!("  filter: {} taps, delay compensated, transition {:.1} Hz", d.n_taps, d.transition_hz);
            }
            println!("  loudness {:.2} -> {:.2} LUFS, true peak {:.2} -> {:.2} dBTP, gain {:+.2} dB",
                r.input.integrated_lufs, r.output.integrated_lufs, r.input.true_peak_dbtp, r.output.true_peak_dbtp, r.gain_db);
            for c in &r.checks {
                println!("  [{}] {:5} {} = {:.3} (limit {})", if c.pass { "PASS" } else { "FAIL" }, c.id, c.description, c.value, c.limit);
            }
            for w in &r.warnings {
                println!("  warning: {w}");
            }
            println!("  report: {}", report_path.display());
            Ok(r.all_pass)
        }
        Cmd::Design { fs, notch, csv } => {
            let d = notch::design(&notch.spec(), fs).map_err(anyhow::Error::msg)?;
            println!("{}", serde_json::to_string_pretty(&d)?);
            if let Some(path) = csv {
                let mut s = String::from("hz,db\n");
                for (f, g) in d.response_db() {
                    s.push_str(&format!("{f:.3},{g:.4}\n"));
                }
                std::fs::write(&path, s).with_context(|| format!("writing {}", path.display()))?;
            }
            Ok(d.meets_spec())
        }
        Cmd::Analyze { input, centers } => {
            let a = wav::read_wav(&input)?;
            let fs = a.fs as f64;
            let psd = Psd::welch(&a.channels, fs, 16_384);
            println!("{}: {} Hz, {} ch, {:.2} s", input.display(), a.fs, a.channels.len(), a.duration_s());
            println!("  integrated loudness {:.2} LUFS", loudness::integrated_loudness(&a.channels, fs));
            println!("  true peak {:.2} dBTP", loudness::true_peak_db(&a.channels, fs));
            println!("  bandwidth (-60 dB, third-octave) {:.0} Hz", psd.bandwidth_hz());
            for c in centers {
                let lvl = psd.band_power(c / 2f64.sqrt(), c * 2f64.sqrt()) / psd.total();
                println!("  one-octave band at {c:.0} Hz: {:.1} dB of total", 10.0 * lvl.max(1e-300).log10());
            }
            Ok(true)
        }
        Cmd::Bench { minutes, fs, centers } => {
            let n = (minutes * 60.0 * fs as f64) as usize;
            let audio = Audio {
                fs,
                channels: vec![signals::pink_noise(n, 0.3, 1), signals::pink_noise(n, 0.3, 2)],
            };
            println!("{minutes} min stereo pink noise at {fs} Hz ({} frames)", n);
            let mut ok = true;
            for c in centers {
                let spec = NotchSpec { centers_hz: vec![c], ..NotchSpec::default() };
                let t = Instant::now();
                let d = notch::design(&spec, fs as f64).map_err(anyhow::Error::msg)?;
                let t_design = t.elapsed().as_secs_f64();
                let taps = d.taps_f32();
                let t = Instant::now();
                let y = conv::filter_zero_phase(&taps, &audio.channels[0]);
                let t_filter = t.elapsed().as_secs_f64();
                std::hint::black_box(&y);
                let opts = Options { spec, ..Options::default() };
                let t = Instant::now();
                let (_, r) = pipeline::run(&audio, &opts)?;
                let t_full = t.elapsed().as_secs_f64();
                let secs = n as f64 / fs as f64;
                println!(
                    "  {c:>6.0} Hz: {:>6} taps | design {:.3} s | filter 1 ch {:.3} s ({:.0}x RT) | full pipeline 2 ch {:.2} s ({:.0}x RT) | checks {}",
                    d.n_taps, t_design, t_filter, secs / t_filter, t_full, secs / t_full,
                    if r.all_pass { "pass" } else { "FAIL" }
                );
                ok &= r.all_pass;
            }
            Ok(ok)
        }
    }
}
