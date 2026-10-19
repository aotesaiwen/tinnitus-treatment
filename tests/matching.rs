//! Specification tests for tinnitus matching.

use tmdsp::analysis::Psd;
use tmdsp::matching::procedures::{AscendingThreshold, Level, LoudnessJudgement, LoudnessMatch, MaskingLevel, OctaveCheck, PitchStaircase};
use tmdsp::matching::sim::{self, SimParams};
use tmdsp::matching::{Answer, Calibration, Renderer, Session, SessionConfig, Sound, Stimulus};
use tmdsp::signals::Rng;

fn renderer(fs: f64) -> Renderer {
    Renderer::new(fs, Calibration::default(), -15.0, 3)
}

/// REQ-S1: tones have the requested frequency and level (dB re a full-scale sine).
#[test]
fn tone_frequency_and_level() {
    let fs = 48_000.0;
    let x = renderer(fs).render(&Stimulus { sound: Sound::Tone { hz: 4000.0 }, level_db: -30.0 }).unwrap();
    let steady = &x[2000..x.len() - 2000];
    let peak = steady.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let want = 10f32.powf(-30.0 / 20.0);
    assert!((peak / want - 1.0).abs() < 1e-3, "peak {peak} want {want}");
    let psd = Psd::welch(&[x], fs, 16_384);
    let k = psd.power.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
    assert!((k as f64 * psd.bin_hz() - 4000.0).abs() <= psd.bin_hz());
}

/// REQ-S2: onsets and offsets are ramped (no clicks).
#[test]
fn tones_are_ramped() {
    let fs = 48_000.0;
    let mut r = renderer(fs);
    for sound in [Sound::Tone { hz: 1000.0 }, Sound::PulsedTone { hz: 6000.0 }, Sound::BroadNoise] {
        let x = r.render(&Stimulus { sound, level_db: -20.0 }).unwrap();
        assert!(x[0].abs() < 1e-6 && x[x.len() - 1].abs() < 1e-3, "{sound:?}");
        let amp = 10f32.powf(-1.0);
        let head = x[..48].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(head < 0.05 * amp * 4.0, "{sound:?}: {head}");
    }
}

/// REQ-S3: narrowband and broadband noise keep their energy in band, at
/// the RMS of a sine of the same level.
#[test]
fn noise_band_and_level() {
    let fs = 48_000.0;
    let mut r = renderer(fs);
    for (sound, lo, hi) in [
        (Sound::NarrowNoise { hz: 4000.0, width_oct: 1.0 / 3.0 }, 4000.0 / 2f64.powf(1.0 / 6.0), 4000.0 * 2f64.powf(1.0 / 6.0)),
        (Sound::BroadNoise, 125.0, 16_000.0),
    ] {
        let x = r.render(&Stimulus { sound, level_db: -30.0 }).unwrap();
        let steady = &x[4000..x.len() - 4000];
        let rms = (steady.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / steady.len() as f64).sqrt();
        let want = 10f64.powf(-30.0 / 20.0) / 2f64.sqrt();
        assert!((20.0 * (rms / want).log10()).abs() < 0.2, "{sound:?}: rms {rms} want {want}");
        let psd = Psd::welch(&[x], fs, 8192);
        let frac = psd.band_power(lo * 0.97, hi * 1.03) / psd.total();
        assert!(frac > 0.99, "{sound:?}: {frac}");
    }
}

/// REQ-S4: nothing above the ceiling is ever rendered, nor anything that clips.
#[test]
fn renderer_refuses_unsafe_levels() {
    let mut r = renderer(48_000.0);
    assert!(r.render(&Stimulus { sound: Sound::Tone { hz: 1000.0 }, level_db: -14.0 }).is_err());
    let mut loud = Renderer::new(48_000.0, Calibration::default(), 10.0, 1);
    assert!(loud.render(&Stimulus { sound: Sound::BroadNoise, level_db: 0.0 }).is_err());
}

/// REQ-S5: calibration interpolates in log frequency.
#[test]
fn calibration_interpolation() {
    let cal = Calibration { points: vec![(2000.0, 90.0), (1000.0, 100.0)] };
    assert!((cal.offset_db(2f64.sqrt() * 1000.0) - 95.0).abs() < 1e-9);
    assert_eq!(cal.offset_db(100.0), 100.0);
    assert_eq!(cal.offset_db(8000.0), 90.0);
    assert!((cal.to_dbfs(70.0, 1000.0) + 30.0).abs() < 1e-9);
    assert_eq!(cal.unit(), "dB SPL");
}

/// REQ-P1: an ideal listener's threshold is found within one 5 dB step.
#[test]
fn threshold_procedure_ideal_listener() {
    for true_thr in [-87.3, -60.0, -41.9] {
        let mut p = AscendingThreshold::new(-50.0, -110.0, -15.0);
        while p.result.is_none() {
            p.respond(p.level() >= true_thr);
        }
        let Level::Found(t) = p.result.unwrap() else { panic!() };
        assert!(t >= true_thr && t < true_thr + 5.0, "{true_thr}: {t}");
    }
    let mut deaf = AscendingThreshold::new(-50.0, -110.0, -15.0);
    while deaf.result.is_none() {
        deaf.respond(false);
    }
    assert_eq!(deaf.result, Some(Level::AboveMax(-15.0)));
}

/// REQ-P2: an ideal listener's loudness match is within 1 dB.
#[test]
fn loudness_procedure_ideal_listener() {
    for target in [-72.4, -55.0, -31.6] {
        let mut p = LoudnessMatch::new(-60.0, -110.0, -15.0);
        while p.result.is_none() {
            let d = p.level() - target;
            p.respond(if d.abs() < 0.5 { LoudnessJudgement::Equal } else if d > 0.0 { LoudnessJudgement::Louder } else { LoudnessJudgement::Softer });
        }
        let t = p.result.unwrap().found().unwrap();
        assert!((t - target).abs() <= 1.0, "{target}: {t}");
    }
}

/// REQ-P3: an ideal listener's pitch is found within 1/12 octave anywhere
/// in the range, from any start.
#[test]
fn pitch_procedure_ideal_listener() {
    let mut rng = Rng::new(5);
    for _ in 0..200 {
        let target = 2f64.powf(8.0 + rng.uniform() * 5.9); // 256 Hz - 15 kHz
        let start = 2f64.powf(9.0 + rng.uniform() * 4.0);
        let mut p = PitchStaircase::new(250.0, 16_000.0, start);
        while p.result.is_none() {
            let (lo, hi) = p.pair();
            let closer_high = (hi.log2() - target.log2()).abs() < (lo.log2() - target.log2()).abs();
            p.respond(Some(closer_high));
        }
        let err = (p.result.unwrap() / target).log2().abs();
        assert!(err <= 1.0 / 12.0, "target {target:.0}, start {start:.0}: error {err:.3} oct");
        assert!(p.trials() <= 40);
    }
}

/// REQ-P4: the octave check corrects a match one octave low, and leaves a
/// correct match alone.
#[test]
fn octave_check_logic() {
    let decide = |true_hz: f64, start: f64| {
        let mut o = OctaveCheck::new(start, 250.0, 16_000.0);
        while o.result.is_none() {
            let (lo, hi) = o.pair();
            o.respond(Some((hi / true_hz).log2().abs() < (lo / true_hz).log2().abs()));
        }
        (o.result.unwrap(), o.split_decisions)
    };
    assert_eq!(decide(6000.0, 3000.0), (6000.0, 0));
    assert_eq!(decide(6000.0, 6000.0), (6000.0, 0));
    assert_eq!(decide(3000.0, 6000.0), (3000.0, 0));
}

/// REQ-P5: masking level is the mean of two ascending runs (2 dB steps).
#[test]
fn masking_level_procedure() {
    let mml = -43.0;
    let mut p = MaskingLevel::new(-80.0, -15.0);
    while p.result.is_none() {
        p.respond(p.level() < mml);
    }
    let m = p.result.unwrap().found().unwrap();
    assert!(m >= mml && m < mml + 2.0, "{m}");
}

/// REQ-W1: a session rejects answers that do not fit the question.
#[test]
fn session_validates_answers() {
    let mut s = Session::new(SessionConfig::new(48_000.0, Calibration::default())).unwrap();
    assert!(s.answer(Answer::First).is_err());
    assert!(s.answer(Answer::Yes).is_ok());
}

/// REQ-W2: with an audiogram the session skips threshold measurement.
#[test]
fn session_uses_known_thresholds() {
    let mut cfg = SessionConfig::new(48_000.0, Calibration::default());
    cfg.known_thresholds = vec![(1000.0, -80.0), (4000.0, -70.0)];
    let s = Session::new(cfg).unwrap();
    assert!(matches!(s.next_prompt().unwrap().stage, tmdsp::matching::Stage::LoudnessReference));
}

fn mc(pitch_sigma_oct: f64, level_sigma_db: f64, octave_confusion: bool) -> sim::SimStats {
    let params = SimParams { n: 1000, pitch_sigma_oct, level_sigma_db, octave_confusion, audiogram: false, seed: 42 };
    sim::monte_carlo(params, &SessionConfig::new(48_000.0, Calibration::default()))
}

/// REQ-V1: realistic listeners (0.2 octave pitch noise, 1.5 dB level noise):
/// the true pitch lies inside the 1-octave notch for ≥ 99 %; loudness and
/// masking level within 2.5 dB for 90 %; no stimulus above the ceiling.
#[test]
fn simulated_realistic_listeners() {
    let s = mc(0.2, 1.5, false);
    assert!(s.within_half_oct >= 0.99, "{s:#?}");
    assert!(s.loudness_err_p90_db <= 2.5, "{s:#?}");
    assert!(s.mml_err_p90_db <= 2.5, "{s:#?}");
    assert!(s.retest_rate <= 0.05, "{s:#?}");
    assert_eq!(s.ceiling_violations, 0);
}

/// REQ-V2: octave-confused listeners with 0.1 octave noise: ≥ 90 % end in
/// the right octave, and most misses are flagged for retest.
#[test]
fn simulated_octave_confusion() {
    let s = mc(0.1, 1.5, true);
    assert!(s.within_half_oct >= 0.90, "{s:#?}");
    assert!(s.misses_flagged >= 0.6, "{s:#?}");
    assert_eq!(s.ceiling_violations, 0);
}
