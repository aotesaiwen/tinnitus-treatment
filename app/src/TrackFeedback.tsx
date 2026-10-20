import { useRef, useState, type FormEvent } from "react";
import { api, errorText } from "./api";

export default function TrackFeedback({
  participantId,
  trackId,
  title,
  refresh,
}: {
  participantId: string;
  trackId: string;
  title: string;
  refresh: () => void;
}) {
  const [rating, setRating] = useState("");
  const [discomfort, setDiscomfort] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!rating || inFlight.current || saved) return;
    inFlight.current = true;
    setSaving(true);
    setError(null);
    try {
      await api.saveTrackFeedback({
        participant_id: participantId,
        track_id: trackId,
        rating: Number(rating),
        discomfort: discomfort === "" ? null : discomfort === "yes",
      });
      setSaved(true);
    } catch (error) {
      setError(errorText(error));
    } finally {
      inFlight.current = false;
      setSaving(false);
      refresh();
    }
  };

  return (
    <section className="card" aria-labelledby="track-feedback-heading">
      <h2 id="track-feedback-heading">How was this music?</h2>
      <p>{title}</p>
      <p className="muted">Rate how much you liked the music. Your answer stays on this computer and does not change your listening settings.</p>
      <form className="form" onSubmit={submit}>
        <fieldset disabled={saving || saved}>
          <legend>Your listening experience</legend>
          <label htmlFor="music-rating">
            Music preference
            <select id="music-rating" required value={rating} onChange={event => setRating(event.target.value)}>
              <option value="">Choose a rating</option>
              <option value="1">1 — Strongly dislike</option>
              <option value="2">2 — Dislike</option>
              <option value="3">3 — Neutral</option>
              <option value="4">4 — Like</option>
              <option value="5">5 — Like very much</option>
            </select>
          </label>
          <label htmlFor="music-discomfort">
            Did this music feel uncomfortable? (optional)
            <select id="music-discomfort" value={discomfort} onChange={event => setDiscomfort(event.target.value)}>
              <option value="">Prefer not to answer</option>
              <option value="no">No</option>
              <option value="yes">Yes</option>
            </select>
          </label>
        </fieldset>
        <button type="submit" disabled={!rating || saving || saved}>
          {saving ? "Saving rating…" : saved ? "Rating saved" : "Save rating"}
        </button>
        {saved && <p role="status">Your rating was saved on this computer.</p>}
        {error && <p className="banner warn" role="alert">{error}</p>}
      </form>
    </section>
  );
}
