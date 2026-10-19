//! Simulated listeners, for checking that the procedures recover a known
//! tinnitus pitch, loudness and masking level, and how many trials it takes.
//!
//! The listener model is deliberately simple: noisy decisions on log
//! frequency and on sensation level. Loudness uses an equal-sensation-level
//! model, the same assumption the session uses to balance comparison tones,
//! so these results are an upper bound on real-world loudness accuracy.

use serde::Serialize;

use super::session::{Answer, MatchReport, Prompt, Question, Session, SessionConfig};
use super::stimulus::Sound;
use crate::signals::Rng;

pub struct SimListener {
    pub pitch_hz: f64,
    /// Decision noise on pitch distance, octaves.
    pub pitch_sigma_oct: f64,
    /// Tones an octave below the tinnitus also sound close (by `OCTAVE_PENALTY`).
    pub octave_confusion: bool,
    /// Tinnitus loudness as a sensation level, dB.
    pub tinnitus_sl_db: f64,
    pub level_sigma_db: f64,
    /// Masking level of broadband noise above its threshold, dB.
    pub mml_sl_db: f64,
    /// Threshold up to 2 kHz, rising by `slope_db_per_oct` above.
    pub thr_flat_db: f64,
    pub slope_db_per_oct: f64,
    pub rng: Rng,
}

const OCTAVE_PENALTY: f64 = 0.15;

impl SimListener {
    pub fn threshold(&self, hz: f64) -> f64 {
        self.thr_flat_db + self.slope_db_per_oct * (hz / 2000.0).log2().max(0.0)
    }

    pub fn noise_threshold(&self) -> f64 {
        self.threshold(1000.0)
    }

    /// Level at `hz` that sounds as loud as the tinnitus.
    pub fn equal_loudness_level(&self, hz: f64) -> f64 {
        self.threshold(hz) + self.tinnitus_sl_db
    }

    pub fn true_mml(&self) -> f64 {
        self.noise_threshold() + self.mml_sl_db
    }

    fn gaussian(&mut self) -> f64 {
        let u1 = 1.0 - self.rng.uniform();
        let u2 = self.rng.uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    fn pitch_distance(&self, hz: f64) -> f64 {
        let (x, t) = (hz.log2(), self.pitch_hz.log2());
        let d = (x - t).abs();
        if self.octave_confusion { d.min((x - (t - 1.0)).abs() + OCTAVE_PENALTY) } else { d }
    }

    pub fn answer(&mut self, p: &Prompt) -> Answer {
        let s = p.stimuli[0];
        match p.question {
            Question::HeardSound => {
                let thr = match s.sound {
                    Sound::BroadNoise => self.noise_threshold(),
                    other => self.threshold(other.reference_hz()),
                };
                if s.level_db + self.level_sigma_db * self.gaussian() >= thr { Answer::Yes } else { Answer::No }
            }
            Question::CompareLoudness => {
                let diff = s.level_db - self.equal_loudness_level(s.sound.reference_hz())
                    + self.level_sigma_db * self.gaussian();
                if diff.abs() < 1.0 {
                    Answer::Equal
                } else if diff > 0.0 {
                    Answer::Louder
                } else {
                    Answer::Softer
                }
            }
            Question::ClosestPitch => {
                let d0 = self.pitch_distance(p.stimuli[0].sound.reference_hz()) + self.pitch_sigma_oct * self.gaussian();
                let d1 = self.pitch_distance(p.stimuli[1].sound.reference_hz()) + self.pitch_sigma_oct * self.gaussian();
                if d0 < d1 { Answer::First } else { Answer::Second }
            }
            Question::TinnitusStillAudible => {
                if s.level_db + self.level_sigma_db * self.gaussian() < self.true_mml() { Answer::Yes } else { Answer::No }
            }
        }
    }
}

/// Runs a whole session with a simulated listener.
pub fn run(session: &mut Session, listener: &mut SimListener) -> MatchReport {
    while let Some(p) = session.next_prompt() {
        let a = listener.answer(&p);
        session.answer(a).expect("simulated answers are always valid");
    }
    session.report().expect("session finished")
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct SimParams {
    pub n: usize,
    pub pitch_sigma_oct: f64,
    pub level_sigma_db: f64,
    pub octave_confusion: bool,
    /// Supply thresholds (rounded to 5 dB, like an audiogram) instead of measuring them.
    pub audiogram: bool,
    pub seed: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SimStats {
    pub params: SimParams,
    pub pitch_err_median_oct: f64,
    pub pitch_err_p90_oct: f64,
    /// Fraction of listeners whose true pitch lies inside a one-octave notch
    /// centred on the match (|error| ≤ 0.5 octave).
    pub within_half_oct: f64,
    pub within_sixth_oct: f64,
    /// Of the listeners whose pitch fell outside the notch, the fraction
    /// whose report recommended a retest.
    pub misses_flagged: f64,
    /// Fraction of all reports that recommended a retest.
    pub retest_rate: f64,
    pub loudness_err_median_db: f64,
    pub loudness_err_p90_db: f64,
    pub mml_err_median_db: f64,
    pub mml_err_p90_db: f64,
    pub trials_median: f64,
    pub trials_max: usize,
    /// Any stimulus above the configured ceiling (must be 0).
    pub ceiling_violations: usize,
}

fn percentile(v: &mut [f64], q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        return f64::NAN;
    }
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// Simulates `n` listeners with random tinnitus pitch (0.5-10 kHz, log-
/// uniform), hearing, tinnitus loudness and masking level.
pub fn monte_carlo(params: SimParams, template: &SessionConfig) -> SimStats {
    let mut rng = Rng::new(params.seed);
    let (mut pitch_err, mut loud_err, mut mml_err, mut trials) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut violations = 0;
    let (mut misses, mut misses_flagged, mut retests) = (0usize, 0usize, 0usize);
    for i in 0..params.n {
        let mut u = || rng.uniform();
        let base = template.start_level_db;
        let mut listener = SimListener {
            pitch_hz: 2f64.powf(500f64.log2() + u() * (10_000f64 / 500.0).log2()),
            pitch_sigma_oct: params.pitch_sigma_oct,
            octave_confusion: params.octave_confusion,
            tinnitus_sl_db: 3.0 + 17.0 * u(),
            level_sigma_db: params.level_sigma_db,
            mml_sl_db: 5.0 + 25.0 * u(),
            thr_flat_db: base - 35.0 - 10.0 * u(),
            slope_db_per_oct: 12.0 * u(),
            rng: Rng::new(params.seed ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        };
        let mut cfg = template.clone();
        cfg.seed = params.seed.wrapping_add(i as u64 * 7919);
        if params.audiogram {
            cfg.known_thresholds =
                cfg.threshold_freqs_hz.iter().map(|&hz| (hz, (listener.threshold(hz) / 5.0).round() * 5.0)).collect();
        }
        let mut session = Session::new(cfg).expect("valid template");
        let r = run(&mut session, &mut listener);

        violations += r.log.iter().flat_map(|t| &t.stimuli).filter(|s| s.level_db > template.max_level_db + 1e-9).count();
        let err = (r.pitch_hz / listener.pitch_hz).log2().abs();
        pitch_err.push(err);
        retests += r.retest_recommended as usize;
        if err > 0.5 {
            misses += 1;
            misses_flagged += r.retest_recommended as usize;
        }
        if let Some(l) = r.loudness_at_pitch.level.found() {
            loud_err.push((l - listener.equal_loudness_level(r.pitch_hz)).abs());
        }
        if let Some(m) = r.minimum_masking_level.and_then(|m| m.found()) {
            mml_err.push((m - listener.true_mml()).abs());
        }
        trials.push(r.trials as f64);
    }
    let n = params.n as f64;
    SimStats {
        params,
        within_half_oct: pitch_err.iter().filter(|&&e| e <= 0.5).count() as f64 / n,
        within_sixth_oct: pitch_err.iter().filter(|&&e| e <= 1.0 / 6.0).count() as f64 / n,
        misses_flagged: if misses == 0 { 1.0 } else { misses_flagged as f64 / misses as f64 },
        retest_rate: retests as f64 / n,
        pitch_err_median_oct: percentile(&mut pitch_err, 0.5),
        pitch_err_p90_oct: percentile(&mut pitch_err, 0.9),
        loudness_err_median_db: percentile(&mut loud_err, 0.5),
        loudness_err_p90_db: percentile(&mut loud_err, 0.9),
        mml_err_median_db: percentile(&mut mml_err, 0.5),
        mml_err_p90_db: percentile(&mut mml_err, 0.9),
        trials_max: trials.iter().fold(0.0f64, |m, &t| m.max(t)) as usize,
        trials_median: percentile(&mut trials, 0.5),
        ceiling_violations: violations,
    }
}
