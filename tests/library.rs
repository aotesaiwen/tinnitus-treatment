#[allow(dead_code)]
#[path = "../app/src-tauri/src/export.rs"]
mod export;
#[allow(dead_code)]
#[path = "../app/src-tauri/src/selection.rs"]
mod selection;
#[allow(dead_code)]
#[path = "../app/src-tauri/src/store.rs"]
mod store;

use selection::{Selection, resolve};
use store::{LibraryPreferences, ListenerPreferences, MusicPreferences, Store};
use tmdsp::generation::{Provider, Style};

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "tmusic-library-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn preferences_survive_reopen_and_updates_without_crossing_listeners() {
    let dir = Temporary::new();
    let store = Store::open(dir.0.clone()).unwrap();
    let mut preferences = ListenerPreferences::new();
    preferences.insert(
        "self".into(),
        LibraryPreferences {
            music: MusicPreferences {
                provider: Provider::AceStep,
                style: Style::Unwind,
                duration_s: 120,
                tempo_bpm: 60,
                prompt: "Warm piano".into(),
            },
            favorites: ["track-a".to_string()].into(),
        },
    );
    preferences.insert("study-2".into(), LibraryPreferences::default());
    store.save_listener_preferences(&preferences).unwrap();
    preferences.get_mut("self").unwrap().music.tempo_bpm = 65;
    store.save_listener_preferences(&preferences).unwrap();
    let reopened = Store::open(dir.0.clone()).unwrap().listener_preferences();
    assert_eq!(reopened["self"].music.tempo_bpm, 65);
    assert_eq!(reopened["self"].music.provider, Provider::AceStep);
    assert!(reopened["self"].favorites.contains("track-a"));
    assert!(reopened["study-2"].favorites.is_empty());
    assert_eq!(reopened["study-2"].music.provider, Provider::Composer);
    assert!(!dir.0.join("library_preferences.json.pending").exists());
}

#[test]
fn partial_preferences_have_defaults_and_invalid_values_are_rejected() {
    let preferences: MusicPreferences = serde_json::from_str(r#"{"style":"focus"}"#).unwrap();
    assert_eq!(preferences.duration_s, 60);
    assert!(preferences.validate().is_ok());
    assert!(
        MusicPreferences {
            duration_s: 0,
            ..preferences.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        MusicPreferences {
            prompt: "x".repeat(501),
            ..preferences
        }
        .validate()
        .is_err()
    );
}

#[test]
fn listening_history_is_isolated_when_a_computer_changes_participants() {
    let dir = Temporary::new();
    let store = Store::open(dir.0.clone()).unwrap();
    for (participant, seconds) in [("self", 60.0), ("study-2", 120.0), ("self", 30.0)] {
        store
            .append_log(&store::LogEntry {
                participant_id: participant.into(),
                track: "shared.wav".into(),
                started_at: "2026-10-21T12:00:00+08:00".into(),
                ended_at: "2026-10-21T12:02:00+08:00".into(),
                seconds,
                volume_db: -6.0,
                report: "report.json".into(),
            })
            .unwrap();
    }
    let reopened = Store::open(dir.0.clone()).unwrap();
    assert_eq!(
        reopened
            .log_for("self")
            .iter()
            .map(|e| e.seconds)
            .sum::<f64>(),
        90.0
    );
    assert_eq!(reopened.log_for("study-2").len(), 1);
    assert!(reopened.log_for("new-listener").is_empty());
}

#[test]
fn recipes_expose_only_creation_inputs_and_reject_invalid_records() {
    let dir = Temporary::new();
    std::fs::create_dir_all(&dir.0).unwrap();
    let record = dir.0.join("record.json");
    std::fs::write(
        &record,
        r#"{
        "provider":"ace_step",
        "request":{"style":"focus","duration_s":30,"tempo_bpm":76,"seed":42,"prompt":"Warm piano"},
        "profile_at_generation":{"participant_id":"P099","arm":"sham"},
        "verification":{"private":"not exported"}
    }"#,
    )
    .unwrap();
    let recipe = store::read_recipe(&record).unwrap();
    let serialized = serde_json::to_string(&recipe).unwrap();
    assert!(serialized.contains("Warm piano"));
    for private in ["P099", "sham", "verification", "profile_at_generation"] {
        assert!(!serialized.contains(private));
    }
    std::fs::write(&record, r#"{"provider":"ace_step","request":{"style":"focus","duration_s":0,"tempo_bpm":76,"seed":42,"prompt":""}}"#).unwrap();
    assert!(store::read_recipe(&record).is_none());
}

#[test]
fn selection_keeps_identity_when_generated_tracks_reorder_the_library() {
    let clicked = Selection::Id("b".into());
    assert_eq!(
        resolve(&clicked, &["b", "c"], None, 0, false, true).unwrap(),
        Some(0)
    );
    assert_eq!(
        resolve(&clicked, &["a", "b", "c"], None, 0, false, true).unwrap(),
        Some(1)
    );
    assert!(resolve(&clicked, &["a", "c"], None, 0, false, true).is_err());
    assert_eq!(
        resolve(&Selection::Offset(-1), &["a", "b"], Some(0), 0, false, true).unwrap(),
        Some(1)
    );
}

#[test]
fn stale_auto_advance_and_idle_requests_cannot_restart_stopped_or_paused_audio() {
    let ids = ["a", "b"];
    assert_eq!(
        resolve(&Selection::AfterEnd(7), &ids, Some(0), 7, true, false).unwrap(),
        Some(1)
    );
    assert_eq!(
        resolve(&Selection::AfterEnd(7), &ids, Some(0), 8, true, true).unwrap(),
        None
    );
    assert_eq!(
        resolve(&Selection::AfterEnd(7), &ids, Some(0), 7, false, false).unwrap(),
        None
    );
    assert_eq!(
        resolve(&Selection::IfIdle(7), &ids, Some(0), 8, false, true).unwrap(),
        None
    );
    assert_eq!(
        resolve(&Selection::IfIdle(7), &ids, Some(0), 7, false, false).unwrap(),
        None
    );
}

#[test]
fn exports_cannot_overwrite_or_reenter_source_libraries() {
    let dir = Temporary::new();
    let sources = dir.0.join("sources");
    let output = dir.0.join("exports");
    std::fs::create_dir_all(&sources).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    let source = sources.join("source.wav");
    std::fs::write(&source, b"source").unwrap();
    let protected = vec![source.clone(), sources.clone()];
    assert!(export::destination(&source, &protected).is_err());
    assert!(export::destination(&sources.join("another.wav"), &protected).is_err());
    assert!(export::destination(&output.join("wrong.mp3"), &protected).is_err());
    assert!(export::destination(std::path::Path::new("relative.wav"), &protected).is_err());
    assert!(export::destination(&output.join("listening.wav"), &protected).is_ok());
    #[cfg(unix)]
    {
        let alias = output.join("alias.wav");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        assert!(export::destination(&alias, &protected).is_err());
    }
}

#[test]
fn atomic_writes_replace_complete_files_and_clean_up_failed_outputs() {
    let dir = Temporary::new();
    std::fs::create_dir_all(&dir.0).unwrap();
    let output = dir.0.join("output.wav");
    store::atomic_write(&output, b"first complete file").unwrap();
    store::atomic_write(&output, b"second complete file").unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), b"second complete file");
    let blocked = dir.0.join("folder.wav");
    std::fs::create_dir(&blocked).unwrap();
    assert!(store::atomic_write(&blocked, b"must not replace the directory").is_err());
    assert!(blocked.is_dir());
    assert!(
        std::fs::read_dir(&dir.0)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".pending"))
    );
}

#[test]
fn export_rechecks_current_settings_and_keeps_original_source_bytes() {
    use tmdsp::notch::NotchSpec;
    use tmdsp::{generation, pipeline, wav};
    let dir = Temporary::new();
    std::fs::create_dir_all(&dir.0).unwrap();
    let source = dir.0.join("source.wav");
    let audio = generation::compose(
        &generation::Request {
            style: Style::Focus,
            duration_s: 30,
            tempo_bpm: 76,
            seed: 42,
            prompt: String::new(),
        },
        || false,
    )
    .unwrap();
    let (original, _) = wav::encode_wav(&audio, wav::SampleFormat::F32, 42);
    std::fs::write(&source, &original).unwrap();
    let opts = pipeline::Options {
        spec: NotchSpec {
            centers_hz: vec![6000.0],
            ..Default::default()
        },
        ..Default::default()
    };
    let (copy, report) = export::prepare(&source, &opts).unwrap();
    assert!(report.all_pass && report.warnings.is_empty());
    assert!((report.output.integrated_lufs + 23.0).abs() < 0.2);
    assert_ne!(copy, original);
    assert!(report.input_sha256.is_some());
    let invalid_control = pipeline::Options {
        mode: pipeline::Mode::Sham,
        sham_centers_hz: vec![6000.0],
        ..opts
    };
    assert!(export::prepare(&source, &invalid_control).is_err());
    assert_eq!(std::fs::read(source).unwrap(), original);
}
