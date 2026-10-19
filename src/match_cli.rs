//! Terminal front end for tinnitus matching, its simulation, and stimulus export.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};

use tmdsp::matching::sim::{self, SimParams};
use tmdsp::matching::{Answer, Calibration, Ear, Question, Renderer, Session, SessionConfig, Sound, Stimulus, TinnitusSound};
use tmdsp::wav::{self, Audio, SampleFormat};

fn load_calibration(path: &Option<PathBuf>) -> Result<Calibration> {
    match path {
        None => Ok(Calibration::default()),
        Some(p) => {
            let text = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))
        }
    }
}

#[derive(Args)]
pub struct MatchArgs {
    #[arg(long, default_value_t = 48_000)]
    fs: u32,
    /// JSON calibration: {"points": [[hz, dB SPL of a full-scale sine], ...]}.
    #[arg(long)]
    calibration: Option<PathBuf>,
    /// Ear to test. For one-sided tinnitus, matching is usually done in the other ear.
    #[arg(long, value_enum, default_value_t = Ear::Both)]
    ear: Ear,
    #[arg(long, value_enum, default_value_t = TinnitusSound::Tonal)]
    sound: TinnitusSound,
    /// Level ceiling. Default: -15 dB FS uncalibrated, 85 dB SPL calibrated.
    #[arg(long, allow_negative_numbers = true)]
    max_level: Option<f64>,
    #[arg(long, default_value_t = 3)]
    repeats: usize,
    #[arg(long, default_value = "match_report.json")]
    report: PathBuf,
}

fn config(fs: u32, cal: Calibration, ear: Ear, sound: TinnitusSound, max_level: Option<f64>, repeats: usize) -> SessionConfig {
    let mut cfg = SessionConfig::new(fs as f64, cal);
    cfg.ear = ear;
    cfg.tinnitus_sound = sound;
    cfg.pitch_repeats = repeats;
    if let Some(m) = max_level {
        cfg.max_level_db = m;
    }
    cfg.seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    cfg
}

fn route(fs: u32, mono: Vec<f32>, ear: Ear) -> Audio {
    let silent = vec![0.0; mono.len()];
    let channels = match ear {
        Ear::Both => vec![mono.clone(), mono],
        Ear::Left => vec![mono, silent],
        Ear::Right => vec![silent, mono],
    };
    Audio { fs, channels }
}

/// Plays a WAV file and waits for it to finish, using the OS player.
fn play(path: &Path) -> Result<()> {
    let status = if cfg!(windows) {
        let p = path.display().to_string().replace('\'', "''");
        Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!("(New-Object Media.SoundPlayer '{p}').PlaySync()")])
            .status()
    } else if cfg!(target_os = "macos") {
        Command::new("afplay").arg(path).status()
    } else {
        Command::new("aplay").arg("-q").arg(path).status()
    }
    .context("starting the audio player")?;
    if !status.success() {
        bail!("audio player failed ({status})");
    }
    Ok(())
}

fn keys(q: Question) -> &'static str {
    match q {
        Question::HeardSound | Question::TinnitusStillAudible => "[y]es / [n]o",
        Question::CompareLoudness => "[l]ouder / [s]ofter / [e]qual",
        Question::ClosestPitch => "[1] first / [2] second / [0] same",
    }
}

fn parse_answer(q: Question, s: &str) -> Option<Answer> {
    let a = match (q, s) {
        (Question::HeardSound | Question::TinnitusStillAudible, "y") => Answer::Yes,
        (Question::HeardSound | Question::TinnitusStillAudible, "n") => Answer::No,
        (Question::CompareLoudness, "l") => Answer::Louder,
        (Question::CompareLoudness, "s") => Answer::Softer,
        (Question::CompareLoudness, "e") => Answer::Equal,
        (Question::ClosestPitch, "1") => Answer::First,
        (Question::ClosestPitch, "2") => Answer::Second,
        (Question::ClosestPitch, "0") => Answer::Same,
        _ => return None,
    };
    Some(a)
}

pub fn interactive(args: MatchArgs) -> Result<bool> {
    let cal = load_calibration(&args.calibration)?;
    let cfg = config(args.fs, cal.clone(), args.ear, args.sound, args.max_level, args.repeats);
    let mut renderer = Renderer::new(cfg.fs, cal, cfg.max_level_db, cfg.seed);
    let mut session = Session::new(cfg.clone()).map_err(anyhow::Error::msg)?;
    let wav_path = std::env::temp_dir().join("tmusic_stimulus.wav");

    println!("Tinnitus matching ({}; ceiling {:.0} {}).", cfg.tinnitus_sound_label(), cfg.max_level_db, cfg.calibration.unit());
    println!("This is a research tool, not a diagnostic test.");
    println!("Before starting: use headphones, turn the system volume LOW, and stop if any sound is uncomfortable.");
    println!("Keys: answer as shown, [r] replay, [q] quit. Press Enter to start.");
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    lines.next();

    let mut n = 0;
    while let Some(prompt) = session.next_prompt() {
        n += 1;
        let samples = renderer.render_all(&prompt.stimuli).map_err(anyhow::Error::msg)?;
        wav::write_wav(&wav_path, &route(args.fs, samples, prompt.ear), SampleFormat::Pcm24, n)?;
        println!("\n#{n} {:?}", prompt.stage);
        loop {
            play(&wav_path)?;
            print!("{} {} > ", prompt.question.text(), keys(prompt.question));
            std::io::stdout().flush()?;
            let line = lines.next().transpose()?.unwrap_or_else(|| "q".into());
            let s = line.trim().to_lowercase();
            if s == "q" {
                let partial = serde_json::to_string_pretty(session.log())?;
                let path = args.report.with_extension("partial.json");
                std::fs::write(&path, partial)?;
                println!("Stopped. Answers so far saved to {}.", path.display());
                return Ok(false);
            }
            if s == "r" {
                continue;
            }
            match parse_answer(prompt.question, &s) {
                Some(a) => {
                    session.answer(a).map_err(anyhow::Error::msg)?;
                    break;
                }
                None => println!("Please answer with {}.", keys(prompt.question)),
            }
        }
    }

    let r = session.report().context("session ended without a report")?;
    std::fs::write(&args.report, serde_json::to_string_pretty(&r)?)
        .with_context(|| format!("writing {}", args.report.display()))?;
    println!("\nPitch matches: {:?} Hz (spread {:.2} oct), octave shift {:+}",
        r.pitch_matches_hz.iter().map(|f| f.round()).collect::<Vec<_>>(), r.pitch_spread_oct, r.octave_shift);
    println!("Tinnitus pitch: {:.0} Hz", r.pitch_hz);
    println!("Loudness at pitch: {:?} ({})", r.loudness_at_pitch.level, r.units);
    println!("Minimum masking level: {:?}", r.minimum_masking_level);
    for w in &r.warnings {
        println!("warning: {w}");
    }
    println!("Report: {}", args.report.display());
    println!("Notch your music with:  tmusic process -i in.wav -o out.wav --center {:.0}", r.recommended_notch.center_hz);
    Ok(true)
}

trait Label {
    fn tinnitus_sound_label(&self) -> &'static str;
}

impl Label for SessionConfig {
    fn tinnitus_sound_label(&self) -> &'static str {
        match self.tinnitus_sound {
            TinnitusSound::Tonal => "tonal tinnitus, pure tones",
            TinnitusSound::NoiseLike => "noise-like tinnitus, narrowband noise",
        }
    }
}

#[derive(Args)]
pub struct SimArgs {
    #[arg(long, default_value_t = 1000)]
    n: usize,
    /// Pitch decision noise, octaves.
    #[arg(long, default_value_t = 0.2)]
    pitch_sigma: f64,
    /// Level decision noise, dB.
    #[arg(long, default_value_t = 1.5)]
    level_sigma: f64,
    #[arg(long)]
    octave_confusion: bool,
    /// Give the session an audiogram instead of measuring thresholds.
    #[arg(long)]
    audiogram: bool,
    #[arg(long, default_value_t = 3)]
    repeats: usize,
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Print JSON instead of a summary.
    #[arg(long)]
    json: bool,
}

pub fn simulate(args: SimArgs) -> Result<bool> {
    let mut cfg = SessionConfig::new(48_000.0, Calibration::default());
    cfg.pitch_repeats = args.repeats;
    let params = SimParams {
        n: args.n,
        pitch_sigma_oct: args.pitch_sigma,
        level_sigma_db: args.level_sigma,
        octave_confusion: args.octave_confusion,
        audiogram: args.audiogram,
        seed: args.seed,
    };
    let t = std::time::Instant::now();
    let s = sim::monte_carlo(params, &cfg);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&s)?);
    } else {
        println!(
            "{} simulated listeners (pitch sigma {} oct, level sigma {} dB, octave confusion {}) in {:.2} s",
            args.n, args.pitch_sigma, args.level_sigma, args.octave_confusion, t.elapsed().as_secs_f64()
        );
        println!("  pitch error: median {:.3} oct, p90 {:.3} oct", s.pitch_err_median_oct, s.pitch_err_p90_oct);
        println!("  inside a 1-octave notch: {:.1} %   within 1/6 octave: {:.1} %", 100.0 * s.within_half_oct, 100.0 * s.within_sixth_oct);
        println!("  pitch misses flagged for retest: {:.1} %   retest rate overall: {:.1} %", 100.0 * s.misses_flagged, 100.0 * s.retest_rate);
        println!("  loudness error: median {:.2} dB, p90 {:.2} dB", s.loudness_err_median_db, s.loudness_err_p90_db);
        println!("  MML error: median {:.2} dB, p90 {:.2} dB", s.mml_err_median_db, s.mml_err_p90_db);
        println!("  trials per session: median {:.0}, max {}", s.trials_median, s.trials_max);
        println!("  stimuli above ceiling: {}", s.ceiling_violations);
    }
    Ok(s.ceiling_violations == 0)
}

#[derive(Clone, Copy, ValueEnum)]
pub enum StimulusKind {
    Tone,
    Pulsed,
    Nbn,
    Bbn,
}

#[derive(Args)]
pub struct StimulusArgs {
    #[arg(long, value_enum)]
    kind: StimulusKind,
    #[arg(long, default_value_t = 1000.0)]
    hz: f64,
    #[arg(long, default_value_t = 1.0 / 3.0)]
    width_oct: f64,
    /// Level in calibration units (dB FS when uncalibrated).
    #[arg(long, allow_negative_numbers = true)]
    level: f64,
    #[arg(long, default_value_t = 48_000)]
    fs: u32,
    #[arg(long)]
    calibration: Option<PathBuf>,
    #[arg(long, default_value_t = -15.0, allow_negative_numbers = true)]
    max_level: f64,
    #[arg(short, long)]
    output: PathBuf,
}

pub fn stimulus(args: StimulusArgs) -> Result<bool> {
    let sound = match args.kind {
        StimulusKind::Tone => Sound::Tone { hz: args.hz },
        StimulusKind::Pulsed => Sound::PulsedTone { hz: args.hz },
        StimulusKind::Nbn => Sound::NarrowNoise { hz: args.hz, width_oct: args.width_oct },
        StimulusKind::Bbn => Sound::BroadNoise,
    };
    let mut r = Renderer::new(args.fs as f64, load_calibration(&args.calibration)?, args.max_level, 1);
    let x = r.render(&Stimulus { sound, level_db: args.level }).map_err(anyhow::Error::msg)?;
    wav::write_wav(&args.output, &Audio { fs: args.fs, channels: vec![x] }, SampleFormat::Pcm24, 1)?;
    println!("wrote {}", args.output.display());
    Ok(true)
}
