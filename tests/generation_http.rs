#![cfg(feature = "generation-http")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use serde_json::{Value, json};
use tmdsp::generation::{Request, Style, acestep::AceStep};
use tmdsp::wav::{self, Audio, SampleFormat};

// A real HTTP fixture exercises serialization, framing, auth and WAV download.
// No model/GPU or remote service is used by these contract tests.
fn server(responses: Vec<(&'static str, Vec<u8>)>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0; 8192];
                let n = stream.read(&mut buf).unwrap();
                assert_ne!(n, 0, "client closed the request early");
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let len = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .map_or(0, |len| len.parse::<usize>().unwrap());
                    if request.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(request).unwrap());
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    (url, handle)
}

fn wrapped(data: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"code":200, "error":null, "data":data})).unwrap()
}

fn request() -> Request {
    Request {
        style: Style::Focus,
        duration_s: 30,
        tempo_bpm: 65,
        seed: 42,
        prompt: "soft piano".into(),
    }
}

fn task(file: &str) -> Vec<u8> {
    wrapped(json!([{"task_id":"job-1", "status":1,
        "result":serde_json::to_string(&json!([{"file":file, "dit_model":"fixture-model", "seed_value":"42"}])).unwrap()}]))
}

#[test]
fn submits_polls_and_downloads_wav_with_provenance() {
    let audio = Audio {
        fs: 16_000,
        channels: vec![vec![0.01; 30 * 16_000]],
    };
    let (bytes, _) = wav::encode_wav(&audio, SampleFormat::F32, 1);
    let (url, server) = server(vec![
        ("200 OK", wrapped(json!({"task_id":"job-1"}))),
        ("200 OK", task("/v1/audio?path=track.wav")),
        ("200 OK", bytes),
    ]);
    let client = AceStep::new(&url, Some("fixture-token".into())).unwrap();
    let (audio, model) = client.generate(&request(), || false, |_| {}).unwrap();
    assert_eq!(audio.duration_s(), 30.0);
    assert_eq!(model["dit_model"], "fixture-model");
    let calls = server.join().unwrap();
    assert!(calls[0].starts_with("POST /release_task "));
    let body: Value = serde_json::from_str(calls[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["batch_size"], 1);
    assert_eq!(body["audio_format"], "wav32");
    assert_eq!(body["use_cot_metas"], false);
    assert_eq!(body["seed"], 42);
    assert_eq!(body["lyrics"], "[Instrumental]");
    assert_eq!(model["submitted_parameters"], body);
    assert!(!serde_json::to_string(&model).unwrap().contains("fixture-token"));
    assert!(!calls[0].contains("pitch_hz"));
    assert!(calls[1].starts_with("POST /query_result "));
    assert!(calls[2].starts_with("GET /v1/audio?path=track.wav "));
    assert!(calls.iter().all(|call| {
        call.to_lowercase()
            .contains("authorization: bearer fixture-token")
    }));
}

#[test]
fn rejects_remote_downloads_and_unexpected_audio_paths() {
    for file in [
        "https://example.com/v1/audio?path=a",
        "//example.com/v1/audio",
        "/private-file",
    ] {
        let (url, server) = server(vec![
            ("200 OK", wrapped(json!({"task_id":"job-1"}))),
            ("200 OK", task(file)),
        ]);
        let error = AceStep::new(&url, None)
            .unwrap()
            .generate(&request(), || false, |_| {})
            .unwrap_err();
        assert!(error.contains("outside"), "{error}");
        assert_eq!(
            server.join().unwrap().len(),
            2,
            "must not follow an unsafe URL"
        );
    }
}

#[test]
fn handles_provider_failure_and_malformed_results() {
    for response in [
        wrapped(json!([{"task_id":"job-1", "status":2}])),
        wrapped(json!([{"task_id":"wrong-job", "status":1}])),
        wrapped(json!([{"task_id":"job-1", "status":1, "result":"not-json"}])),
        wrapped(json!([{"task_id":"job-1", "status":99}])),
        b"not-json".to_vec(),
    ] {
        let (url, server) = server(vec![
            ("200 OK", wrapped(json!({"task_id":"job-1"}))),
            ("200 OK", response),
        ]);
        assert!(
            AceStep::new(&url, None)
                .unwrap()
                .generate(&request(), || false, |_| {})
                .is_err()
        );
        server.join().unwrap();
    }
}

#[test]
fn handles_http_authentication_error_without_exposing_token() {
    let (url, server) = server(vec![("401 Unauthorized", b"fixture-secret".to_vec())]);
    let error = AceStep::new(&url, Some("fixture-secret".into()))
        .unwrap()
        .generate(&request(), || false, |_| {})
        .unwrap_err();
    assert!(error.contains("401"));
    assert!(!error.contains("fixture-secret"));
    server.join().unwrap();
}

#[test]
fn cancellation_prevents_submission_and_invalid_origins_are_rejected() {
    let client = AceStep::new("http://127.0.0.1:1", None).unwrap();
    assert!(
        client
            .generate(&request(), || true, |_| {})
            .unwrap_err()
            .contains("cancelled")
    );
    for endpoint in [
        "http://example.com",
        "file:///tmp/audio",
        "https://user:secret@example.com",
        "http://127.0.0.1:8001/api",
        "https://example.com?key=secret",
    ] {
        assert!(AceStep::new(endpoint, None).is_err(), "{endpoint}");
    }
}

#[test]
fn rejects_audio_with_wrong_duration_and_malformed_wav() {
    let (short, _) = wav::encode_wav(
        &Audio {
            fs: 16_000,
            channels: vec![vec![0.01; 16_000]],
        },
        SampleFormat::F32,
        1,
    );
    for bytes in [short, b"not a wav".to_vec()] {
        let (url, server) = server(vec![
            ("200 OK", wrapped(json!({"task_id":"job-1"}))),
            ("200 OK", task("/v1/audio?path=bad.wav")),
            ("200 OK", bytes),
        ]);
        assert!(
            AceStep::new(&url, None)
                .unwrap()
                .generate(&request(), || false, |_| {})
                .is_err()
        );
        server.join().unwrap();
    }
}

#[test]
fn cancellation_stops_a_queued_task_without_downloading() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (url, server) = server(vec![
        ("200 OK", wrapped(json!({"task_id":"job-1"}))),
        ("200 OK", wrapped(json!([{"task_id":"job-1", "status":0}]))),
    ]);
    let checks = AtomicUsize::new(0);
    let error = AceStep::new(&url, None)
        .unwrap()
        .generate(
            &request(),
            || checks.fetch_add(1, Ordering::Relaxed) >= 3,
            |_| {},
        )
        .unwrap_err();
    assert!(error.contains("cancelled"));
    assert_eq!(server.join().unwrap().len(), 2);
}

#[test]
fn health_check_validates_the_service_response() {
    let (url, server) = server(vec![("200 OK", wrapped(json!({"status":"ok"})))]);
    AceStep::new(&url, None).unwrap().health().unwrap();
    assert!(server.join().unwrap()[0].starts_with("GET /health "));
    let (url, server) = self::server(vec![("200 OK", b"<html>not ACE-Step</html>".to_vec())]);
    assert!(AceStep::new(&url, None).unwrap().health().is_err());
    server.join().unwrap();
}
