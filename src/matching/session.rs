//! A full matching session following the standard audiologic sequence:
//! thresholds → loudness match at 1 kHz → pitch match (repeated) →
//! octave-confusion check → threshold and loudness at the pitch →
//! broadband-noise threshold → minimum masking level.
//!
//! The session only decides what to present and records answers. Any UI
//! (CLI, app, web) renders the prompt and feeds the answer back.

use serde::{Deserialize, Serialize};

use super::procedures::{AscendingThreshold, Level, LoudnessJudgement, LoudnessMatch, MaskingLevel, OctaveCheck, PitchStaircase};
use super::stimulus::{Calibration, Sound, Stimulus};
use crate::signals::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Ear {
    Both,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum TinnitusSound {
    /// Ringing or whistling: match with pure tones.
    Tonal,
    /// Hissing or rushing: match with narrowband noise.
    NoiseLike,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionConfig {
    pub fs: f64,
    pub calibration: Calibration,
    pub start_level_db: f64,
    pub min_level_db: f64,
    /// Hard ceiling. No prompt ever exceeds it.
    pub max_level_db: f64,
    pub threshold_freqs_hz: Vec<f64>,
    /// Use these (same transducer, same units) instead of measuring.
    pub known_thresholds: Vec<(f64, f64)>,
    pub pitch_range_hz: (f64, f64),
    pub pitch_repeats: usize,
    pub tinnitus_sound: TinnitusSound,
    pub nbn_width_oct: f64,
    pub ear: Ear,
    pub seed: u64,
}

impl SessionConfig {
    pub fn new(fs: f64, calibration: Calibration) -> Self {
        let cal = calibration.is_calibrated();
        Self {
            fs,
            start_level_db: if cal { 40.0 } else { -50.0 },
            min_level_db: if cal { -10.0 } else { -110.0 },
            max_level_db: if cal { 85.0 } else { -15.0 },
            calibration,
            threshold_freqs_hz: vec![500.0, 1000.0, 2000.0, 3000.0, 4000.0, 6000.0, 8000.0, 12_000.0],
            known_thresholds: Vec::new(),
            pitch_range_hz: (250.0, 16_000.0),
            pitch_repeats: 3,
            tinnitus_sound: TinnitusSound::Tonal,
            nbn_width_oct: 1.0 / 3.0,
            ear: Ear::Both,
            seed: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    Threshold { hz: f64 },
    LoudnessReference,
    Pitch { run: usize },
    OctaveCheck,
    ThresholdAtPitch,
    LoudnessAtPitch,
    NoiseThreshold,
    MaskingLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Question {
    HeardSound,
    CompareLoudness,
    ClosestPitch,
    TinnitusStillAudible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Yes,
    No,
    Louder,
    Softer,
    Equal,
    First,
    Second,
    Same,
}

impl Question {
    pub fn text(&self) -> &'static str {
        match self {
            Question::HeardSound => "Did you hear the sound?",
            Question::CompareLoudness => "Was the sound louder than your tinnitus, softer, or equally loud?",
            Question::ClosestPitch => "Which of the two sounds was closer in pitch to your tinnitus?",
            Question::TinnitusStillAudible => "While the noise played, could you still hear your tinnitus?",
        }
    }

    pub fn options(&self) -> &'static [Answer] {
        match self {
            Question::HeardSound | Question::TinnitusStillAudible => &[Answer::Yes, Answer::No],
            Question::CompareLoudness => &[Answer::Louder, Answer::Softer, Answer::Equal],
            Question::ClosestPitch => &[Answer::First, Answer::Second, Answer::Same],
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Prompt {
    pub stage: Stage,
    pub question: Question,
    pub stimuli: Vec<Stimulus>,
    pub ear: Ear,
}

#[derive(Clone, Debug, Serialize)]
pub struct Trial {
    pub stage: Stage,
    pub stimuli: Vec<Stimulus>,
    pub answer: Answer,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ThresholdPoint {
    pub hz: f64,
    pub level: Level,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct LoudnessResult {
    pub hz: f64,
    pub level: Level,
    /// Level above the threshold at the same frequency, dB.
    pub sensation_level_db: Option<f64>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct RecommendedNotch {
    pub center_hz: f64,
    pub width_oct: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MatchReport {
    pub units: &'static str,
    pub calibrated: bool,
    pub ear: Ear,
    pub tinnitus_sound: TinnitusSound,
    pub thresholds: Vec<ThresholdPoint>,
    pub loudness_reference: LoudnessResult,
    pub pitch_matches_hz: Vec<f64>,
    /// Standard deviation of the matches, octaves.
    pub pitch_spread_oct: f64,
    pub pitch_before_octave_check_hz: f64,
    pub octave_shift: i32,
    /// The octave check was not unanimous: the tinnitus may be an octave off.
    pub octave_uncertain: bool,
    pub pitch_hz: f64,
    pub threshold_at_pitch: Level,
    pub loudness_at_pitch: LoudnessResult,
    pub noise_threshold: Option<Level>,
    pub minimum_masking_level: Option<Level>,
    pub mml_sensation_level_db: Option<f64>,
    pub recommended_notch: RecommendedNotch,
    pub trials: usize,
    /// Pitch is unreliable (spread, split octave vote, or range edge): retest.
    pub retest_recommended: bool,
    pub warnings: Vec<String>,
    pub log: Vec<Trial>,
}

enum Current {
    Threshold { idx: usize, p: AscendingThreshold },
    LoudnessRef(LoudnessMatch),
    Pitch { run: usize, p: PitchStaircase },
    Octave(OctaveCheck),
    ThresholdAtPitch(AscendingThreshold),
    LoudnessAtPitch(LoudnessMatch),
    NoiseThreshold(AscendingThreshold),
    Mml(MaskingLevel),
    Done,
}

pub struct Session {
    cfg: SessionConfig,
    pitch_lo: f64,
    pitch_hi: f64,
    threshold_freqs: Vec<f64>,
    rng: Rng,
    /// Present the higher tone first in the current pair.
    swap: bool,
    current: Current,
    thresholds: Vec<ThresholdPoint>,
    loudness_ref: Option<Level>,
    pitch_matches: Vec<f64>,
    pitch_limit_hits: usize,
    octave: Option<(f64, i32)>,
    octave_split_decisions: usize,
    threshold_at_pitch: Option<Level>,
    loudness_at_pitch: Option<Level>,
    noise_threshold: Option<Level>,
    mml: Option<Level>,
    capped_presentations: usize,
    log: Vec<Trial>,
}

const LOUDNESS_REF_HZ: f64 = 1000.0;

fn log2_mean(v: &[f64]) -> f64 {
    v.iter().map(|f| f.log2()).sum::<f64>() / v.len() as f64
}

fn log2_median(v: &[f64]) -> f64 {
    let mut l: Vec<f64> = v.iter().map(|f| f.log2()).collect();
    l.sort_by(f64::total_cmp);
    let n = l.len();
    if n % 2 == 1 { l[n / 2] } else { 0.5 * (l[n / 2 - 1] + l[n / 2]) }
}

impl Session {
    pub fn new(cfg: SessionConfig) -> Result<Self, String> {
        if cfg.fs < 8000.0 {
            return Err("sample rate must be at least 8 kHz".into());
        }
        if cfg.max_level_db.partial_cmp(&cfg.min_level_db) != Some(std::cmp::Ordering::Greater) {
            return Err("max_level_db must exceed min_level_db".into());
        }
        if cfg.pitch_repeats == 0 {
            return Err("pitch_repeats must be at least 1".into());
        }
        let top = 0.45 * cfg.fs;
        let (pitch_lo, pitch_hi) = (cfg.pitch_range_hz.0.max(20.0), cfg.pitch_range_hz.1.min(top));
        if pitch_hi / pitch_lo < 2.0 {
            return Err(format!("pitch range {pitch_lo:.0}-{pitch_hi:.0} Hz must span at least an octave"));
        }
        let threshold_freqs: Vec<f64> = cfg.threshold_freqs_hz.iter().copied().filter(|&f| f > 0.0 && f <= top).collect();
        let mut s = Self {
            pitch_lo,
            pitch_hi,
            threshold_freqs,
            rng: Rng::new(cfg.seed),
            swap: false,
            current: Current::Done,
            thresholds: cfg.known_thresholds.iter().map(|&(hz, l)| ThresholdPoint { hz, level: Level::Found(l) }).collect(),
            loudness_ref: None,
            pitch_matches: Vec::new(),
            pitch_limit_hits: 0,
            octave: None,
            octave_split_decisions: 0,
            threshold_at_pitch: None,
            loudness_at_pitch: None,
            noise_threshold: None,
            mml: None,
            capped_presentations: 0,
            log: Vec::new(),
            cfg,
        };
        s.swap = s.rng.uniform() < 0.5;
        s.current = if s.thresholds.is_empty() && !s.threshold_freqs.is_empty() {
            Current::Threshold { idx: 0, p: s.new_threshold(s.cfg.start_level_db) }
        } else {
            s.start_loudness_ref()
        };
        Ok(s)
    }

    pub fn config(&self) -> &SessionConfig {
        &self.cfg
    }

    pub fn log(&self) -> &[Trial] {
        &self.log
    }

    pub fn is_done(&self) -> bool {
        matches!(self.current, Current::Done)
    }

    fn new_threshold(&self, start: f64) -> AscendingThreshold {
        AscendingThreshold::new(start, self.cfg.min_level_db, self.cfg.max_level_db)
    }

    /// Threshold at `hz`, interpolated in log frequency between measured
    /// points (a threshold above the ceiling counts as the ceiling).
    pub fn threshold_at(&self, hz: f64) -> f64 {
        let mut p: Vec<(f64, f64)> = self.thresholds.iter().map(|t| (t.hz, t.level.db())).collect();
        p.sort_by(|a, b| a.0.total_cmp(&b.0));
        match p.as_slice() {
            [] => self.cfg.start_level_db,
            [first, ..] if hz <= first.0 => first.1,
            [.., last] if hz >= last.0 => last.1,
            _ => {
                let i = p.windows(2).position(|w| hz >= w[0].0 && hz < w[1].0).expect("inside range");
                let (a, b) = (p[i], p[i + 1]);
                a.1 + (hz / a.0).log2() / (b.0 / a.0).log2() * (b.1 - a.1)
            }
        }
    }

    /// Sensation level of the tinnitus loudness match at 1 kHz.
    fn reference_sl(&self) -> f64 {
        self.loudness_ref.map_or(10.0, |l| l.db() - self.threshold_at(LOUDNESS_REF_HZ))
    }

    /// Comparison tones are presented at equal sensation level, so the
    /// listener judges pitch rather than loudness.
    fn balanced_level(&self, hz: f64) -> f64 {
        (self.threshold_at(hz) + self.reference_sl()).clamp(self.cfg.min_level_db, self.cfg.max_level_db)
    }

    fn pitch_sound(&self, hz: f64) -> Sound {
        match self.cfg.tinnitus_sound {
            TinnitusSound::Tonal => Sound::Tone { hz },
            TinnitusSound::NoiseLike => Sound::NarrowNoise { hz, width_oct: self.cfg.nbn_width_oct },
        }
    }

    fn start_loudness_ref(&self) -> Current {
        let start = self.threshold_at(LOUDNESS_REF_HZ) + 10.0;
        Current::LoudnessRef(LoudnessMatch::new(start, self.cfg.min_level_db, self.cfg.max_level_db))
    }

    fn start_pitch(&mut self, run: usize) -> Current {
        // Random start, at least an octave in from either end, so the start
        // point does not bias the matches.
        let (lo, hi) = (self.pitch_lo.log2() + 1.0, self.pitch_hi.log2() - 1.0);
        let x = if hi > lo { lo + self.rng.uniform() * (hi - lo) } else { 0.5 * (lo + hi) };
        Current::Pitch { run, p: PitchStaircase::new(self.pitch_lo, self.pitch_hi, 2f64.powf(x)) }
    }

    fn after_pitch_runs(&mut self) -> Current {
        let center = 2f64.powf(log2_median(&self.pitch_matches));
        let oc = OctaveCheck::new(center, self.pitch_lo, self.pitch_hi);
        match oc.result {
            Some(f) => {
                self.octave = Some((f, 0));
                self.start_threshold_at_pitch()
            }
            None => Current::Octave(oc),
        }
    }

    pub fn pitch_hz(&self) -> Option<f64> {
        self.octave.map(|o| o.0)
    }

    fn start_threshold_at_pitch(&self) -> Current {
        let pitch = self.octave.expect("octave check done").0;
        Current::ThresholdAtPitch(self.new_threshold(self.threshold_at(pitch) + 15.0))
    }

    fn single(&self, stage: Stage, question: Question, sound: Sound, level_db: f64) -> Prompt {
        Prompt { stage, question, stimuli: vec![Stimulus { sound, level_db }], ear: self.cfg.ear }
    }

    fn pair(&self, stage: Stage, (low, high): (f64, f64)) -> Prompt {
        let mut stimuli = vec![
            Stimulus { sound: self.pitch_sound(low), level_db: self.balanced_level(low) },
            Stimulus { sound: self.pitch_sound(high), level_db: self.balanced_level(high) },
        ];
        if self.swap {
            stimuli.reverse();
        }
        Prompt { stage, question: Question::ClosestPitch, stimuli, ear: self.cfg.ear }
    }

    pub fn next_prompt(&self) -> Option<Prompt> {
        Some(match &self.current {
            Current::Threshold { idx, p } => {
                let hz = self.threshold_freqs[*idx];
                self.single(Stage::Threshold { hz }, Question::HeardSound, Sound::PulsedTone { hz }, p.level())
            }
            Current::LoudnessRef(p) => self.single(
                Stage::LoudnessReference,
                Question::CompareLoudness,
                Sound::Tone { hz: LOUDNESS_REF_HZ },
                p.level(),
            ),
            Current::Pitch { run, p } => self.pair(Stage::Pitch { run: *run }, p.pair()),
            Current::Octave(p) => self.pair(Stage::OctaveCheck, p.pair()),
            Current::ThresholdAtPitch(p) => {
                let hz = self.octave?.0;
                self.single(Stage::ThresholdAtPitch, Question::HeardSound, Sound::PulsedTone { hz }, p.level())
            }
            Current::LoudnessAtPitch(p) => {
                let hz = self.octave?.0;
                self.single(Stage::LoudnessAtPitch, Question::CompareLoudness, self.pitch_sound(hz), p.level())
            }
            Current::NoiseThreshold(p) => self.single(Stage::NoiseThreshold, Question::HeardSound, Sound::BroadNoise, p.level()),
            Current::Mml(p) => self.single(Stage::MaskingLevel, Question::TinnitusStillAudible, Sound::BroadNoise, p.level()),
            Current::Done => return None,
        })
    }

    pub fn answer(&mut self, a: Answer) -> Result<(), String> {
        let prompt = self.next_prompt().ok_or("the session is finished")?;
        if !prompt.question.options().contains(&a) {
            return Err(format!("{a:?} is not an answer to {:?}", prompt.question));
        }
        if prompt.question == Question::ClosestPitch
            && prompt.stimuli.iter().any(|s| s.level_db >= self.cfg.max_level_db - 1e-9)
        {
            self.capped_presentations += 1;
        }
        self.log.push(Trial { stage: prompt.stage, stimuli: prompt.stimuli, answer: a });

        let yes = a == Answer::Yes;
        let higher_closer = match a {
            Answer::First => Some(self.swap),
            Answer::Second => Some(!self.swap),
            _ => None,
        };
        let judgement = match a {
            Answer::Louder => LoudnessJudgement::Louder,
            Answer::Softer => LoudnessJudgement::Softer,
            _ => LoudnessJudgement::Equal,
        };

        let cur = std::mem::replace(&mut self.current, Current::Done);
        self.current = match cur {
            Current::Threshold { idx, mut p } => {
                p.respond(yes);
                match p.result {
                    None => Current::Threshold { idx, p },
                    Some(level) => {
                        self.thresholds.push(ThresholdPoint { hz: self.threshold_freqs[idx], level });
                        if idx + 1 < self.threshold_freqs.len() {
                            Current::Threshold { idx: idx + 1, p: self.new_threshold(self.cfg.start_level_db) }
                        } else {
                            self.start_loudness_ref()
                        }
                    }
                }
            }
            Current::LoudnessRef(mut p) => {
                p.respond(judgement);
                match p.result {
                    None => Current::LoudnessRef(p),
                    Some(level) => {
                        self.loudness_ref = Some(level);
                        self.start_pitch(0)
                    }
                }
            }
            Current::Pitch { run, mut p } => {
                p.respond(higher_closer);
                match p.result {
                    None => Current::Pitch { run, p },
                    Some(hz) => {
                        self.pitch_matches.push(hz);
                        self.pitch_limit_hits += p.limit_hits;
                        if run + 1 < self.cfg.pitch_repeats { self.start_pitch(run + 1) } else { self.after_pitch_runs() }
                    }
                }
            }
            Current::Octave(mut p) => {
                p.respond(higher_closer);
                match p.result {
                    None => Current::Octave(p),
                    Some(hz) => {
                        self.octave = Some((hz, p.shift));
                        self.octave_split_decisions = p.split_decisions;
                        self.start_threshold_at_pitch()
                    }
                }
            }
            Current::ThresholdAtPitch(mut p) => {
                p.respond(yes);
                match p.result {
                    None => Current::ThresholdAtPitch(p),
                    Some(level) => {
                        self.threshold_at_pitch = Some(level);
                        Current::LoudnessAtPitch(LoudnessMatch::new(level.db() + 10.0, self.cfg.min_level_db, self.cfg.max_level_db))
                    }
                }
            }
            Current::LoudnessAtPitch(mut p) => {
                p.respond(judgement);
                match p.result {
                    None => Current::LoudnessAtPitch(p),
                    Some(level) => {
                        self.loudness_at_pitch = Some(level);
                        Current::NoiseThreshold(self.new_threshold(self.cfg.start_level_db))
                    }
                }
            }
            Current::NoiseThreshold(mut p) => {
                p.respond(yes);
                match p.result {
                    None => Current::NoiseThreshold(p),
                    Some(level) => {
                        self.noise_threshold = Some(level);
                        match level {
                            Level::AboveMax(_) => Current::Done,
                            _ => Current::Mml(MaskingLevel::new(level.db(), self.cfg.max_level_db)),
                        }
                    }
                }
            }
            Current::Mml(mut p) => {
                p.respond(yes);
                match p.result {
                    None => Current::Mml(p),
                    Some(level) => {
                        self.mml = Some(level);
                        Current::Done
                    }
                }
            }
            Current::Done => Current::Done,
        };
        self.swap = self.rng.uniform() < 0.5;
        Ok(())
    }

    pub fn report(&self) -> Option<MatchReport> {
        if !self.is_done() {
            return None;
        }
        let (pitch_hz, octave_shift) = self.octave?;
        let mean = log2_mean(&self.pitch_matches);
        let spread = (self.pitch_matches.iter().map(|f| (f.log2() - mean).powi(2)).sum::<f64>()
            / (self.pitch_matches.len().max(2) - 1) as f64)
            .sqrt();
        let threshold_at_pitch = self.threshold_at_pitch?;
        let loudness_ref = self.loudness_ref?;
        let loudness_at_pitch = self.loudness_at_pitch?;
        let sl = |level: Level, thr: Level| match (level, thr) {
            (Level::Found(l), Level::Found(t)) => Some(l - t),
            _ => None,
        };
        let thr_ref = self
            .thresholds
            .iter()
            .find(|t| (t.hz - LOUDNESS_REF_HZ).abs() < 1.0)
            .map_or(Level::Found(self.threshold_at(LOUDNESS_REF_HZ)), |t| t.level);

        let mut warnings = Vec::new();
        if !self.cfg.calibration.is_calibrated() {
            warnings.push(
                "uncalibrated: levels are dB re digital full scale and depend on the device volume, so compare \
                 sessions only with the same headphones and volume setting"
                    .into(),
            );
        }
        if self.cfg.ear != Ear::Both {
            warnings.push(format!("stimuli were presented to the {:?} ear only", self.cfg.ear).to_lowercase());
        }
        if spread > 0.5 {
            warnings.push(format!(
                "pitch matches vary by {spread:.2} octaves (SD); repeat matching on another day before relying on the pitch"
            ));
        }
        if octave_shift != 0 {
            warnings.push(format!("octave check moved the match by {octave_shift:+} octave(s)"));
        }
        let octave_uncertain = self.octave_split_decisions > 0;
        if octave_uncertain {
            warnings.push(
                "octave check was not unanimous: the tinnitus may be an octave above or below the reported pitch".into(),
            );
        }
        let edge = PitchStaircase::MIN_STEP_OCT;
        let at_edge = (pitch_hz / self.pitch_lo).log2() < edge || (self.pitch_hi / pitch_hz).log2() < edge;
        if at_edge {
            warnings.push(format!(
                "pitch is at the edge of the tested range ({:.0}-{:.0} Hz); the tinnitus may lie outside it",
                self.pitch_lo, self.pitch_hi
            ));
        }
        if self.capped_presentations > 0 {
            warnings.push(format!(
                "{} pitch comparisons used tones capped at the ceiling (reduced hearing): loudness was not balanced",
                self.capped_presentations
            ));
        }
        if matches!(threshold_at_pitch, Level::AboveMax(_)) {
            warnings.push("no hearing measured at the pitch within the ceiling; sound there would be inaudible".into());
        }
        if matches!(loudness_ref, Level::AboveMax(_)) || matches!(loudness_at_pitch, Level::AboveMax(_)) {
            warnings.push("tinnitus is louder than the loudest permitted tone".into());
        }
        if matches!(self.mml, Some(Level::AboveMax(_))) {
            warnings.push("tinnitus was not masked at the ceiling".into());
        }

        Some(MatchReport {
            units: self.cfg.calibration.unit(),
            calibrated: self.cfg.calibration.is_calibrated(),
            ear: self.cfg.ear,
            tinnitus_sound: self.cfg.tinnitus_sound,
            thresholds: self.thresholds.clone(),
            loudness_reference: LoudnessResult {
                hz: LOUDNESS_REF_HZ,
                level: loudness_ref,
                sensation_level_db: sl(loudness_ref, thr_ref),
            },
            pitch_matches_hz: self.pitch_matches.clone(),
            pitch_spread_oct: spread,
            pitch_before_octave_check_hz: 2f64.powf(log2_median(&self.pitch_matches)),
            octave_shift,
            octave_uncertain,
            pitch_hz,
            threshold_at_pitch,
            loudness_at_pitch: LoudnessResult {
                hz: pitch_hz,
                level: loudness_at_pitch,
                sensation_level_db: sl(loudness_at_pitch, threshold_at_pitch),
            },
            noise_threshold: self.noise_threshold,
            minimum_masking_level: self.mml,
            mml_sensation_level_db: match (self.mml, self.noise_threshold) {
                (Some(m), Some(t)) => sl(m, t),
                _ => None,
            },
            recommended_notch: RecommendedNotch { center_hz: pitch_hz, width_oct: 1.0 },
            trials: self.log.len(),
            retest_recommended: spread > 0.5 || octave_uncertain || at_edge,
            warnings,
            log: self.log.clone(),
        })
    }
}
