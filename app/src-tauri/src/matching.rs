use super::*;

// ---------- matching ----------

#[derive(Deserialize)]
pub(super) struct MatchStart {
    ear: Ear,
    sound: TinnitusSound,
    repeats: usize,
}

#[derive(Serialize)]
pub(super) struct AnswerOption {
    answer: Answer,
    label: &'static str,
}

#[derive(Serialize)]
pub(super) struct MatchSummary {
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
pub(super) enum MatchView {
    Prompt {
        stage: String,
        step: usize,
        steps: usize,
        question: &'static str,
        options: Vec<AnswerOption>,
        trial: usize,
    },
    Done {
        summary: MatchSummary,
    },
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
        Stage::Pitch { run } => (
            format!(
                "Finding your tinnitus pitch (round {} of {repeats})",
                run + 1
            ),
            3,
        ),
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
            options: p
                .question
                .options()
                .iter()
                .map(|&a| AnswerOption {
                    answer: a,
                    label: answer_label(a),
                })
                .collect(),
            trial: m.session.log().len() + 1,
        });
    }
    let m = g.matching.take().expect("checked above");
    let r = m
        .session
        .report()
        .ok_or("matching ended without a result")?;
    let file = g.store.save_record(
        "matching",
        &Local::now().format("%Y%m%d-%H%M%S").to_string(),
        &r,
    )?;
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
pub(super) async fn match_start(
    state: State<'_, AppState>,
    opts: MatchStart,
) -> Result<MatchView, String> {
    with_state(&state, move |g, engine| {
        let self_setup = !g.unlocked;
        if self_setup && g.setup_done() {
            return Err(
                "this computer is already set up; ask your clinician to repeat matching".into(),
            );
        }
        g.stop(engine);
        let fs = engine.device_fs()?;
        let mut cfg = SessionConfig::new(fs as f64, Calibration::default());
        cfg.ear = opts.ear;
        cfg.tinnitus_sound = opts.sound;
        cfg.pitch_repeats = opts.repeats.clamp(1, 5);
        cfg.seed = Local::now().timestamp_nanos_opt().unwrap_or(1) as u64;
        let renderer = Renderer::new(
            cfg.fs,
            cfg.calibration.clone(),
            cfg.max_level_db,
            cfg.seed ^ 0xA5A5,
        );
        let session = Session::new(cfg)?;
        g.matching = Some(Matching {
            session,
            renderer,
            self_setup,
        });
        match_view(g)
    })
    .await
}

/// Plays the current prompt's sounds and returns when they have finished.
#[tauri::command]
pub(super) async fn match_play(state: State<'_, AppState>) -> Result<(), String> {
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
pub(super) async fn match_answer(
    state: State<'_, AppState>,
    answer: Answer,
) -> Result<MatchView, String> {
    with_state(&state, move |g, _| {
        g.matching
            .as_mut()
            .ok_or("matching is not running")?
            .session
            .answer(answer)?;
        match_view(g)
    })
    .await
}

#[tauri::command]
pub(super) async fn match_cancel(state: State<'_, AppState>) -> Result<(), String> {
    with_state(&state, |g, engine| {
        g.matching = None;
        engine.stop();
        Ok(())
    })
    .await
}
