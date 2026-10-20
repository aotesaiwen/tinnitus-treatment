import { useState } from "react";
import { api, errorText, fmtTime, type MusicRecipe, type Status } from "./api";
import MusicStudio from "./MusicStudio";
import MusicLibrary from "./MusicLibrary";
import TrackFeedback from "./TrackFeedback";

function Ring({ value, goal }: { value: number; goal: number }) {
  const frac = Math.min(1, goal > 0 ? value / goal : 0);
  const r = 54;
  const c = 2 * Math.PI * r;
  return (
    <svg viewBox="0 0 140 140" className="ring" role="img" aria-label={`${Math.round(value)} of ${goal} minutes`}>
      <circle cx="70" cy="70" r={r} className="ring-track" />
      {frac > 0 && (
        <circle
          cx="70"
          cy="70"
          r={r}
          className="ring-fill"
          strokeDasharray={`${c * frac} ${c}`}
          transform="rotate(-90 70 70)"
        />
      )}
      <text x="70" y="66" textAnchor="middle" className="ring-big">
        {Math.floor(value)}
      </text>
      <text x="70" y="88" textAnchor="middle" className="ring-small">
        of {goal} min
      </text>
    </svg>
  );
}

export default function Participant({
  status,
  onSetup,
  refresh,
}: {
  status: Status;
  onSetup: () => void;
  refresh: () => void;
}) {
  const [reuse, setReuse] = useState<{ recipe: MusicRecipe; revision: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const run = async (f: () => Promise<unknown>) => {
    setErr(null);
    try {
      await f();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      refresh();
    }
  };

  if (!status.setup_done) {
    return (
      <section className="card narrow">
        <h1>Your music. Your listening routine.</h1>
        <p>Create fresh instrumentals with the offline composer or describe your sound to AI. Each track is checked before playback.</p>
        <p>
          This app plays relaxing music shaped to your tinnitus. First it needs to know the pitch of your tinnitus. That
          takes about 15–20 minutes of short listening questions.
        </p>
        <p className="muted">If you are taking part in a study, your clinician will set this up for you.</p>
        <button className="primary" onClick={onSetup}>
          Find my tinnitus pitch
        </button>
      </section>
    );
  }

  const p = status.player;
  const max = Math.max(...status.week.map((d) => d.minutes), status.daily_goal_min, 1);
  const pct = p.duration_s > 0 ? (100 * p.position_s) / p.duration_s : 0;

  return (
    <div className="participant">
      <MusicStudio key={`${status.participant_id}-${reuse?.revision ?? 0}`} status={status} refresh={refresh} recipe={reuse?.recipe} />
      <div className="grid">
      <section className="card today">
        <h2>Today</h2>
        <Ring value={status.today_min} goal={status.daily_goal_min} />
        <p className="muted center">
          {status.today_min >= status.daily_goal_min ? "Daily goal reached. Well done." : "Listening time counts while music plays."}
        </p>
      </section>

      <section className="card player">
        <h2>Music</h2>
        {status.library.length === 0 ? (
          <p className="muted">{status.can_generate ? "Your library starts here. Create your first track in the studio above." : "No music has been added yet. Your clinician can create or add music for you."}</p>
        ) : (
          <>
            <div className="now">
              <div className="title">{p.state === "loading" ? `Preparing ${p.title ?? "track"}…` : (p.title ?? "Press play to start")}</div>
              <div className="sub">
                {p.state === "idle" || p.state === "loading" ? `${status.library.length} ${status.library.length === 1 ? "track" : "tracks"}` : `${fmtTime(p.position_s)} / ${fmtTime(p.duration_s)}`}
              </div>
            </div>
            <div className="progress" aria-hidden>
              <div style={{ width: `${pct}%` }} />
            </div>
            <div className="controls">
              <button className="round" onClick={() => run(() => api.skip(-1))} aria-label="Previous">
                ⏮︎
              </button>
              <button className="round big" onClick={() => run(api.toggle)} aria-label={p.state === "playing" ? "Pause" : p.state === "loading" ? "Cancel loading" : "Play"}>
                {p.state === "playing" ? "❚❚" : p.state === "loading" ? "…" : "▶"}
              </button>
              <button className="round" onClick={() => run(() => api.skip(1))} aria-label="Next">
                ⏭︎
              </button>
            </div>
            <label className="volume">
              <span>Volume</span>
              <input
                type="range"
                min={-40}
                max={0}
                step={1}
                value={status.volume_db}
                onChange={(e) => run(() => api.setVolume(Number(e.target.value)))}
              />
              <span className="vol-db">{status.volume_db <= -40 ? "muted" : `${status.volume_db} dB`}</span>
            </label>
            {status.volume_db <= -40 && p.state === "playing" && (
              <p className="muted">Muted: time does not count towards today.</p>
            )}
          </>
        )}
      </section>

      {p.can_rate && status.participant_id && p.track_id && (
        <TrackFeedback
          key={`${status.participant_id}-${p.track_id}`}
          participantId={status.participant_id}
          trackId={p.track_id}
          title={p.title ?? "Current track"}
          refresh={refresh}
        />
      )}

      <MusicLibrary key={status.participant_id} status={status} refresh={refresh} onReuse={recipe => {
        setReuse(current => ({ recipe, revision: (current?.revision ?? 0) + 1 }));
        requestAnimationFrame(() => document.getElementById("music-engine")?.focus());
      }} />
      {err && <div className="banner warn" role="alert">{err}</div>}

      <section className="card week">
        <h2>This week</h2>
        <div className="bars">
          {status.week.map((d) => {
            const day = new Date(d.date + "T12:00:00");
            return (
              <div className="bar" key={d.date} title={`${Math.round(d.minutes)} min`}>
                <div className="bar-col">
                  <div
                    className={d.minutes >= status.daily_goal_min ? "fill goal" : "fill"}
                    style={{ height: `${(100 * d.minutes) / max}%` }}
                  />
                </div>
                <span>{day.toLocaleDateString(undefined, { weekday: "short" })}</span>
              </div>
            );
          })}
        </div>
      </section>

      <section className="card tip">
        <h2>Listening tips</h2>
        <ul>
          <li>Listen at a comfortable, moderate level. It should never feel loud.</li>
          <li>Use headphones or speakers. Listening while relaxing or working is fine.</li>
          <li>If any sound feels unpleasant, stop and tell your clinician.</li>
        </ul>
      </section>
      </div>
    </div>
  );
}
