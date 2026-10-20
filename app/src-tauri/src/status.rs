use super::*;

// ---------- status ----------

#[derive(Serialize)]
pub(super) struct DayTotal {
    pub(super) date: String,
    pub(super) minutes: f64,
}

#[derive(Serialize)]
pub(super) struct PlayerStatus {
    state: PlayState,
    track_id: Option<String>,
    title: Option<String>,
    position_s: f64,
    duration_s: f64,
}

#[derive(Serialize)]
pub(super) struct TrackView {
    id: String,
    title: String,
    duration_s: f64,
    favorite: bool,
    recipe: Option<Recipe>,
}

#[derive(Serialize)]
pub(super) struct Status {
    setup_done: bool,
    participant_id: Option<String>,
    daily_goal_min: u32,
    today_min: f64,
    week: Vec<DayTotal>,
    player: PlayerStatus,
    volume_db: f32,
    library: Vec<TrackView>,
    music_preferences: MusicPreferences,
    library_dir: String,
    clinician_unlocked: bool,
    pin_set: bool,
    device_fs: Option<u32>,
    matching_active: bool,
    error: Option<String>,
    generation: generation::Job,
    can_generate: bool,
}

#[tauri::command]
pub(super) async fn status(state: State<'_, AppState>) -> Result<Status, String> {
    with_state(&state, |g, engine| {
        let snap = engine.snapshot();
        let days = g.daily_minutes(engine);
        let today = Local::now().date_naive();
        let week = (0..7)
            .rev()
            .map(|k| {
                let d = today - chrono::Days::new(k);
                DayTotal {
                    date: d.to_string(),
                    minutes: days.get(&d).copied().unwrap_or(0.0),
                }
            })
            .collect();
        let preferences = g
            .profile
            .as_ref()
            .and_then(|p| g.listener_preferences.get(&p.participant_id))
            .cloned()
            .unwrap_or_default();
        let music = g.matching.is_none() && g.state != PlayState::Idle;
        Ok(Status {
            setup_done: g.setup_done(),
            participant_id: g.profile.as_ref().map(|p| p.participant_id.clone()),
            daily_goal_min: g.profile.as_ref().map_or(60, |p| p.daily_goal_min),
            today_min: days.get(&today).copied().unwrap_or(0.0),
            week,
            player: PlayerStatus {
                state: g.state,
                track_id: g.current.map(|i| g.tracks[i].id.clone()),
                title: g.current.map(|i| g.tracks[i].title.clone()),
                position_s: if music {
                    snap.pos_frames as f64 / snap.fs.max(1) as f64
                } else {
                    0.0
                },
                duration_s: g.current.map_or(0.0, |i| g.tracks[i].duration_s),
            },
            volume_db: g.volume_db(),
            library: g
                .tracks
                .iter()
                .map(|t| TrackView {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    duration_s: t.duration_s,
                    favorite: preferences.favorites.contains(&t.id),
                    recipe: if generation::allowed(g) {
                        t.recipe.clone()
                    } else {
                        None
                    },
                })
                .collect(),
            music_preferences: if generation::allowed(g) {
                preferences.music
            } else {
                MusicPreferences::default()
            },
            library_dir: g.store.library_dir(&g.settings).display().to_string(),
            clinician_unlocked: g.unlocked,
            pin_set: g.settings.pin_sha256.is_some(),
            device_fs: engine.device_fs().ok(),
            matching_active: g.matching.is_some(),
            error: g.error.clone().or_else(|| engine.error()),
            generation: g.generation.clone(),
            can_generate: generation::allowed(g),
        })
    })
    .await
}
