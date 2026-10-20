#[allow(dead_code)]
#[path = "../app/src-tauri/src/store.rs"]
mod store;

use store::{FeedbackRequest, Store};
use tmdsp::generation::{Provider, Recipe, Request, Style};

struct Temporary(std::path::PathBuf);

impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "tmusic-feedback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )))
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request() -> FeedbackRequest {
    FeedbackRequest {
        participant_id: "P012".into(),
        track_id: "track-42".into(),
        rating: 4,
        discomfort: Some(false),
    }
}

fn recipe() -> Recipe {
    Recipe {
        provider: Provider::AceStep,
        request: Request {
            style: Style::Focus,
            duration_s: 60,
            tempo_bpm: 76,
            seed: 42,
            prompt: "Gentle piano\nwith soft strings".into(),
        },
    }
}

#[test]
fn feedback_requires_the_current_listener_and_current_playing_generated_track() {
    let request = request();
    let recipe = recipe();
    assert!(
        request
            .validate_for(Some("P012"), Some("track-42"), Some(&recipe), true)
            .is_ok()
    );
    for (participant, track, playing) in [
        (None, Some("track-42"), true),
        (Some("other-listener"), Some("track-42"), true),
        (Some("P012"), None, true),
        (Some("P012"), Some("another-track"), true),
        (Some("P012"), Some("track-42"), false),
    ] {
        assert!(
            request
                .validate_for(participant, track, Some(&recipe), playing)
                .is_err()
        );
    }
    assert!(
        request
            .validate_for(Some("P012"), Some("track-42"), None, true)
            .is_err()
    );
}

#[test]
fn malformed_ratings_and_recipes_never_create_feedback_files() {
    let dir = Temporary::new();
    let store = Store::open(dir.0.clone()).unwrap();
    for rating in [0, 6, 255] {
        let invalid = FeedbackRequest {
            rating,
            ..request()
        };
        assert!(
            store
                .append_feedback(&invalid, &recipe(), "2026-10-21T12:00:00+08:00")
                .is_err()
        );
    }
    for participant_id in ["", " ", "\n\t"] {
        let invalid = FeedbackRequest {
            participant_id: participant_id.into(),
            ..request()
        };
        assert!(
            store
                .append_feedback(&invalid, &recipe(), "2026-10-21T12:00:00+08:00")
                .is_err()
        );
    }
    for track_id in [
        "".into(),
        " ".into(),
        " track".into(),
        "track\n".into(),
        "x".repeat(129),
    ] {
        let invalid = FeedbackRequest {
            track_id,
            ..request()
        };
        assert!(
            store
                .append_feedback(&invalid, &recipe(), "2026-10-21T12:00:00+08:00")
                .is_err()
        );
    }
    let mut invalid_recipe = recipe();
    invalid_recipe.request.duration_s = 0;
    assert!(
        store
            .append_feedback(&request(), &invalid_recipe, "2026-10-21T12:00:00+08:00")
            .is_err()
    );
    invalid_recipe = recipe();
    invalid_recipe.provider = Provider::Composer;
    assert!(
        store
            .append_feedback(&request(), &invalid_recipe, "2026-10-21T12:00:00+08:00")
            .is_err()
    );
    assert!(!dir.0.join("music_feedback.jsonl").exists());
}

#[test]
fn frontend_cannot_submit_a_recipe_or_clinical_fields_with_feedback() {
    for extra in [
        r#""recipe":{"provider":"composer"}"#,
        r#""arm":"sham""#,
        r#""created_at":"invented""#,
    ] {
        let json = format!(
            r#"{{"participant_id":"P012","track_id":"track-42","rating":4,"discomfort":false,{extra}}}"#
        );
        assert!(serde_json::from_str::<FeedbackRequest>(&json).is_err());
    }
    for invalid_rating in ["1.5", "true", "-1", "256"] {
        let json = format!(
            r#"{{"participant_id":"P012","track_id":"track-42","rating":{invalid_rating},"discomfort":false}}"#
        );
        assert!(serde_json::from_str::<FeedbackRequest>(&json).is_err());
    }
}

#[test]
fn existing_nonblank_participant_identifiers_are_preserved_exactly() {
    let dir = Temporary::new();
    let store = Store::open(dir.0.clone()).unwrap();
    let recipe = recipe();
    let identities = [" P012 ".into(), "P012\n".into(), "participant".repeat(30)];
    for identity in &identities {
        let request = FeedbackRequest {
            participant_id: identity.clone(),
            ..request()
        };
        assert!(
            request
                .validate_for(Some(identity), Some("track-42"), Some(&recipe), true)
                .is_ok()
        );
        assert!(
            request
                .validate_for(Some("P012"), Some("track-42"), Some(&recipe), true)
                .is_err()
        );
        store
            .append_feedback(&request, &recipe, "2026-10-21T12:00:00+08:00")
            .unwrap();
    }
    let text = std::fs::read_to_string(dir.0.join("music_feedback.jsonl")).unwrap();
    let saved: Vec<String> = text
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["participant_id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(saved, identities);
}

#[test]
fn feedback_appends_unique_versioned_events_with_original_recipe_and_optional_discomfort() {
    let dir = Temporary::new();
    let store = Store::open(dir.0.clone()).unwrap();
    let path = dir.0.join("music_feedback.jsonl");
    let first_id = store
        .append_feedback(&request(), &recipe(), "2026-10-21T12:00:00+08:00")
        .unwrap();
    let first_bytes = std::fs::read(&path).unwrap();
    let mut original_recipe = recipe();
    original_recipe.request.seed = 7;
    let reopened = Store::open(dir.0.clone()).unwrap();
    let second_id = reopened
        .append_feedback(
            &FeedbackRequest {
                participant_id: "another-listener".into(),
                rating: 2,
                discomfort: None,
                ..request()
            },
            &original_recipe,
            "2026-10-21T12:01:00+08:00",
        )
        .unwrap();
    let third_id = reopened
        .append_feedback(
            &FeedbackRequest {
                rating: 1,
                discomfort: Some(true),
                ..request()
            },
            &recipe(),
            "2026-10-21T12:01:00+08:00",
        )
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(&first_bytes));
    assert!(bytes.ends_with(b"\n"));
    assert_eq!(
        [&first_id, &second_id, &third_id]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    let events: Vec<serde_json::Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["schema_version"], 1);
    assert_eq!(events[0]["event_id"], first_id);
    assert_eq!(events[0]["participant_id"], "P012");
    assert_eq!(events[1]["participant_id"], "another-listener");
    assert_eq!(events[0]["track_id"], "track-42");
    assert_eq!(events[0]["recipe"], serde_json::to_value(recipe()).unwrap());
    assert_eq!(events[1]["recipe"]["request"]["seed"], 7);
    assert_eq!(events[0]["rating"], 4);
    assert_eq!(events[0]["discomfort"], false);
    assert!(events[1]["discomfort"].is_null());
    assert_eq!(events[2]["discomfort"], true);
    assert_eq!(events[0]["created_at"], "2026-10-21T12:00:00+08:00");
    assert_eq!(
        events[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "created_at",
            "discomfort",
            "event_id",
            "participant_id",
            "rating",
            "recipe",
            "schema_version",
            "track_id",
        ]
    );
}

#[test]
fn failed_persistence_reports_an_error_without_creating_a_success_record() {
    let dir = Temporary::new();
    let store = Store::open(dir.0.clone()).unwrap();
    let path = dir.0.join("music_feedback.jsonl");
    std::fs::create_dir(&path).unwrap();
    assert!(
        store
            .append_feedback(&request(), &recipe(), "2026-10-21T12:00:00+08:00")
            .unwrap_err()
            .contains("Could not read music feedback")
    );
    assert!(path.is_dir());
    assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
}

#[test]
fn failed_validation_and_publication_preserve_history_and_retry_adds_only_one_event() {
    let dir = Temporary::new();
    let store = Store::open(dir.0.clone()).unwrap();
    let path = dir.0.join("music_feedback.jsonl");
    std::fs::write(&path, b"").unwrap();
    store
        .append_feedback(&request(), &recipe(), "2026-10-21T12:00:00+08:00")
        .unwrap();
    let previous = std::fs::read(&path).unwrap();
    let invalid = FeedbackRequest {
        rating: 0,
        ..request()
    };
    assert!(
        store
            .append_feedback(&invalid, &recipe(), "2026-10-21T12:01:00+08:00")
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    let error = store
        .append_feedback_with(
            &request(),
            &recipe(),
            "2026-10-21T12:01:00+08:00",
            |destination, prepared| {
                assert_eq!(destination, path);
                assert!(prepared.starts_with(&previous));
                assert_eq!(std::str::from_utf8(prepared).unwrap().lines().count(), 2);
                Err("simulated publication failure".into())
            },
        )
        .unwrap_err();
    assert!(error.contains("Could not save music feedback: simulated publication failure"));
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    store
        .append_feedback(&request(), &recipe(), "2026-10-21T12:01:00+08:00")
        .unwrap();
    let retried = std::fs::read_to_string(path).unwrap();
    assert!(retried.as_bytes().starts_with(&previous));
    assert_eq!(retried.lines().count(), 2);
}

#[test]
fn a_partial_ledger_is_preserved_and_refused_before_publication() {
    let dir = Temporary::new();
    let store = Store::open(dir.0.clone()).unwrap();
    let path = dir.0.join("music_feedback.jsonl");
    store
        .append_feedback(&request(), &recipe(), "2026-10-21T12:00:00+08:00")
        .unwrap();
    let mut partial = std::fs::read(&path).unwrap();
    partial.extend_from_slice(br#"{"schema_version":1,"event_id":"interrupted""#);
    std::fs::write(&path, &partial).unwrap();
    let error = store
        .append_feedback_with(
            &request(),
            &recipe(),
            "2026-10-21T12:01:00+08:00",
            |_, _| panic!("an incomplete ledger must never be published"),
        )
        .unwrap_err();
    assert!(error.contains("incomplete final record"));
    assert_eq!(std::fs::read(path).unwrap(), partial);
}
