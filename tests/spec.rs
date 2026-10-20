//! Specification tests. Each test states the requirement it verifies.

use tmdsp::analysis::Psd;
use tmdsp::conv::{self, StreamingFilter};
use tmdsp::fir;
use tmdsp::loudness::{self, k_weighting};
use tmdsp::notch::{self, NotchSpec};
use tmdsp::pipeline::{self, Mode, Options};
use tmdsp::signals::{self, Rng};
use tmdsp::wav::{self, Audio, SampleFormat};

fn spec(centers: &[f64], width_oct: f64) -> NotchSpec {
    NotchSpec { centers_hz: centers.to_vec(), width_oct, ..NotchSpec::default() }
}

fn db(x: f64) -> f64 {
    20.0 * x.abs().max(1e-300).log10()
}

/// REQ-F1/F2: across the clinically relevant range, every design meets the
/// stopband and passband spec, checked independently of the design loop by
/// exact DTFT at random frequencies.
#[test]
fn designs_meet_spec_across_tinnitus_range() {
    let mut rng = Rng::new(7);
    for fs in [44_100.0, 48_000.0, 96_000.0] {
        for &c in &[250.0, 500.0, 1000.0, 2000.0, 3000.0, 4000.0, 6000.0, 8000.0, 10_000.0, 12_000.0, 14_000.0] {
            for width in [0.5, 1.0] {
                let s = spec(&[c], width);
                let d = notch::design(&s, fs).unwrap_or_else(|e| panic!("{fs} Hz, {c} Hz, {width} oct: {e}"));
                assert!(d.meets_spec(), "{fs}/{c}/{width}: {:.2} dB stop, {:.4} dB ripple",
                    d.achieved_stop_atten_db, d.achieved_pass_ripple_db);
                for _ in 0..60 {
                    for b in &d.stop_bands {
                        let f = b.lo_hz + rng.uniform() * (b.hi_hz - b.lo_hz);
                        let g = db(fir::amplitude_at(&d.taps, f / fs));
                        assert!(g <= -s.stop_atten_db, "{fs}/{c}/{width}: {g:.2} dB at {f:.1} Hz in stopband");
                    }
                    for b in &d.pass_bands {
                        let f = b.lo_hz + rng.uniform() * (b.hi_hz - b.lo_hz);
                        let g = db(fir::amplitude_at(&d.taps, f / fs));
                        assert!(g.abs() <= s.pass_ripple_db, "{fs}/{c}/{width}: {g:.4} dB at {f:.1} Hz in passband");
                    }
                }
            }
        }
    }
}

/// REQ-F3: the nominal band is exactly one octave, geometrically centred.
#[test]
fn band_is_one_octave_centred_on_tinnitus() {
    let d = notch::design(&spec(&[6000.0], 1.0), 48_000.0).unwrap();
    let b = d.stop_bands[0];
    assert!((b.hi_hz / b.lo_hz - 2.0).abs() < 1e-12);
    assert!(((b.lo_hz * b.hi_hz).sqrt() - 6000.0).abs() < 1e-9);
}

/// REQ-F4: overlapping notches merge; separated ones keep a passband between.
#[test]
fn multiple_notches() {
    let fs = 48_000.0;
    let merged = notch::design(&spec(&[4000.0, 5000.0], 1.0), fs).unwrap();
    assert_eq!(merged.stop_bands.len(), 1);
    let apart = notch::design(&spec(&[1000.0, 8000.0], 0.5), fs).unwrap();
    assert_eq!(apart.stop_bands.len(), 2);
    assert!(apart.meets_spec());
    assert!(db(fir::amplitude_at(&apart.taps, 2800.0 / fs)).abs() < 0.1);
}

/// REQ-F5: impossible requests are rejected, not silently degraded.
#[test]
fn rejects_invalid_specs() {
    assert!(notch::design(&spec(&[], 1.0), 48_000.0).is_err());
    assert!(notch::design(&spec(&[30_000.0], 1.0), 48_000.0).is_err());
    assert!(notch::design(&spec(&[5.0], 1.0), 48_000.0).is_err());
    assert!(notch::design(&NotchSpec { width_oct: 0.0, ..spec(&[4000.0], 1.0) }, 48_000.0).is_err());
}

/// REQ-F6: a band reaching Nyquist becomes a low-pass and still meets spec.
#[test]
fn notch_reaching_nyquist() {
    let d = notch::design(&spec(&[16_000.0], 1.0), 44_100.0).unwrap();
    assert_eq!(d.pass_bands.len(), 1);
    assert!(d.meets_spec());
}

/// REQ-P1: zero-phase processing. A passband sine comes out sample-aligned
/// and unchanged; a sine at the tinnitus frequency is removed.
#[test]
fn sines_through_zero_phase_filter() {
    let fs = 48_000.0;
    let d = notch::design(&spec(&[4000.0], 1.0), fs).unwrap();
    let taps = d.taps_f32();
    let n = 96_000;
    let skip = d.n_taps; // ignore start/end transients
    for (f, expect_removed) in [(4000.0, true), (2900.0, true), (5600.0, true), (1000.0, false), (11_000.0, false)] {
        let x = signals::sine(n, fs, f, 0.5, 0.3);
        let y = conv::filter_zero_phase(&taps, &x);
        assert_eq!(y.len(), x.len());
        let mid = skip..n - skip;
        if expect_removed {
            let peak = y[mid].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(db(peak as f64 / 0.5) <= -60.0, "{f} Hz: residual {:.1} dB", db(peak as f64 / 0.5));
        } else {
            let err = x[mid.clone()].iter().zip(&y[mid]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
            assert!(err < 0.5 * 0.012, "{f} Hz: max sample error {err}");
        }
    }
}

/// REQ-P2: the streaming filter (any callback size) produces the same
/// samples as offline processing, delayed by its reported latency.
#[test]
fn streaming_matches_offline() {
    let fs = 48_000.0;
    let d = notch::design(&spec(&[3000.0], 1.0), fs).unwrap();
    let taps = d.taps_f32();
    let x = signals::white_noise(60_000, 0.5, 3);
    let offline = conv::filter_zero_phase(&taps, &x);

    let mut sf = StreamingFilter::new(&taps);
    let lat = sf.latency();
    let mut padded = x.clone();
    padded.extend(std::iter::repeat_n(0.0, lat));
    let mut out = vec![0.0f32; padded.len()];
    let mut rng = Rng::new(11);
    let mut pos = 0;
    while pos < padded.len() {
        let len = (1 + (rng.uniform() * 1500.0) as usize).min(padded.len() - pos);
        sf.process(&padded[pos..pos + len], &mut out[pos..pos + len]);
        pos += len;
    }
    let streamed = &out[lat..lat + x.len()];
    let err = streamed.iter().zip(&offline).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(err < 1e-5, "max difference {err}");
}

/// REQ-M1: on broadband noise the notch is measured at spec and nothing
/// outside it changes.
#[test]
fn measured_notch_on_noise() {
    for (fs, c) in [(48_000u32, 4000.0), (44_100, 8000.0), (48_000, 500.0)] {
        let n = 20 * fs as usize;
        let audio = Audio { fs, channels: vec![signals::white_noise(n, 0.3, 1), signals::pink_noise(n, 0.3, 2)] };
        let opts = Options { spec: spec(&[c], 1.0), ..Options::default() };
        let (_, r) = pipeline::run(&audio, &opts).unwrap();
        let m = r.measurement.as_ref().unwrap();
        let b = &m.bands[0];
        assert!(b.attenuation_db >= 60.0, "{c} Hz: measured {:.1} dB", b.attenuation_db);
        assert!(m.passband_change_db.abs() < 0.02, "{c} Hz: passband {:.4} dB", m.passband_change_db);
        assert!(b.lower_flank_change_db.abs() < 0.05 && b.upper_flank_change_db.abs() < 0.05);
        assert!(r.all_pass, "{:#?}", r.checks);
    }
}

/// REQ-L1: K-weighting matches the BS.1770-4 48 kHz coefficient table.
#[test]
fn k_weighting_matches_bs1770_table() {
    let [s1, s2] = k_weighting(48_000.0);
    let want_b1 = [1.53512485958697, -2.69169618940638, 1.19839281085285];
    let want_a1 = [1.0, -1.69065929318241, 0.73248077421585];
    let want_a2 = [1.0, -1.99004745483398, 0.99007225036621];
    for i in 0..3 {
        assert!((s1.b[i] - want_b1[i]).abs() < 1e-8, "shelf b{i}: {}", s1.b[i]);
        assert!((s1.a[i] - want_a1[i]).abs() < 1e-8, "shelf a{i}: {}", s1.a[i]);
        assert!((s2.a[i] - want_a2[i]).abs() < 1e-8, "hp a{i}: {}", s2.a[i]);
    }
    assert_eq!(s2.b, [1.0, -2.0, 1.0]);
}

fn ebu_sequence(fs: f64, parts: &[(f64, f64)]) -> Vec<Vec<f32>> {
    let mut ch = Vec::new();
    for &(dbfs, secs) in parts {
        let n = (secs * fs).round() as usize;
        let a = 10f32.powf(dbfs as f32 / 20.0);
        let start = ch.len();
        ch.extend((0..n).map(|i| a * (2.0 * std::f64::consts::PI * 1000.0 * (start + i) as f64 / fs).sin() as f32));
    }
    vec![ch.clone(), ch]
}

/// REQ-L2: integrated loudness passes EBU Tech 3341 cases 1-5 (±0.1 LU),
/// at 48 kHz and at 44.1 kHz.
#[test]
fn ebu_tech_3341_loudness_cases() {
    let cases: [(&[(f64, f64)], f64); 5] = [
        (&[(-23.0, 20.0)], -23.0),
        (&[(-33.0, 20.0)], -33.0),
        (&[(-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0)], -23.0),
        (&[(-72.0, 10.0), (-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0), (-72.0, 10.0)], -23.0),
        (&[(-26.0, 20.0), (-20.0, 20.1), (-26.0, 20.0)], -23.0),
    ];
    for fs in [48_000.0, 44_100.0] {
        for (i, (parts, want)) in cases.iter().enumerate() {
            let l = loudness::integrated_loudness(&ebu_sequence(fs, parts), fs);
            assert!((l - want).abs() <= 0.1, "case {} at {fs} Hz: {l:.3} LUFS, want {want}", i + 1);
        }
    }
}

/// REQ-L3: true peak finds inter-sample peaks the sample peak misses.
#[test]
fn true_peak_catches_intersample_peaks() {
    let fs = 48_000.0;
    // fs/4 sine at 45 degrees: every sample is at 0.707 of the true peak.
    let mut x = signals::sine(48_000, fs, fs / 4.0, 0.5, std::f64::consts::FRAC_PI_4);
    // 10 ms raised-cosine fades: an abrupt start would add Gibbs overshoot.
    let fade = 480;
    let n = x.len();
    for i in 0..fade {
        let g = 0.5 - 0.5 * (std::f64::consts::PI * i as f64 / fade as f64).cos();
        x[i] *= g as f32;
        x[n - 1 - i] *= g as f32;
    }
    let sample_peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!((db(sample_peak as f64 / 0.5) + 3.01).abs() < 0.01);
    let tp = loudness::true_peak_db(&[x], fs);
    assert!((tp - db(0.5)).abs() < 0.1, "true peak {tp:.3} dBTP, want {:.3}", db(0.5));
}

/// REQ-O1: output hits the loudness target and the true-peak ceiling; when
/// both cannot hold, the ceiling wins and the report says so.
#[test]
fn loudness_target_and_peak_ceiling() {
    let fs = 48_000u32;
    let n = 15 * fs as usize;
    let quiet = Audio { fs, channels: vec![signals::pink_noise(n, 0.05, 4), signals::pink_noise(n, 0.05, 5)] };
    let opts = Options { spec: spec(&[5000.0], 1.0), format: SampleFormat::Pcm24, ..Options::default() };
    let (_, r) = pipeline::run(&quiet, &opts).unwrap();
    assert!(!r.gain_limited_by_true_peak);
    assert!((r.output.integrated_lufs + 23.0).abs() <= 0.1, "{}", r.output.integrated_lufs);
    assert!(r.all_pass, "{:#?}", r.checks);

    // A very peaky signal cannot reach -10 LUFS under a -1 dBTP ceiling.
    let mut x = signals::white_noise(n, 0.02, 6);
    for i in (0..n).step_by(4800) {
        x[i] = 0.9;
    }
    let peaky = Audio { fs, channels: vec![x] };
    let opts = Options { target_lufs: Some(-10.0), ..opts };
    let (_, r) = pipeline::run(&peaky, &opts).unwrap();
    assert!(r.gain_limited_by_true_peak);
    assert!(r.output.true_peak_dbtp <= -1.0 + 0.05, "{}", r.output.true_peak_dbtp);
    assert!(r.warnings.iter().any(|w| w.contains("not reached")));
    assert!(r.all_pass, "{:#?}", r.checks);
}

/// REQ-C1: passthrough (active control) applies only a gain, so the arms
/// differ in the notch and nothing else.
#[test]
fn passthrough_is_pure_gain() {
    let fs = 48_000u32;
    let x = signals::pink_noise(10 * fs as usize, 0.1, 9);
    let audio = Audio { fs, channels: vec![x.clone()] };
    let opts = Options { mode: Mode::Passthrough, ..Options::default() };
    let (bytes, r) = pipeline::run(&audio, &opts).unwrap();
    let out = wav::parse_wav(&bytes).unwrap();
    let g = 10f32.powf(r.gain_db as f32 / 20.0);
    let err = x.iter().zip(&out.channels[0]).fold(0.0f32, |m, (a, b)| m.max((a * g - b).abs()));
    assert!(err < 1e-6, "{err}");
}

/// REQ-C2: treatment and control end at the same loudness, so a listener
/// cannot tell the arms apart by level.
#[test]
fn arms_are_loudness_matched() {
    let fs = 48_000u32;
    let n = 20 * fs as usize;
    let audio = Audio { fs, channels: vec![signals::pink_noise(n, 0.2, 21), signals::pink_noise(n, 0.2, 22)] };
    let base = Options { spec: spec(&[3000.0], 1.0), sham_centers_hz: vec![700.0], ..Options::default() };
    let lufs: Vec<f64> = [Mode::Notch, Mode::Sham, Mode::Passthrough]
        .into_iter()
        .map(|mode| pipeline::run(&audio, &Options { mode, ..base.clone() }).unwrap().1.output.integrated_lufs)
        .collect();
    for l in &lufs {
        assert!((l - lufs[0]).abs() < 0.05, "{lufs:?}");
    }
}

/// REQ-C3: a sham notch on top of the tinnitus band is flagged.
#[test]
fn sham_overlap_is_flagged() {
    let fs = 48_000u32;
    let audio = Audio { fs, channels: vec![signals::pink_noise(5 * fs as usize, 0.2, 1)] };
    let opts = Options { mode: Mode::Sham, spec: spec(&[4000.0], 1.0), sham_centers_hz: vec![5000.0], ..Options::default() };
    let (_, r) = pipeline::run(&audio, &opts).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("not a sham")));
}

/// REQ-W1: WAV round trips: float is bit-exact, 24-bit within dither.
#[test]
fn wav_round_trip() {
    let x = signals::white_noise(10_000, 0.9, 13);
    let a = Audio { fs: 44_100, channels: vec![x.clone(), x.iter().map(|v| -v).collect()] };
    let (bytes, stats) = wav::encode_wav(&a, SampleFormat::F32, 1);
    assert_eq!(stats.clipped_samples, 0);
    let b = wav::parse_wav(&bytes).unwrap();
    assert_eq!(b.fs, 44_100);
    assert_eq!(a.channels, b.channels);

    let (bytes, _) = wav::encode_wav(&a, SampleFormat::Pcm24, 1);
    let c = wav::parse_wav(&bytes).unwrap();
    let lsb = 1.0 / 8_388_608.0;
    let err = a.channels[1].iter().zip(&c.channels[1]).fold(0.0f32, |m, (p, q)| m.max((p - q).abs()));
    assert!(err <= 1.5 * lsb, "{err}");
}

/// REQ-A1: band-limited input (e.g. a codec cutting at 7 kHz) is flagged
/// when the notch sits above the content.
#[test]
fn band_limited_input_is_flagged() {
    let fs = 48_000u32;
    let lp = notch::design(&NotchSpec { centers_hz: vec![12_000.0], width_oct: 2.0, ..NotchSpec::default() }, fs as f64).unwrap();
    let x = conv::filter_zero_phase(&lp.taps_f32(), &signals::pink_noise(10 * fs as usize, 0.2, 3));
    let audio = Audio { fs, channels: vec![x] };
    let opts = Options { spec: spec(&[8000.0], 1.0), ..Options::default() };
    let (_, r) = pipeline::run(&audio, &opts).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("band-limited")), "{:?}", r.warnings);
}

/// Sanity check of the Psd helper used throughout.
#[test]
fn psd_band_power_of_sine() {
    let fs = 48_000.0;
    let psd = Psd::welch(&[signals::sine(48_000, fs, 1000.0, 1.0, 0.0)], fs, 8192);
    let in_band = psd.band_power(900.0, 1100.0) / psd.total();
    assert!(in_band > 0.999999, "{in_band}");
}

/// REQ-R1: resampling 48 -> 44.1 kHz keeps tone frequency and level, and
/// keeps the notch (the resampler is linear).
#[test]
fn resampling_preserves_tone_and_notch() {
    use tmdsp::resample::resample;
    let fs = 48_000u32;
    let n = 10 * fs as usize;
    let tone = Audio { fs, channels: vec![signals::sine(n, fs as f64, 1000.0, 0.5, 0.0)] };
    let r = resample(&tone, 44_100).unwrap();
    assert_eq!(r.fs, 44_100);
    assert!((r.frames() as f64 - n as f64 * 44_100.0 / 48_000.0).abs() <= 2.0, "{}", r.frames());
    let mid = &r.channels[0][4410..r.frames() - 4410];
    let peak = mid.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(db(peak as f64 / 0.5).abs() < 0.05, "{peak}");
    let psd = Psd::welch(&[mid.to_vec()], 44_100.0, 16_384);
    assert!(psd.band_power(990.0, 1010.0) / psd.total() > 0.9999);

    let d = notch::design(&spec(&[4000.0], 1.0), fs as f64).unwrap();
    let noise = Audio { fs, channels: vec![conv::filter_zero_phase(&d.taps_f32(), &signals::white_noise(n, 0.3, 8))] };
    let r = resample(&noise, 44_100).unwrap();
    let psd = Psd::welch(&r.channels, 44_100.0, 8192);
    let band = psd.band_power(3000.0, 5500.0) / psd.band_power(1000.0, 2500.0) * (1500.0 / 2500.0);
    assert!(10.0 * band.log10() < -55.0, "notch after resampling: {:.1} dB", 10.0 * band.log10());
}
