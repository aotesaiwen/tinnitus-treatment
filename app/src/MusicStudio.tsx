import { useState } from "react";
import { api, errorText, type MusicProvider, type MusicStyle, type MusicRecipe, type Status } from "./api";

const styles: { id: MusicStyle; name: string; description: string; tempo: number }[] = [
  { id: "drift", name: "Slow drift", description: "Warm pads · spacious harmony", tempo: 54 },
  { id: "focus", name: "Gentle focus", description: "Soft keys · a steady pulse", tempo: 76 },
  { id: "unwind", name: "Evening unwind", description: "Plucked tones · room to breathe", tempo: 60 },
];

const stages: Record<string, string> = {
  composing: "Creating your music",
  waiting: "ACE-Step is composing",
  downloading: "Receiving your track",
  verifying: "Checking audio quality",
  saving: "Adding to your library",
  cancelling: "Cancelling",
};

export default function MusicStudio({ status, refresh, recipe }: { status: Status; refresh: () => void; recipe?: MusicRecipe }) {
  const initial = recipe ? { ...recipe.request, provider: recipe.provider } : status.music_preferences;
  const [provider, setProvider] = useState<MusicProvider>(initial.provider);
  const [style, setStyle] = useState<MusicStyle>(initial.style);
  const [duration, setDuration] = useState(initial.duration_s);
  const [tempo, setTempo] = useState(initial.tempo_bpm);
  const [prompt, setPrompt] = useState(initial.prompt);
  const [seed, setSeed] = useState(recipe ? String(recipe.request.seed) : "");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [playingNew, setPlayingNew] = useState(false);
  const [checking, setChecking] = useState(false);
  const [connection, setConnection] = useState<{ ok: boolean; message: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const job = status.generation;
  const busy = submitting || job.stage in stages;
  const available = status.can_generate && status.setup_done && !status.matching_active;

  async function remember() {
    if (!status.participant_id) throw new Error("Complete your listening setup first.");
    await api.saveMusicPreferences(status.participant_id, { provider, style, duration_s: duration, tempo_bpm: tempo, prompt });
  }

  async function generate(newVariation = false) {
    setError(null);
    const chosenSeed = newVariation || seed === "" ? crypto.getRandomValues(new Uint32Array(1))[0] : Number(seed);
    if (!Number.isInteger(chosenSeed) || chosenSeed < 0 || chosenSeed > 4294967295) {
      setError("Use a whole-number seed between 0 and 4294967295, or leave it blank.");
      return;
    }
    if (newVariation) setSeed("");
    setSubmitting(true);
    try {
      await remember();
      await api.generate(provider, { style, duration_s: duration, tempo_bpm: tempo, seed: chosenSeed, prompt: provider === "ace_step" ? prompt : "" });
      refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSubmitting(false);
    }
  }

  async function checkService() {
    setChecking(true);
    try {
      await api.generationHealth();
      setConnection({ ok: true, message: "ACE-Step is reachable. Ready to request music." });
    } catch (e) {
      setConnection({ ok: false, message: errorText(e) });
    } finally {
      setChecking(false);
    }
  }

  return (
    <section className="studio" aria-labelledby="studio-title">
      <div className="studio-intro">
        <div className="studio-kicker">YOUR MUSIC STUDIO</div>
        <h1 id="studio-title">Music, made for your day.</h1>
        <p>Compose offline or create with AI. Every track is checked against your listening settings before it joins your library.</p>
        <div className="studio-process" aria-label="Creation workflow">
          <span>Choose a mood</span><span>Create</span><span>Check & listen</span>
        </div>
        <p className="studio-note">
          {provider === "composer"
            ? "The offline composer creates music on this computer. No account or GPU needed."
            : "ACE-Step turns your description into music. Only music preferences are sent to the configured service."}
        </p>
      </div>
      <div className="studio-workspace" onChange={() => setSaved(false)}>
        {recipe && <p className="recipe-notice" role="status">Creation settings restored. Keep the seed to reuse this variation, or clear it to create another.</p>}
        <div className="studio-engine">
          <label htmlFor="music-engine">Create with</label>
          <select id="music-engine" value={provider} disabled={busy} onChange={(e) => setProvider(e.target.value as MusicProvider)}>
            <option value="composer">Offline composer</option>
            <option value="ace_step">AI music · ACE-Step</option>
          </select>
        </div>
        <fieldset className="mood-picker" disabled={busy || !available}>
          <legend>Set the mood</legend>
          {styles.map((item) => (
            <label className={`mood-option ${style === item.id ? "selected" : ""}`} key={item.id}>
              <input type="radio" name="music-style" value={item.id} checked={style === item.id}
                onChange={() => { setStyle(item.id); setTempo(item.tempo); }} />
              <span><strong>{item.name}</strong><small>{item.description}</small></span>
            </label>
          ))}
        </fieldset>
        {provider === "ace_step" && (
          <div className="ai-description">
            <label htmlFor="music-description">Describe your sound <span className="muted">(optional)</span></label>
            <textarea id="music-description" rows={2} maxLength={500} value={prompt} disabled={busy || !available}
              onChange={(e) => setPrompt(e.target.value)} placeholder="Soft piano beside a rainy window, a warm and unhurried melody…" />
            <div className="service-check">
              <button className="ghost" disabled={checking || busy} onClick={checkService}>{checking ? "Checking…" : "Check AI connection"}</button>
              <span className="muted">Requires a running ACE-Step service.</span>
            </div>
            {connection && <p className={connection.ok ? "pass" : "warnings"} role="status">{connection.message}</p>}
          </div>
        )}
        <div className="music-settings">
          <label>Track length
            <select value={duration} disabled={busy || !available} onChange={(e) => setDuration(Number(e.target.value))}>
              <option value={30}>30 seconds</option><option value={60}>1 minute</option>
              <option value={120}>2 minutes</option><option value={180}>3 minutes</option><option value={300}>5 minutes</option>
            </select>
          </label>
          <label htmlFor="music-tempo">Tempo <span>{tempo} BPM</span>
            <input id="music-tempo" type="range" min={45} max={100} value={tempo} disabled={busy || !available} onChange={(e) => setTempo(Number(e.target.value))} />
          </label>
        </div>
        <details className="variation-settings" open={recipe ? true : undefined}>
          <summary>Recreate a variation</summary>
          <label htmlFor="music-seed">Seed <span className="muted">(blank creates a new variation)</span></label>
          <input id="music-seed" type="number" min={0} max={4294967295} step={1} value={seed} disabled={busy || !available} onChange={(e) => setSeed(e.target.value)} />
        </details>
        <div className="remember-settings">
          <button className="ghost" disabled={busy || saving || !available} onClick={async () => {
            setSaving(true); setError(null);
            try { await remember(); setSaved(true); refresh(); } catch (e) { setError(errorText(e)); }
            finally { setSaving(false); }
          }}>{saving ? "Saving…" : "Remember settings"}</button>
          <span className="muted" role="status">{saved ? "Saved on this computer." : "Your settings are also saved when you create."}</span>
        </div>
        <div className="generate-actions">
          <button className="primary generate-button" disabled={busy || !available} onClick={() => generate()}>
            {busy ? (stages[job.stage] ?? "Starting…") : provider === "composer" ? "Compose my music" : "Generate AI music"}
          </button>
          {busy && <button disabled={job.stage === "cancelling" || submitting || !status.can_generate} onClick={async () => {
            try { await api.generationCancel(); refresh(); } catch (e) { setError(errorText(e)); }
          }}>Cancel</button>}
        </div>
        {!busy && job.stage === "failed" && available && <button className="ghost" onClick={() => generate(true)}>Try a new variation</button>}
        {!available && <p className="muted">{!status.setup_done ? "Complete your listening setup to create music." : status.matching_active ? "Finish matching before creating music." : "Your clinician manages music creation for this study."}</p>}
        <div aria-live="polite" aria-atomic="true">
          {busy && <p className="job-status"><span className="activity-indicator" aria-hidden />{stages[job.stage] ?? "Starting your track"} · You can keep listening.</p>}
          {!busy && job.message && <p className={`banner ${job.stage === "ready" ? "ok" : "warn"}`}>{job.message}</p>}
        </div>
        {job.stage === "ready" && job.track_id && available && <button className="primary" disabled={playingNew} onClick={async () => {
          setPlayingNew(true); setError(null);
          try { await api.playTrack(job.track_id!); } catch (e) { setError(errorText(e)); }
          finally { setPlayingNew(false); refresh(); }
        }}>{playingNew ? "Preparing your track…" : "Play new track"}</button>}
        {error && <p className="banner warn" role="alert">{error}</p>}
      </div>
    </section>
  );
}
