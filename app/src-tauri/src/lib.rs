//! Tinnitus Music desktop app: one window, two modes.
//!
//! Participant mode plays the music library through the verified notch
//! pipeline (or the control condition the clinician assigned) and logs
//! listening time. Clinician mode, behind a PIN, sets up the participant,
//! runs tinnitus matching, verifies the library and exports logs.

mod audio;
mod store;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use tauri::{Manager, RunEvent, State};

use tmdsp::matching::{Answer, Calibration, Ear, MatchReport, Renderer, Session, SessionConfig, Stage, TinnitusSound};
use tmdsp::notch::NotchSpec;
use tmdsp::pipeline::{self, Mode, Options};
use tmdsp::{resample, wav};

use audio::AudioEngine;
use store::{Arm, LogEntry, Profile, Settings, Store};

const DEFAULT_VOLUME_DB: f32 = -6.0;
const MIN_VOLUME_DB: f32 = -40.0;

struct Track {
    path: PathBuf,
    file: String,
    title: String,
    duration_s: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum PlayState {
    Idle,
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
    tracks: Vec<Track>,
    current: Option<usize>,
    current_report: String,
    state: PlayState,
    segment: Option<Segment>,
    matching: Option<Matching>,
    last_match: Option<(MatchReport, String)>,
    unlocked: bool,
    error: Option<String>,
}

pub struct AppState {
    inner: Arc<Mutex<Inner>>,
    engine: Arc<AudioEngine>,
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn volume_gain(db: f32) -> f32 {
    if db <= MIN_VOLUME_DB { 0.0 } else { 10f32.powf(db / 20.0) }
}

fn interleave(a: &wav::Audio) -> Vec<f32> {
    let l = &a.channels[0];
    let r = a.channels.get(1).unwrap_or(l);
    l.iter().zip(r).flat_map(|(&x, &y)| [x, y]).collect()
}

/// Load → resample to the device rate → notch/control + loudness, verified.
fn prepare(path: &Path, fs: u32, opts: &Options) -> Result<(Arc<Vec<f32>>, pipeline::Report), String> {
    let audio = wav::read_wav(path).map_err(|e| format!("{e:#}"))?;
    let audio = resample::resample(&audio, fs)?;
    let (out, mut report) = pipeline::run_to_audio(&audio, opts).map_err(|e| format!("{e:#}"))?;
    report.input_path = Some(path.display().to_string());
    Ok((Arc::new(interleave(&out)), report))
}

fn local_date(rfc3339: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(rfc3339).ok().map(|d| d.with_timezone(&Local).date_naive())
}

impl Inner {
    fn volume_db(&self) -> f32 {
        self.settings.volume_db.unwrap_or(DEFAULT_VOLUME_DB)
    }

    fn require_unlocked(&self) -> Result<(), String> {
        if self.unlocked { Ok(()) } else { Err("clinician mode is locked".into()) }
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
        let dir = self.store.library_dir(&self.settings);
        let mut tracks: Vec<Track> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let stem = path.file_stem()?.to_string_lossy().to_string();
                let is_wav = path.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav"));
                // Skip files that were already processed by `tmusic process`.
                if !is_wav || stem.contains("_notch") {
                    return None;
                }
                Some(Track {
                    duration_s: store::wav_duration(&path)?,
                    title: store::track_title(&stem),
                    file: path.file_name()?.to_string_lossy().into(),
                    path,
                })
            })
            .collect();
        tracks.sort_by(|a, b| a.file.cmp(&b.file));
        self.tracks = tracks;
        self.current = None;
    }

    fn options(&self) -> Result<Options, String> {
        let p = self.profile.as_ref().ok_or("this computer has not been set up yet")?;
        let mode = match p.arm {
            Arm::Notch => Mode::Notch,
            Arm::Passthrough => Mode::Passthrough,
            Arm::Sham => Mode::Sham,
        };
        if mode == Mode::Notch && p.pitch_hz.is_none() {
            return Err("no tinnitus pitch has been set".into());
        }
        let spec = NotchSpec { centers_hz: p.pitch_hz.into_iter().collect(), width_oct: p.width_oct, ..NotchSpec::default() };
        Ok(Options { mode, spec, sham_centers_hz: p.sham_hz.into_iter().collect(), ..Options::default() })
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
        let Some(seg) = self.segment.take() else { return };
        if secs < 1.0 {
            return;
        }
        let entry = LogEntry {
            participant_id: self.profile.as_ref().map_or("unassigned".into(), |p| p.participant_id.clone()),
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
    }

    /// Plays track `index`, skipping any track that fails verification.
    fn play_index(&mut self, engine: &AudioEngine, index: usize) -> Result<(), String> {
        self.stop(engine);
        let n = self.tracks.len();
        if n == 0 {
            return Err("the music library is empty".into());
        }
        let opts = self.options()?;
        let fs = engine.device_fs()?;
        let mut idx = index % n;
        for _ in 0..n {
            let (path, title) = (self.tracks[idx].path.clone(), self.tracks[idx].title.clone());
            match prepare(&path, fs, &opts) {
                Ok((buffer, report)) => {
                    let stem = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into());
                    let name = format!("{}_{stem}", Local::now().format("%Y%m%d-%H%M%S"));
                    let file = self.store.save_record("reports", &name, &report)?;
                    if report.all_pass {
                        engine.load(buffer, volume_gain(self.volume_db()), true);
                        self.current = Some(idx);
                        self.current_report = file;
                        self.state = PlayState::Playing;
                        self.open_segment(engine);
                        self.error = None;
                        return Ok(());
                    }
                    self.error = Some(format!("\u{201c}{title}\u{201d} failed verification and was skipped"));
                }
                Err(e) => self.error = Some(format!("{title}: {e}")),
            }
            idx = (idx + 1) % n;
        }
        Err(self.error.clone().unwrap_or_else(|| "no playable track".into()))
    }

    /// Listening minutes per local day, including the segment in progress.
    fn daily_minutes(&self, engine: &AudioEngine) -> BTreeMap<NaiveDate, f64> {
        let mut days = BTreeMap::new();
        for e in self.store.log() {
            if let Some(d) = local_date(&e.started_at) {
                *days.entry(d).or_insert(0.0) += e.seconds / 60.0;
            }
        }
        if let Some(seg) = &self.segment {
            *days.entry(seg.started_at.date_naive()).or_insert(0.0) += self.segment_seconds(engine) / 60.0;
        }
        days
    }
}

/// Runs `f` with the state locked, off the UI thread.
async fn with_state<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&mut Inner, &AudioEngine) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (inner, engine) = (state.inner.clone(), state.engine.clone());
    tauri::async_runtime::spawn_blocking(move || f(&mut lock(&inner), &engine)).await.map_err(|e| e.to_string())?
}

// ---------- status ----------

#[derive(Serialize)]
struct DayTotal {
    date: String,
    minutes: f64,
}

#[derive(Serialize)]
struct PlayerStatus {
    state: PlayState,
    index: Option<usize>,
    title: Option<String>,
    position_s: f64,
    duration_s: f64,
}

#[derive(Serialize)]
struct Status {
    setup_done: bool,
    participant_id: Option<String>,
    daily_goal_min: u32,
    today_min: f64,
    week: Vec<DayTotal>,
    player: PlayerStatus,
    volume_db: f32,
    tracks: Vec<String>,
    library_dir: String,
    clinician_unlocked: bool,
    pin_set: bool,
    device_fs: Option<u32>,
    matching_active: bool,
    error: Option<String>,
}

#[tauri::command]
async fn status(state: State<'_, AppState>) -> Result<Status, String> {
    with_state(&state, |g, engine| {
        let snap = engine.snapshot();
        let days = g.daily_minutes(engine);
        let today = Local::now().date_naive();
        let week = (0..7)
            .rev()
            .map(|k| {
                let d = today - chrono::Days::new(k);
                DayTotal { date: d.to_string(), minutes: days.get(&d).copied().unwrap_or(0.0) }
            })
            .collect();
        let music = g.matching.is_none() && g.state != PlayState::Idle;
        Ok(Status {
            setup_done: g.setup_done(),
            participant_id: g.profile.as_ref().map(|p| p.participant_id.clone()),
            daily_goal_min: g.profile.as_ref().map_or(60, |p| p.daily_goal_min),
            today_min: days.get(&today).copied().unwrap_or(0.0),
            week,
            player: PlayerStatus {
                state: g.state,
                index: g.current,
                title: g.current.map(|i| g.tracks[i].title.clone()),
                position_s: if music { snap.pos_frames as f64 / snap.fs.max(1) as f64 } else { 0.0 },
                duration_s: g.current.map_or(0.0, |i| g.tracks[i].duration_s),
            },
            volume_db: g.volume_db(),
            tracks: g.tracks.iter().map(|t| t.title.clone()).collect(),
            library_dir: g.store.library_dir(&g.settings).display().to_string(),
            clinician_unlocked: g.unlocked,
            pin_set: g.settings.pin_sha256.is_some(),
            device_fs: engine.device_fs().ok(),
            matching_active: g.matching.is_some(),
            error: g.error.clone().or_else(|| engine.error()),
        })
    })
    .await
}

// ---------- player ----------

#[tauri::command]
async fn player_play(state: State<'_, AppState>, index: Option<usize>) -> Result<(), String> {
    with_state(&state, move |g, engine| {
        if g.matching.is_some() {
            return Err("finish or cancel matching first".into());
        }
        let i = index.or(g.current).unwrap_or(0);
        g.play_index(engine, i)
    })
    .await
}

#[tauri::command]
async fn player_toggle(state: State<'_, AppState>) -> Result<(), String> {
    with_state(&state, |g, engine| match g.state {
        PlayState::Playing => {
            g.close_segment(engine);
            engine.set_playing(false);
            g.state = PlayState::Paused;
            Ok(())
        }
        PlayState::Paused => {
            engine.set_playing(true);
            g.state = PlayState::Playing;
            g.open_segment(engine);
            Ok(())
        }
        PlayState::Idle => {
            let i = g.current.unwrap_or(0);
            g.play_index(engine, i)
        }
    })
    .await
}

#[tauri::command]
async fn player_skip(state: State<'_, AppState>, delta: i32) -> Result<(), String> {
    with_state(&state, move |g, engine| {
        let n = g.tracks.len().max(1) as i64;
        let cur = g.current.map_or(0, |i| i as i64);
        g.play_index(engine, (cur + delta as i64).rem_euclid(n) as usize)
    })
    .await
}

#[tauri::command]
async fn player_stop(state: State<'_, AppState>) -> Result<(), String> {
    with_state(&state, |g, engine| {
        g.stop(engine);
        Ok(())
    })
    .await
}

#[tauri::command]
async fn set_volume(state: State<'_, AppState>, db: f32) -> Result<(), String> {
    with_state(&state, move |g, engine| {
        let db = db.clamp(MIN_VOLUME_DB, 0.0);
        let music_playing = g.matching.is_none() && g.state == PlayState::Playing;
        // Split the log segment so time at each volume (and muted time,
        // which is not counted) is recorded correctly.
        if music_playing {
            g.close_segment(engine);
        }
        g.settings.volume_db = Some(db);
        if g.matching.is_none() {
            engine.set_gain(volume_gain(db));
        }
        if music_playing {
            g.open_segment(engine);
        }
        g.store.save_settings(&g.settings)
    })
    .await
}

// ---------- matching ----------

#[derive(Deserialize)]
struct MatchStart {
    ear: Ear,
    sound: TinnitusSound,
    repeats: usize,
}

#[derive(Serialize)]
struct AnswerOption {
    answer: Answer,
    label: &'static str,
}

#[derive(Serialize)]
struct MatchSummary {
    pitch_hz: f64,
    pitch_matches_hz: Vec<f64>,
    pitch_spread_oct: f64,
    octave_shift: i32,
    loudness_sl_db: Option<f64>,
    mml_sl_db: Option<f64>,
    retest_recommended: bool,
    warnings: Vec<String>,
    trials: usize,
    report_file: String,
    self_setup: bool,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum MatchView {
    Prompt { stage: String, step: usize, steps: usize, question: &'static str, options: Vec<AnswerOption>, trial: usize },
    Done { summary: MatchSummary },
}

fn answer_label(a: Answer) -> &'static str {
    match a {
        Answer::Yes => "Yes",
        Answer::No => "No",
        Answer::Louder => "Louder than my tinnitus",
        Answer::Softer => "Softer than my tinnitus",
        Answer::Equal => "About the same",
        Answer::First => "First sound",
        Answer::Second => "Second sound",
        Answer::Same => "Can't tell",
    }
}

fn stage_label(stage: Stage, repeats: usize) -> (String, usize) {
    match stage {
        Stage::Threshold { hz } => (format!("Hearing check at {hz:.0} Hz"), 1),
        Stage::LoudnessReference => ("How loud is your tinnitus?".into(), 2),
        Stage::Pitch { run } => (format!("Finding your tinnitus pitch (round {} of {repeats})", run + 1), 3),
        Stage::OctaveCheck => ("Double-checking the pitch".into(), 4),
        Stage::ThresholdAtPitch => ("Hearing check at your tinnitus pitch".into(), 5),
        Stage::LoudnessAtPitch => ("Loudness at your tinnitus pitch".into(), 5),
        Stage::NoiseThreshold => ("Noise check".into(), 6),
        Stage::MaskingLevel => ("How much noise covers your tinnitus?".into(), 6),
    }
}

fn match_view(g: &mut Inner) -> Result<MatchView, String> {
    let m = g.matching.as_ref().ok_or("matching is not running")?;
    if let Some(p) = m.session.next_prompt() {
        let (stage, step) = stage_label(p.stage, m.session.config().pitch_repeats);
        return Ok(MatchView::Prompt {
            stage,
            step,
            steps: 6,
            question: p.question.text(),
            options: p.question.options().iter().map(|&a| AnswerOption { answer: a, label: answer_label(a) }).collect(),
            trial: m.session.log().len() + 1,
        });
    }
    let m = g.matching.take().expect("checked above");
    let r = m.session.report().ok_or("matching ended without a result")?;
    let file = g.store.save_record("matching", &Local::now().format("%Y%m%d-%H%M%S").to_string(), &r)?;
    if m.self_setup {
        let p = Profile {
            participant_id: "self".into(),
            arm: Arm::Notch,
            pitch_hz: Some(r.pitch_hz),
            sham_hz: None,
            width_oct: 1.0,
            daily_goal_min: 60,
            matching_report: Some(file.clone()),
            updated_at: Local::now().to_rfc3339(),
        };
        g.store.save_profile(&p)?;
        g.profile = Some(p);
    }
    let summary = MatchSummary {
        pitch_hz: r.pitch_hz,
        pitch_matches_hz: r.pitch_matches_hz.clone(),
        pitch_spread_oct: r.pitch_spread_oct,
        octave_shift: r.octave_shift,
        loudness_sl_db: r.loudness_at_pitch.sensation_level_db,
        mml_sl_db: r.mml_sensation_level_db,
        retest_recommended: r.retest_recommended,
        warnings: r.warnings.clone(),
        trials: r.trials,
        report_file: file.clone(),
        self_setup: m.self_setup,
    };
    g.last_match = Some((r, file));
    Ok(MatchView::Done { summary })
}

#[tauri::command]
async fn match_start(state: State<'_, AppState>, opts: MatchStart) -> Result<MatchView, String> {
    with_state(&state, move |g, engine| {
        let self_setup = !g.unlocked;
        if self_setup && g.setup_done() {
            return Err("this computer is already set up; ask your clinician to repeat matching".into());
        }
        g.stop(engine);
        let fs = engine.device_fs()?;
        let mut cfg = SessionConfig::new(fs as f64, Calibration::default());
        cfg.ear = opts.ear;
        cfg.tinnitus_sound = opts.sound;
        cfg.pitch_repeats = opts.repeats.clamp(1, 5);
        cfg.seed = Local::now().timestamp_nanos_opt().unwrap_or(1) as u64;
        let renderer = Renderer::new(cfg.fs, cfg.calibration.clone(), cfg.max_level_db, cfg.seed ^ 0xA5A5);
        let session = Session::new(cfg)?;
        g.matching = Some(Matching { session, renderer, self_setup });
        match_view(g)
    })
    .await
}

/// Plays the current prompt's sounds and returns when they have finished.
#[tauri::command]
async fn match_play(state: State<'_, AppState>) -> Result<(), String> {
    let (inner, engine) = (state.inner.clone(), state.engine.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let buffer = {
            let mut g = lock(&inner);
            let m = g.matching.as_mut().ok_or("matching is not running")?;
            let p = m.session.next_prompt().ok_or("no sound to play")?;
            let mono = m.renderer.render_all(&p.stimuli)?;
            let silent = 0.0f32;
            let stereo: Vec<f32> = mono
                .iter()
                .flat_map(|&x| match p.ear {
                    Ear::Both => [x, x],
                    Ear::Left => [x, silent],
                    Ear::Right => [silent, x],
                })
                .collect();
            Arc::new(stereo)
        };
        // Test sounds play at their calibrated level, ignoring the volume slider.
        engine.load(buffer, 1.0, false);
        let t = Instant::now();
        while !engine.snapshot().finished && t.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn match_answer(state: State<'_, AppState>, answer: Answer) -> Result<MatchView, String> {
    with_state(&state, move |g, _| {
        g.matching.as_mut().ok_or("matching is not running")?.session.answer(answer)?;
        match_view(g)
    })
    .await
}

#[tauri::command]
async fn match_cancel(state: State<'_, AppState>) -> Result<(), String> {
    with_state(&state, |g, engine| {
        g.matching = None;
        engine.stop();
        Ok(())
    })
    .await
}

// ---------- clinician ----------

#[tauri::command]
async fn clinician_unlock(state: State<'_, AppState>, pin: String) -> Result<bool, String> {
    with_state(&state, move |g, _| {
        let ok = g.settings.pin_sha256.as_deref() == Some(store::hash_pin(&pin).as_str());
        g.unlocked = ok;
        Ok(ok)
    })
    .await
}

#[tauri::command]
async fn clinician_set_pin(state: State<'_, AppState>, pin: String) -> Result<(), String> {
    with_state(&state, move |g, _| {
        if g.settings.pin_sha256.is_some() {
            g.require_unlocked()?;
        }
        if pin.chars().count() < 4 {
            return Err("use at least 4 digits".into());
        }
        g.settings.pin_sha256 = Some(store::hash_pin(&pin));
        g.store.save_settings(&g.settings)?;
        g.unlocked = true;
        Ok(())
    })
    .await
}

#[tauri::command]
async fn clinician_lock(state: State<'_, AppState>) -> Result<(), String> {
    with_state(&state, |g, _| {
        g.unlocked = false;
        Ok(())
    })
    .await
}

#[tauri::command]
async fn profile_get(state: State<'_, AppState>) -> Result<Option<Profile>, String> {
    with_state(&state, |g, _| {
        g.require_unlocked()?;
        Ok(g.profile.clone())
    })
    .await
}

#[tauri::command]
async fn profile_save(state: State<'_, AppState>, mut profile: Profile) -> Result<(), String> {
    with_state(&state, move |g, engine| {
        g.require_unlocked()?;
        if profile.participant_id.trim().is_empty() {
            return Err("participant ID is required".into());
        }
        if profile.arm == Arm::Sham && profile.sham_hz.is_none() {
            return Err("the sham arm needs a sham notch frequency".into());
        }
        if profile.arm == Arm::Notch && profile.pitch_hz.is_none() {
            return Err("the notch arm needs a tinnitus pitch".into());
        }
        profile.updated_at = Local::now().to_rfc3339();
        g.stop(engine);
        g.store.save_profile(&profile)?;
        g.profile = Some(profile);
        Ok(())
    })
    .await
}

/// The most recent matching result, for filling in the profile.
#[tauri::command]
async fn last_match_pitch(state: State<'_, AppState>) -> Result<Option<(f64, String)>, String> {
    with_state(&state, |g, _| {
        g.require_unlocked()?;
        Ok(g.last_match.as_ref().map(|(r, f)| (r.pitch_hz, f.clone())))
    })
    .await
}

#[tauri::command]
async fn library_set_dir(state: State<'_, AppState>, dir: String) -> Result<(), String> {
    with_state(&state, move |g, engine| {
        g.require_unlocked()?;
        if !Path::new(&dir).is_dir() {
            return Err(format!("{dir} is not a folder"));
        }
        g.stop(engine);
        g.settings.library_dir = Some(dir);
        g.store.save_settings(&g.settings)?;
        g.scan_library();
        Ok(())
    })
    .await
}

#[derive(Serialize)]
struct VerifyRow {
    title: String,
    file: String,
    duration_s: f64,
    all_pass: bool,
    attenuation_db: Option<f64>,
    input_lufs: f64,
    failed_checks: Vec<String>,
    warnings: Vec<String>,
}

/// Runs every track through the participant's processing and reports the checks.
#[tauri::command]
async fn library_verify(state: State<'_, AppState>) -> Result<Vec<VerifyRow>, String> {
    let (inner, _) = (state.inner.clone(), state.engine.clone());
    let (tracks, opts) = tauri::async_runtime::spawn_blocking(move || {
        let g = lock(&inner);
        g.require_unlocked()?;
        let tracks: Vec<(PathBuf, String, String, f64)> =
            g.tracks.iter().map(|t| (t.path.clone(), t.title.clone(), t.file.clone(), t.duration_s)).collect();
        Ok::<_, String>((tracks, g.options()?))
    })
    .await
    .map_err(|e| e.to_string())??;
    tauri::async_runtime::spawn_blocking(move || {
        tracks
            .into_iter()
            .map(|(path, title, file, duration_s)| {
                let audio = wav::read_wav(&path).map_err(|e| format!("{title}: {e:#}"))?;
                let (_, r) = pipeline::run(&audio, &opts).map_err(|e| format!("{title}: {e:#}"))?;
                Ok(VerifyRow {
                    title,
                    file,
                    duration_s,
                    all_pass: r.all_pass,
                    attenuation_db: r.measurement.as_ref().and_then(|m| m.bands.first()).map(|b| b.attenuation_db),
                    input_lufs: r.input.integrated_lufs,
                    failed_checks: r.checks.iter().filter(|c| !c.pass).map(|c| format!("{} {}", c.id, c.description)).collect(),
                    warnings: r.warnings,
                })
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct Logs {
    entries: Vec<LogEntry>,
    days: Vec<DayTotal>,
}

#[tauri::command]
async fn logs_get(state: State<'_, AppState>) -> Result<Logs, String> {
    with_state(&state, |g, engine| {
        g.require_unlocked()?;
        let days = g.daily_minutes(engine).into_iter().rev().map(|(d, m)| DayTotal { date: d.to_string(), minutes: m }).collect();
        let mut entries = g.store.log();
        entries.reverse();
        Ok(Logs { entries, days })
    })
    .await
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

#[tauri::command]
async fn logs_export(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    with_state(&state, move |g, _| {
        g.require_unlocked()?;
        let entries = g.store.log();
        let mut csv = String::from("participant_id,track,started_at,ended_at,seconds,volume_db,report\n");
        for e in &entries {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                csv_field(&e.participant_id),
                csv_field(&e.track),
                e.started_at,
                e.ended_at,
                e.seconds,
                e.volume_db,
                csv_field(&e.report)
            ));
        }
        std::fs::write(&path, csv).map_err(|e| format!("writing {path}: {e}"))?;
        Ok(entries.len())
    })
    .await
}

#[tauri::command]
async fn data_dir(state: State<'_, AppState>) -> Result<String, String> {
    with_state(&state, |g, _| Ok(g.store.dir.display().to_string())).await
}

/// Advances to the next track when one ends.
fn monitor(inner: Arc<Mutex<Inner>>, engine: Arc<AudioEngine>) {
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let mut g = lock(&inner);
        if g.matching.is_none() && g.state == PlayState::Playing && engine.snapshot().finished {
            let next = g.current.map_or(0, |i| i + 1);
            if let Err(e) = g.play_index(&engine, next) {
                g.error = Some(e);
                g.state = PlayState::Idle;
            }
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let store = Store::open(app.path().app_data_dir()?)?;
            let engine = AudioEngine::start();
            let mut inner = Inner {
                profile: store.profile(),
                settings: store.settings(),
                store,
                tracks: Vec::new(),
                current: None,
                current_report: String::new(),
                state: PlayState::Idle,
                segment: None,
                matching: None,
                last_match: None,
                unlocked: false,
                error: None,
            };
            inner.scan_library();
            let inner = Arc::new(Mutex::new(inner));
            let (i2, e2) = (inner.clone(), engine.clone());
            std::thread::Builder::new().name("player-monitor".into()).spawn(move || monitor(i2, e2))?;
            app.manage(AppState { inner, engine });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            player_play,
            player_toggle,
            player_skip,
            player_stop,
            set_volume,
            match_start,
            match_play,
            match_answer,
            match_cancel,
            clinician_unlock,
            clinician_set_pin,
            clinician_lock,
            profile_get,
            profile_save,
            last_match_pitch,
            library_set_dir,
            library_verify,
            logs_get,
            logs_export,
            data_dir,
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
