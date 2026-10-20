import { useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api, errorText, fmtTime, type MusicRecipe, type Status } from "./api";

export default function MusicLibrary({ status, refresh, onReuse }: {
  status: Status; refresh: () => void; onReuse: (recipe: MusicRecipe) => void;
}) {
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState<Set<string>>(new Set());
  const [exporting, setExporting] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const tracks = status.library.filter(t =>
    t.title.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())
    && (filter !== "favorites" || t.favorite));
  const creating = ["composing", "waiting", "downloading", "verifying", "saving", "cancelling"].includes(status.generation.stage);
  return (
    <section className="card library">
      <div className="row between library-heading"><h2>Your library</h2><span className="muted">{status.library.length} {status.library.length === 1 ? "track" : "tracks"}</span></div>
      {status.library.length > 0 ? <>
        <div className="library-tools">
          <input type="search" aria-label="Search your library" placeholder="Find a track…" value={search} onChange={e => setSearch(e.target.value)} />
          <select aria-label="Show tracks" value={filter} onChange={e => setFilter(e.target.value)}>
            <option value="all">All tracks</option><option value="favorites">Favorites</option>
          </select>
        </div>
        <ol className="track-list">
          {tracks.map(track => {
            const selected = status.player.track_id === track.id;
            return <li className="track-row" key={track.id}>
              <button className={selected ? "track active" : "track"} aria-label={`Play ${track.title}`}
                aria-current={selected ? "true" : undefined} onClick={async () => {
                  setError(null);
                  try { await api.playTrack(track.id); } catch (e) { setError(errorText(e)); } finally { refresh(); }
                }}>
                <span className="track-title">{track.title}<small>{fmtTime(track.duration_s)}{track.recipe ? ` · ${track.recipe.provider === "ace_step" ? "AI music" : "Offline composition"}` : ""}</small></span>
                <span className="track-action">{selected && status.player.state === "playing" ? "Playing" : selected && status.player.state === "loading" ? "Checking…" : "Play"}</span>
              </button>
              <div className="track-tools">
                {track.recipe && status.can_generate && <button className="ghost" disabled={!!exporting || status.matching_active}
                  aria-label={`Save ${track.title} as WAV`} onClick={async () => {
                    setExporting(track.id); setError(null); setSavedPath(null);
                    try {
                      const safeName = track.title.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_");
                      const path = await save({ defaultPath: `${safeName}_listening.wav`, filters: [{ name: "WAV audio", extensions: ["wav"] }] });
                      if (!path) return;
                      const result = await api.exportTrack(status.participant_id!, track.id, path);
                      setSavedPath(result.path);
                    } catch (e) { setError(errorText(e)); }
                    finally { setExporting(null); }
                  }}>{exporting === track.id ? "Saving…" : "Save WAV…"}</button>}
                {track.recipe && status.can_generate && <button className="ghost" disabled={creating || status.matching_active}
                  aria-label={`Reuse settings for ${track.title}`} onClick={() => onReuse(track.recipe!)}>Reuse settings</button>}
                <button className="favorite-button" aria-label={`${track.favorite ? "Unfavorite" : "Favorite"} ${track.title}`}
                  aria-pressed={track.favorite} disabled={saving.has(track.id) || !status.participant_id}
                  onClick={async () => {
                    setError(null); setSaving(current => new Set(current).add(track.id));
                    try { await api.favorite(status.participant_id!, track.id, !track.favorite); }
                    catch (e) { setError(errorText(e)); }
                    finally { setSaving(current => { const next = new Set(current); next.delete(track.id); return next; }); refresh(); }
                  }}>{track.favorite ? "★" : "☆"}</button>
              </div>
            </li>;
          })}
        </ol>
        {tracks.length === 0 && <p className="muted">{search ? "No tracks match your search." : "Favorite a track to keep it here."}</p>}
      </> : <p className="muted">New music will appear here once its audio checks pass.</p>}
      {error && <p className="banner warn" role="alert">{error}</p>}
      {savedPath && <p className="banner ok export-result" role="status">Saved a checked listening copy to {savedPath}. Your source and creation settings remain in the library.</p>}
    </section>
  );
}
