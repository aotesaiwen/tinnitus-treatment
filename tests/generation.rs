use tmdsp::generation::{self, Request, Style};
use tmdsp::notch::NotchSpec;
use tmdsp::pipeline::{self, Mode, Options};
use tmdsp::wav;

fn request(style: Style) -> Request {
    Request {
        style,
        duration_s: 30,
        tempo_bpm: 60,
        seed: 42,
        prompt: String::new(),
    }
}

#[test]
fn composer_is_reproducible_and_variations_are_distinct() {
    let req = request(Style::Focus);
    let a = generation::compose(&req, || false).unwrap();
    let b = generation::compose(&req, || false).unwrap();
    let c = generation::compose(&Request { seed: 43, ..req }, || false).unwrap();
    assert_eq!(a.channels, b.channels);
    assert_ne!(a.channels, c.channels);
    assert_ne!(
        a.channels[0], a.channels[1],
        "stereo should have actual spatial variation"
    );
}

#[test]
fn composer_outputs_finite_unclipped_audio_with_fades() {
    for style in [Style::Drift, Style::Focus, Style::Unwind] {
        let audio = generation::compose(&request(style), || false).unwrap();
        assert_eq!(audio.frames(), 30 * 48_000);
        for channel in &audio.channels {
            assert_eq!(channel[0], 0.0);
            assert_eq!(*channel.last().unwrap(), 0.0);
            assert!(channel.iter().all(|s| s.is_finite() && s.abs() < 1.0));
            assert!(channel.iter().any(|s| s.abs() > 0.01));
            let max_jump = channel
                .windows(2)
                .map(|w| (w[0] - w[1]).abs())
                .fold(0.0f32, f32::max);
            assert!(max_jump < 0.05, "abrupt sample discontinuity: {max_jump}");
        }
    }
}

#[test]
fn composer_rejects_unbounded_requests_and_supports_cancellation() {
    for duration in [0, 29, 301, u32::MAX] {
        assert!(
            generation::compose(
                &Request {
                    duration_s: duration,
                    ..request(Style::Drift)
                },
                || false
            )
            .is_err()
        );
    }
    for tempo in [0, 44, 101, u32::MAX] {
        assert!(
            generation::compose(
                &Request {
                    tempo_bpm: tempo,
                    ..request(Style::Drift)
                },
                || false
            )
            .is_err()
        );
    }
    assert!(
        Request {
            prompt: "x".repeat(501),
            ..request(Style::Drift)
        }
        .validate()
        .is_err()
    );
    assert!(
        generation::compose(&request(Style::Drift), || true)
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
fn generated_styles_pass_existing_audio_pipeline() {
    for style in [Style::Drift, Style::Focus, Style::Unwind] {
        let audio = generation::compose(&request(style), || false).unwrap();
        for mode in [Mode::Notch, Mode::Passthrough, Mode::Sham] {
            let options = Options {
                mode,
                spec: NotchSpec {
                    centers_hz: vec![6000.0],
                    ..NotchSpec::default()
                },
                sham_centers_hz: vec![1500.0],
                ..Options::default()
            };
            let (_, report) = pipeline::run(&audio, &options).unwrap();
            assert!(
                report.all_pass,
                "{style:?} {mode:?}: {:?}",
                report.checks.iter().filter(|c| !c.pass).collect::<Vec<_>>()
            );
            assert!(
                report.warnings.is_empty(),
                "{style:?} {mode:?}: {:?}",
                report.warnings
            );
        }
    }
}

#[test]
fn malformed_generated_wav_is_rejected_without_panicking() {
    let audio = generation::compose(&request(Style::Drift), || false).unwrap();
    let (mut bytes, _) = wav::encode_wav(&audio, wav::SampleFormat::F32, 1);
    bytes[34] = 0; // zero bits per sample must not cause division by zero
    bytes[35] = 0;
    assert!(wav::parse_wav(&bytes).is_err());
}
