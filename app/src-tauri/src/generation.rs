//! Background generation jobs. Only a verified source is published to the library.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tauri::State;
use tmdsp::generation::{self, Provider, Request, acestep::AceStep};
use tmdsp::{pipeline, wav};

use crate::{AppState, Inner, lock};

#[derive(Clone, Default, Serialize)]
pub struct Job {
    pub stage: String,
    pub title: Option<String>,
    pub track_id: Option<String>,
    pub message: Option<String>,
    #[serde(skip)]
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Job {
    pub fn busy(&self) -> bool {
        self.cancel.is_some()
    }
}

pub fn allowed(inner: &Inner) -> bool {
    inner.unlocked
        || inner
            .profile
            .as_ref()
            .is_some_and(|p| p.participant_id == "self")
}

fn service() -> Result<AceStep, String> {
    AceStep::new(
        &std::env::var("TINNITUS_MUSIC_ACESTEP_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8001".into()),
        std::env::var("ACESTEP_API_KEY")
            .ok()
            .filter(|key| !key.is_empty()),
    )
}

#[tauri::command]
pub async fn generation_health() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| service()?.health())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn generation_cancel(state: State<'_, AppState>) -> Result<(), String> {
    let mut inner = lock(&state.inner);
    if !allowed(&inner) {
        return Err("Your clinician manages music generation for this study.".into());
    }
    if let Some(cancel) = &inner.generation.cancel {
        cancel.store(true, Ordering::Relaxed);
        inner.generation.stage = "cancelling".into();
    }
    Ok(())
}

#[tauri::command]
pub async fn generation_start(
    state: State<'_, AppState>,
    provider: Provider,
    request: Request,
) -> Result<(), String> {
    request.validate()?;
    if matches!(provider, Provider::Composer) && !request.prompt.trim().is_empty() {
        return Err("Text descriptions are available with ACE-Step. The offline composer uses style, tempo and seed.".into());
    }
    let (dir, opts, profile, cancel) = {
        let mut inner = lock(&state.inner);
        if !allowed(&inner) {
            return Err("Your clinician manages music generation for this study.".into());
        }
        if inner.matching.is_some() {
            return Err("Finish matching before generating music.".into());
        }
        if inner.generation.busy() {
            return Err("A music generation is already in progress.".into());
        }
        let opts = inner.options()?;
        let profile = serde_json::to_value(&inner.profile).map_err(|e| e.to_string())?;
        let cancel = Arc::new(AtomicBool::new(false));
        inner.generation = Job {
            stage: "composing".into(),
            title: Some(request.style.title().into()),
            track_id: None,
            message: None,
            cancel: Some(cancel.clone()),
        };
        (inner.store.dir.clone(), opts, profile, cancel)
    };
    let shared = state.inner.clone();
    let worker = shared.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = tauri::async_runtime::spawn_blocking(move || -> Result<(String, String), String> {
            let cancelled = || cancel.load(Ordering::Relaxed);
            let stage = |value: &str| {
                let mut inner = lock(&worker);
                if !cancelled() {
                    inner.generation.stage = value.into();
                }
            };
            let (audio, model) = match provider {
                Provider::Composer => (generation::compose(&request, cancelled)?, json!({"engine": "additive-composer-v1"})),
                Provider::AceStep => service()?.generate(&request, cancelled, stage)?,
            };
            if cancelled() { return Err("Generation cancelled.".into()); }
            stage("verifying");
            let (_, mut report) = pipeline::run(&audio, &opts).map_err(|e| format!("Verification failed: {e:#}"))?;
            if cancelled() { return Err("Generation cancelled.".into()); }
            // Reject warnings too: a technically correct notch in a source with
            // no useful energy in its band is not enough for library admission.
            if !report.all_pass || !report.warnings.is_empty() {
                let sparse_source = report.warnings.iter().any(|warning|
                    warning.contains("little energy") || warning.contains("band-limited"));
                let rejected = dir.join("generated").join("rejected");
                std::fs::create_dir_all(&rejected).map_err(|e| e.to_string())?;
                let record = json!({
                    "schema_version": 1, "provider": provider, "request": request, "model": model,
                    "profile_at_generation": profile, "verification": report,
                });
                std::fs::write(
                    rejected.join(format!("{}.json", chrono::Local::now().format("%Y%m%dT%H%M%S%f"))),
                    serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?,
                ).map_err(|e| e.to_string())?;
                return Err(if sparse_source {
                    "This track has too little sound in the frequency range needed by your listening settings. Try Gentle focus or Evening unwind, or create a new variation."
                } else {
                    "This track did not pass the audio quality checks for your settings. Try a new variation or a different style."
                }.into());
            }
            stage("saving");
            let id = format!("{}-{}", chrono::Local::now().format("%Y%m%dT%H%M%S%f"), request.seed);
            let stem = format!("{}_{id}", request.style.title().to_lowercase().replace(' ', "_"));
            let source_dir = dir.join("generated").join("library");
            let records = dir.join("generated").join("records");
            std::fs::create_dir_all(&source_dir).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&records).map_err(|e| e.to_string())?;
            let pending = source_dir.join(format!("{stem}.pending"));
            let source = source_dir.join(format!("{stem}.wav"));
            let record = records.join(format!("{stem}.json"));
            let (bytes, _) = wav::encode_wav(&audio, wav::SampleFormat::F32, request.seed as u64);
            let hash = format!("{:x}", Sha256::digest(&bytes));
            report.input_sha256 = Some(hash.clone());
            report.input_path = Some(source.display().to_string());
            let metadata = json!({
                "schema_version": 1, "provider": provider, "request": request, "model": model,
                "created_at": chrono::Local::now().to_rfc3339(), "source_sha256": hash,
                "profile_at_generation": profile, "verification": report,
            });
            std::fs::write(&pending, bytes).map_err(|e| format!("Could not save generated music: {e}"))?;
            let publish = (|| {
                let mut inner = lock(&worker);
                if cancelled() {
                    return Err("Generation cancelled.".into());
                }
                if serde_json::to_value(&inner.profile).map_err(|e| e.to_string())? != profile {
                    return Err("Your listening settings changed. Generate again using the new settings.".into());
                }
                // The manifest is complete before the WAV becomes visible.
                std::fs::write(&record, serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                std::fs::rename(&pending, &source).map_err(|e| e.to_string())?;
                inner.scan_library();
                Ok((format!("{} · {}", request.style.title(), request.seed), crate::store::track_id(&source)))
            })();
            if publish.is_err() {
                let _ = std::fs::remove_file(pending);
                let _ = std::fs::remove_file(record);
            }
            publish
        }).await;
        let mut inner = lock(&shared);
        let cancelled = inner
            .generation
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed));
        inner.generation.cancel = None;
        match outcome.unwrap_or_else(|_| {
            Err("Music generation stopped unexpectedly. Please try again.".into())
        }) {
            Ok((title, track_id)) => {
                inner.generation.stage = "ready".into();
                inner.generation.title = Some(title);
                inner.generation.track_id = Some(track_id);
                inner.generation.message = Some(
                    "Added to your library. Select the track when you are ready to listen.".into(),
                );
            }
            Err(message) => {
                inner.generation.stage = if cancelled { "cancelled" } else { "failed" }.into();
                inner.generation.message = Some(message);
            }
        }
    });
    Ok(())
}
