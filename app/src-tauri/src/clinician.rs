use super::*;
use crate::status::DayTotal;

// ---------- clinician ----------

#[tauri::command]
pub(super) async fn clinician_unlock(
    state: State<'_, AppState>,
    pin: String,
) -> Result<bool, String> {
    with_state(&state, move |g, _| {
        let ok = g.settings.pin_sha256.as_deref() == Some(store::hash_pin(&pin).as_str());
        g.unlocked = ok;
        Ok(ok)
    })
    .await
}

#[tauri::command]
pub(super) async fn clinician_set_pin(
    state: State<'_, AppState>,
    pin: String,
) -> Result<(), String> {
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
pub(super) async fn clinician_lock(state: State<'_, AppState>) -> Result<(), String> {
    with_state(&state, |g, _| {
        g.unlocked = false;
        Ok(())
    })
    .await
}

#[tauri::command]
pub(super) async fn profile_get(state: State<'_, AppState>) -> Result<Option<Profile>, String> {
    with_state(&state, |g, _| {
        g.require_unlocked()?;
        Ok(g.profile.clone())
    })
    .await
}

#[tauri::command]
pub(super) async fn profile_save(
    state: State<'_, AppState>,
    mut profile: Profile,
) -> Result<(), String> {
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
pub(super) async fn last_match_pitch(
    state: State<'_, AppState>,
) -> Result<Option<(f64, String)>, String> {
    with_state(&state, |g, _| {
        g.require_unlocked()?;
        Ok(g.last_match.as_ref().map(|(r, f)| (r.pitch_hz, f.clone())))
    })
    .await
}

#[derive(Serialize)]
pub(super) struct Logs {
    entries: Vec<LogEntry>,
    days: Vec<DayTotal>,
}

#[tauri::command]
pub(super) async fn logs_get(state: State<'_, AppState>) -> Result<Logs, String> {
    with_state(&state, |g, engine| {
        g.require_unlocked()?;
        let days = g
            .daily_minutes(engine)
            .into_iter()
            .rev()
            .map(|(d, m)| DayTotal {
                date: d.to_string(),
                minutes: m,
            })
            .collect();
        let mut entries = g.store.log_for(
            &g.profile
                .as_ref()
                .ok_or("No listener is configured.")?
                .participant_id,
        );
        entries.reverse();
        Ok(Logs { entries, days })
    })
    .await
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[tauri::command]
pub(super) async fn logs_export(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    with_state(&state, move |g, _| {
        g.require_unlocked()?;
        let entries = g.store.log_for(
            &g.profile
                .as_ref()
                .ok_or("No listener is configured.")?
                .participant_id,
        );
        let mut csv =
            String::from("participant_id,track,started_at,ended_at,seconds,volume_db,report\n");
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
pub(super) async fn data_dir(state: State<'_, AppState>) -> Result<String, String> {
    with_state(&state, |g, _| Ok(g.store.dir.display().to_string())).await
}
