use super::*;

// ---------- player ----------

#[tauri::command]
pub(super) async fn player_play(
    state: State<'_, AppState>,
    track_id: Option<String>,
) -> Result<(), String> {
    with_shared(&state, move |inner, engine| {
        let selection = track_id.map_or(Selection::Current, Selection::Id);
        play_track(inner, engine, selection)
    })
    .await
}

#[tauri::command]
pub(super) async fn player_toggle(state: State<'_, AppState>) -> Result<(), String> {
    with_shared(&state, |inner, engine| {
        let start_at = {
            let mut g = lock(inner);
            match g.state {
                PlayState::Playing => {
                    g.close_segment(engine);
                    engine.set_playing(false);
                    g.state = PlayState::Paused;
                    None
                }
                PlayState::Paused => {
                    engine.set_playing(true);
                    g.state = PlayState::Playing;
                    g.open_segment(engine);
                    None
                }
                // Pressing the button again while a track loads cancels it.
                PlayState::Loading => {
                    g.stop(engine);
                    None
                }
                PlayState::Idle => Some(g.play_gen),
            }
        };
        match start_at {
            Some(revision) => play_track(inner, engine, Selection::IfIdle(revision)),
            None => Ok(()),
        }
    })
    .await
}

#[tauri::command]
pub(super) async fn player_skip(state: State<'_, AppState>, delta: i32) -> Result<(), String> {
    with_shared(&state, move |inner, engine| {
        play_track(inner, engine, Selection::Offset(delta))
    })
    .await
}

#[tauri::command]
pub(super) async fn player_stop(state: State<'_, AppState>) -> Result<(), String> {
    with_state(&state, |g, engine| {
        g.stop(engine);
        Ok(())
    })
    .await
}

#[tauri::command]
pub(super) async fn set_volume(state: State<'_, AppState>, db: f32) -> Result<(), String> {
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
