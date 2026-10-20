use super::*;

// ---------- music preferences ----------

#[tauri::command]
pub(super) async fn music_preferences_save(
    state: State<'_, AppState>,
    participant_id: String,
    preferences: MusicPreferences,
) -> Result<(), String> {
    preferences.validate()?;
    with_state(&state, move |g, _| {
        if !generation::allowed(g) {
            return Err("Your clinician manages music creation for this study.".into());
        }
        if g.profile.as_ref().map(|p| &p.participant_id) != Some(&participant_id) {
            return Err("The listener changed. Reopen the studio to save preferences.".into());
        }
        let mut updated = g.listener_preferences.clone();
        updated.entry(participant_id).or_default().music = preferences;
        g.store.save_listener_preferences(&updated)?;
        g.listener_preferences = updated;
        Ok(())
    })
    .await
}

#[tauri::command]
pub(super) async fn library_favorite(
    state: State<'_, AppState>,
    participant_id: String,
    track_id: String,
    favorite: bool,
) -> Result<(), String> {
    with_state(&state, move |g, _| {
        if g.profile.as_ref().map(|p| &p.participant_id) != Some(&participant_id) {
            return Err("The listener changed. Refresh the library and try again.".into());
        }
        if !g.tracks.iter().any(|t| t.id == track_id) {
            return Err("This track is no longer in the library.".into());
        }
        let mut updated = g.listener_preferences.clone();
        let favorites = &mut updated.entry(participant_id).or_default().favorites;
        if favorite {
            favorites.insert(track_id);
        } else {
            favorites.remove(&track_id);
        }
        g.store.save_listener_preferences(&updated)?;
        g.listener_preferences = updated;
        Ok(())
    })
    .await
}

#[derive(Serialize)]
pub(super) struct ExportResult {
    path: String,
    report_file: String,
}

#[tauri::command]
pub(super) async fn library_export(
    state: State<'_, AppState>,
    participant_id: String,
    track_id: String,
    path: String,
) -> Result<ExportResult, String> {
    with_shared(&state, move |inner, _| {
        let (source, destination, opts, profile) = {
            let g = lock(inner);
            if !generation::allowed(&g) {
                return Err("Your clinician manages music export for this study.".into());
            }
            if g.matching.is_some() {
                return Err("Finish matching before exporting music.".into());
            }
            if g.profile.as_ref().map(|p| &p.participant_id) != Some(&participant_id) {
                return Err("The listener changed. Refresh the library and try again.".into());
            }
            let track = g
                .tracks
                .iter()
                .find(|t| t.id == track_id && t.recipe.is_some())
                .ok_or("This generated track is no longer available.")?;
            let mut protected: Vec<PathBuf> = g.tracks.iter().map(|t| t.path.clone()).collect();
            protected.push(g.store.library_dir(&g.settings));
            protected.push(g.store.dir.join("generated/library"));
            (
                track.path.clone(),
                export::destination(Path::new(&path), &protected)?,
                g.options()?,
                serde_json::to_value(&g.profile).map_err(|e| e.to_string())?,
            )
        };
        let (bytes, mut report) = export::prepare(&source, &opts)?;
        let g = lock(inner);
        if !generation::allowed(&g)
            || serde_json::to_value(&g.profile).map_err(|e| e.to_string())? != profile
        {
            return Err(
                "Your listening settings changed. Export again using the new settings.".into(),
            );
        }
        let mut protected: Vec<PathBuf> = g.tracks.iter().map(|t| t.path.clone()).collect();
        protected.push(g.store.library_dir(&g.settings));
        protected.push(g.store.dir.join("generated/library"));
        let destination = export::destination(&destination, &protected)?;
        report.output_path = Some(destination.display().to_string());
        let record_name = format!("export_{}", Local::now().format("%Y%m%dT%H%M%S%f"));
        let report_file = g.store.save_record("reports", &record_name, &report)?;
        if let Err(error) = store::atomic_write(&destination, &bytes) {
            let _ = std::fs::remove_file(g.store.dir.join("reports").join(&report_file));
            return Err(error);
        }
        Ok(ExportResult {
            path: destination.display().to_string(),
            report_file,
        })
    })
    .await
}

#[tauri::command]
pub(super) async fn library_set_dir(state: State<'_, AppState>, dir: String) -> Result<(), String> {
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
pub(super) struct VerifyRow {
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
pub(super) async fn library_verify(state: State<'_, AppState>) -> Result<Vec<VerifyRow>, String> {
    let (inner, _) = (state.inner.clone(), state.engine.clone());
    let (tracks, opts) = tauri::async_runtime::spawn_blocking(move || {
        let g = lock(&inner);
        g.require_unlocked()?;
        let tracks: Vec<(PathBuf, String, String, f64)> = g
            .tracks
            .iter()
            .map(|t| {
                (
                    t.path.clone(),
                    t.title.clone(),
                    t.file.clone(),
                    t.duration_s,
                )
            })
            .collect();
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
                    attenuation_db: r
                        .measurement
                        .as_ref()
                        .and_then(|m| m.bands.first())
                        .map(|b| b.attenuation_db),
                    input_lufs: r.input.integrated_lufs,
                    failed_checks: r
                        .checks
                        .iter()
                        .filter(|c| !c.pass)
                        .map(|c| format!("{} {}", c.id, c.description))
                        .collect(),
                    warnings: r.warnings,
                })
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}
