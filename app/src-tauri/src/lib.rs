//! Tinnitus Music desktop app: one window, two modes.
//!
//! Participant mode plays the music library through the verified notch
//! pipeline (or the control condition the clinician assigned) and logs
//! listening time. Clinician mode, behind a PIN, sets up the participant,
//! runs tinnitus matching, verifies the library and exports logs.

mod audio;
mod clinician;
mod export;
mod feedback;
mod generation;
mod library;
mod matching;
mod player;
mod selection;
mod status;
mod store;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use tauri::{Manager, RunEvent, State};

use tmdsp::matching::{
    Answer, Calibration, Ear, MatchReport, Renderer, Session, SessionConfig, Stage, TinnitusSound,
};
use tmdsp::notch::NotchSpec;
use tmdsp::pipeline::{self, Mode, Options};
use tmdsp::{resample, wav};

use audio::AudioEngine;
use selection::Selection;
use store::{Arm, ListenerPreferences, LogEntry, MusicPreferences, Profile, Settings, Store};
use tmdsp::generation::Recipe;

const DEFAULT_VOLUME_DB: f32 = -6.0;
const MIN_VOLUME_DB: f32 = -40.0;

struct Track {
    id: String,
    recipe: Option<Recipe>,
    path: PathBuf,
    file: String,
    title: String,
    duration_s: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PlayState {
    Idle,
    /// Decoding and verifying the next track.
    Loading,
    Playing,
    Paused,
}

struct Segment {
    start_frames: u64,
    started_at: DateTime<Local>,
    track: String,
    report: String,
}

struct Matching {
    session: Session,
    renderer: Renderer,
    self_setup: bool,
}

struct Inner {
    store: Store,
    profile: Option<Profile>,
    settings: Settings,
    listener_preferences: ListenerPreferences,
    tracks: Vec<Track>,
    current: Option<usize>,
    current_report: String,
    state: PlayState,
    /// Bumped by every play/stop request; a track that finishes preparing
    /// after a newer request is discarded.
    play_gen: u64,
    segment: Option<Segment>,
    matching: Option<Matching>,
    last_match: Option<(MatchReport, String)>,
    unlocked: bool,
    error: Option<String>,
    generation: generation::Job,
}

pub struct AppState {
    inner: Arc<Mutex<Inner>>,
    engine: Arc<AudioEngine>,
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn volume_gain(db: f32) -> f32 {
    if db <= MIN_VOLUME_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

fn interleave(a: &wav::Audio) -> Vec<f32> {
    let l = &a.channels[0];
    let r = a.channels.get(1).unwrap_or(l);
    l.iter().zip(r).flat_map(|(&x, &y)| [x, y]).collect()
}

/// Load → resample to the device rate → notch/control + loudness, verified.
fn prepare(
    path: &Path,
    fs: u32,
    opts: &Options,
) -> Result<(Arc<Vec<f32>>, pipeline::Report), String> {
    let audio = wav::read_wav(path).map_err(|e| format!("{e:#}"))?;
    let audio = resample::resample(&audio, fs)?;
    let (out, mut report) = pipeline::run_to_audio(&audio, opts).map_err(|e| format!("{e:#}"))?;
    report.input_path = Some(path.display().to_string());
    Ok((Arc::new(interleave(&out)), report))
}

fn local_date(rfc3339: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|d| d.with_timezone(&Local).date_naive())
}

impl Inner {
    fn volume_db(&self) -> f32 {
        self.settings.volume_db.unwrap_or(DEFAULT_VOLUME_DB)
    }

    fn require_unlocked(&self) -> Result<(), String> {
        if self.unlocked {
            Ok(())
        } else {
            Err("clinician mode is locked".into())
        }
    }

    fn setup_done(&self) -> bool {
        match &self.profile {
            None => false,
            Some(p) => match p.arm {
                Arm::Passthrough => true,
                Arm::Notch => p.pitch_hz.is_some(),
                Arm::Sham => p.sham_hz.is_some(),
            },
        }
    }

    fn scan_library(&mut self) {
        let current_path = self
            .current
            .and_then(|i| self.tracks.get(i))
            .map(|t| t.path.clone());
        let dir = self.store.library_dir(&self.settings);
        let generated = self.store.dir.join("generated").join("library");
        let mut directories = vec![dir];
        if directories[0] != generated {
            directories.push(generated.clone());
        }
        let mut tracks: Vec<Track> = directories
            .iter()
            .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten())
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let stem = path.file_stem()?.to_string_lossy().to_string();
                let is_wav = path
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("wav"));
                // Skip files that were already processed by `tmusic process`.
                if !is_wav || stem.contains("_notch") || stem.ends_with("_listening") {
                    return None;
                }
                let title = if path.parent() == Some(generated.as_path()) {
                    let (name, variation) = stem.rsplit_once('_').unwrap_or((&stem, ""));
                    format!(
                        "{} · {}",
                        store::track_title(name),
                        variation.rsplit('-').next().unwrap_or("")
                    )
                } else {
                    store::track_title(&stem)
                };
                let recipe = if path.parent() == Some(generated.as_path()) {
                    store::read_recipe(
                        &self
                            .store
                            .dir
                            .join("generated/records")
                            .join(format!("{stem}.json")),
                    )
                } else {
                    None
                };
                Some(Track {
                    id: store::track_id(&path),
                    recipe,
                    duration_s: store::wav_duration(&path)?,
                    title,
                    file: path.file_name()?.to_string_lossy().into(),
                    path,
                })
            })
            .collect();
        tracks.sort_by(|a, b| a.file.cmp(&b.file));
        self.current = current_path.and_then(|path| tracks.iter().position(|t| t.path == path));
        self.tracks = tracks;
    }

    fn options(&self) -> Result<Options, String> {
        let p = self
            .profile
            .as_ref()
            .ok_or("this computer has not been set up yet")?;
        let mode = match p.arm {
            Arm::Notch => Mode::Notch,
            Arm::Passthrough => Mode::Passthrough,
            Arm::Sham => Mode::Sham,
        };
        if mode == Mode::Notch && p.pitch_hz.is_none() {
            return Err("no tinnitus pitch has been set".into());
        }
        let spec = NotchSpec {
            centers_hz: p.pitch_hz.into_iter().collect(),
            width_oct: p.width_oct,
            ..NotchSpec::default()
        };
        Ok(Options {
            mode,
            spec,
            sham_centers_hz: p.sham_hz.into_iter().collect(),
            ..Options::default()
        })
    }

    fn open_segment(&mut self, engine: &AudioEngine) {
        if let Some(i) = self.current {
            self.segment = Some(Segment {
                start_frames: engine.snapshot().played_frames,
                started_at: Local::now(),
                track: self.tracks[i].file.clone(),
                report: self.current_report.clone(),
            });
        }
    }

    /// Seconds of the open segment. Muted playback is not listening.
    fn segment_seconds(&self, engine: &AudioEngine) -> f64 {
        if self.volume_db() <= MIN_VOLUME_DB {
            return 0.0;
        }
        self.segment.as_ref().map_or(0.0, |s| {
            let snap = engine.snapshot();
            snap.played_frames.saturating_sub(s.start_frames) as f64 / snap.fs.max(1) as f64
        })
    }

    fn close_segment(&mut self, engine: &AudioEngine) {
        let secs = self.segment_seconds(engine);
        let Some(seg) = self.segment.take() else {
            return;
        };
        if secs < 1.0 {
            return;
        }
        let entry = LogEntry {
            participant_id: self
                .profile
                .as_ref()
                .map_or("unassigned".into(), |p| p.participant_id.clone()),
            track: seg.track,
            started_at: seg.started_at.to_rfc3339(),
            ended_at: Local::now().to_rfc3339(),
            seconds: (secs * 10.0).round() / 10.0,
            volume_db: self.volume_db(),
            report: seg.report,
        };
        if let Err(e) = self.store.append_log(&entry) {
            self.error = Some(e);
        }
    }

    fn stop(&mut self, engine: &AudioEngine) {
        self.close_segment(engine);
        engine.stop();
        self.state = PlayState::Idle;
        self.play_gen += 1;
    }

    /// Listening minutes per local day, including the segment in progress.
    fn daily_minutes(&self, engine: &AudioEngine) -> BTreeMap<NaiveDate, f64> {
        let mut days = BTreeMap::new();
        let Some(profile) = &self.profile else {
            return days;
        };
        for e in self.store.log_for(&profile.participant_id) {
            if let Some(d) = local_date(&e.started_at) {
                *days.entry(d).or_insert(0.0) += e.seconds / 60.0;
            }
        }
        if let Some(seg) = &self.segment {
            *days.entry(seg.started_at.date_naive()).or_insert(0.0) +=
                self.segment_seconds(engine) / 60.0;
        }
        days
    }
}

/// Resolves a selection atomically, skipping any track that fails verification.
/// Decoding and verification (about a second per track) run without the
/// state lock, so the rest of the app stays responsive meanwhile.
fn play_track(
    inner: &Mutex<Inner>,
    engine: &AudioEngine,
    selection: Selection,
) -> Result<(), String> {
    let (generation, tracks, opts, fs, index) = {
        let mut g = lock(inner);
        let ids: Vec<&str> = g.tracks.iter().map(|t| t.id.as_str()).collect();
        let Some(index) = selection::resolve(
            &selection,
            &ids,
            g.current,
            g.play_gen,
            g.state == PlayState::Playing && engine.snapshot().finished,
            g.state == PlayState::Idle,
        )?
        else {
            return Ok(());
        };
        if g.matching.is_some() {
            return Err("finish or cancel matching first".into());
        }
        g.stop(engine);
        if g.tracks.is_empty() {
            return Err("the music library is empty".into());
        }
        let opts = g.options()?;
        let fs = engine.device_fs()?;
        let tracks: Vec<(PathBuf, String)> = g
            .tracks
            .iter()
            .map(|t| (t.path.clone(), t.title.clone()))
            .collect();
        g.current = Some(index % tracks.len());
        g.state = PlayState::Loading;
        (g.play_gen, tracks, opts, fs, index)
    };
    let n = tracks.len();
    let mut idx = index % n;
    let mut last_error = None;
    for _ in 0..n {
        let (path, title) = &tracks[idx];
        let prepared = prepare(path, fs, &opts);
        let mut g = lock(inner);
        if g.play_gen != generation {
            return Ok(()); // superseded by a newer play or stop request
        }
        match prepared {
            Ok((buffer, report)) => {
                let stem = path
                    .file_stem()
                    .map_or(String::new(), |s| s.to_string_lossy().into());
                let name = format!("{}_{stem}", Local::now().format("%Y%m%d-%H%M%S"));
                let file = match g.store.save_record("reports", &name, &report) {
                    Ok(file) => file,
                    Err(error) => {
                        g.state = PlayState::Idle;
                        return Err(error);
                    }
                };
                if report.all_pass {
                    let Some(current) = g.tracks.iter().position(|track| &track.path == path)
                    else {
                        g.state = PlayState::Idle;
                        return Err("The selected track is no longer in the library.".into());
                    };
                    engine.load(buffer, volume_gain(g.volume_db()), true);
                    g.current = Some(current);
                    g.current_report = file;
                    g.state = PlayState::Playing;
                    g.open_segment(engine);
                    g.error = None;
                    return Ok(());
                }
                last_error = Some(format!(
                    "\u{201c}{title}\u{201d} failed verification and was skipped"
                ));
            }
            Err(e) => last_error = Some(format!("{title}: {e}")),
        }
        g.error = last_error.clone();
        idx = (idx + 1) % n;
        g.current = g
            .tracks
            .iter()
            .position(|track| track.path == tracks[idx].0);
    }
    let mut g = lock(inner);
    if g.play_gen == generation {
        g.state = PlayState::Idle;
    }
    Err(last_error.unwrap_or_else(|| "no playable track".into()))
}

/// Runs `f` off the UI thread with access to the shared state (the
/// closure takes the lock itself, for as long as it needs).
async fn with_shared<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Mutex<Inner>, &AudioEngine) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (inner, engine) = (state.inner.clone(), state.engine.clone());
    tauri::async_runtime::spawn_blocking(move || f(&inner, &engine))
        .await
        .map_err(|e| e.to_string())?
}

/// Runs `f` with the state locked, off the UI thread.
async fn with_state<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&mut Inner, &AudioEngine) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    with_shared(state, move |inner, engine| f(&mut lock(inner), engine)).await
}

/// Advances to the next track when one ends.
fn monitor(inner: Arc<Mutex<Inner>>, engine: Arc<AudioEngine>) {
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let next = {
            let g = lock(&inner);
            (g.matching.is_none() && g.state == PlayState::Playing && engine.snapshot().finished)
                .then_some(g.play_gen)
        };
        if let Some(next) = next
            && let Err(e) = play_track(&inner, &engine, Selection::AfterEnd(next))
        {
            lock(&inner).error = Some(e);
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // TINNITUS_MUSIC_DATA_DIR points the app at another data folder
            // (testing, or one folder per participant on a shared machine).
            let dir = match std::env::var_os("TINNITUS_MUSIC_DATA_DIR") {
                Some(d) => PathBuf::from(d),
                None => app.path().app_data_dir()?,
            };
            let store = Store::open(dir)?;
            let engine = AudioEngine::start();
            let mut inner = Inner {
                profile: store.profile(),
                settings: store.settings(),
                listener_preferences: store.listener_preferences(),
                store,
                tracks: Vec::new(),
                current: None,
                current_report: String::new(),
                state: PlayState::Idle,
                play_gen: 0,
                segment: None,
                matching: None,
                last_match: None,
                unlocked: false,
                error: None,
                generation: generation::Job::default(),
            };
            inner.scan_library();
            let inner = Arc::new(Mutex::new(inner));
            let (i2, e2) = (inner.clone(), engine.clone());
            std::thread::Builder::new()
                .name("player-monitor".into())
                .spawn(move || monitor(i2, e2))?;
            app.manage(AppState { inner, engine });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status::status,
            player::player_play,
            player::player_toggle,
            player::player_skip,
            player::player_stop,
            player::set_volume,
            library::music_preferences_save,
            library::library_favorite,
            library::library_export,
            feedback::music_feedback_save,
            matching::match_start,
            matching::match_play,
            matching::match_answer,
            matching::match_cancel,
            clinician::clinician_unlock,
            clinician::clinician_set_pin,
            clinician::clinician_lock,
            clinician::profile_get,
            clinician::profile_save,
            clinician::last_match_pitch,
            library::library_set_dir,
            library::library_verify,
            clinician::logs_get,
            clinician::logs_export,
            clinician::data_dir,
            generation::generation_start,
            generation::generation_cancel,
            generation::generation_health,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let state = app.state::<AppState>();
                lock(&state.inner).close_segment(&state.engine);
            }
        });
}
