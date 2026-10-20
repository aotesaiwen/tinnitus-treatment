use super::*;
use store::FeedbackRequest;

/// Eligibility reveals no recipe or clinical assignment to the participant.
pub(super) fn allowed(g: &Inner) -> bool {
    g.setup_done()
        && g.matching.is_none()
        && matches!(g.state, PlayState::Playing | PlayState::Paused)
        && g.current
            .and_then(|index| g.tracks.get(index))
            .is_some_and(|track| track.recipe.is_some())
}

#[tauri::command]
pub(super) async fn music_feedback_save(
    state: State<'_, AppState>,
    feedback: FeedbackRequest,
) -> Result<(), String> {
    with_state(&state, move |g, _| {
        let current = g.current.and_then(|index| g.tracks.get(index));
        let recipe = feedback.validate_for(
            g.profile.as_ref().map(|p| p.participant_id.as_str()),
            current.map(|track| track.id.as_str()),
            current.and_then(|track| track.recipe.as_ref()),
            allowed(g),
        )?;
        g.store
            .append_feedback(&feedback, recipe, &Local::now().to_rfc3339())?;
        Ok(())
    })
    .await
}
