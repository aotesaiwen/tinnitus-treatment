import { useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorText, fmtTime, type DayTotal, type LogEntry, type MatchSummary, type Profile, type Status, type VerifyRow } from "./api";

type Tab = "participant" | "library" | "log" | "security";

const EMPTY: Profile = {
  participant_id: "",
  arm: "notch",
  pitch_hz: null,
  sham_hz: null,
  width_oct: 1,
  daily_goal_min: 60,
  matching_report: null,
  updated_at: "",
};

function num(v: string): number | null {
  const n = Number(v);
  return v.trim() === "" || !Number.isFinite(n) ? null : n;
}

function ParticipantTab({ lastMatch, onRunMatching }: { lastMatch: MatchSummary | null; onRunMatching: () => void }) {
  const [p, setP] = useState<Profile>(EMPTY);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    api.profileGet().then((x) => x && setP(x));
  }, []);

  const useLast = async () => {
    const last = await api.lastMatchPitch();
    if (last) setP({ ...p, pitch_hz: Math.round(last[0]), matching_report: last[1] });
  };

  const saveProfile = async () => {
    try {
      await api.profileSave(p);
      setMsg({ ok: true, text: "Saved. The participant view now uses these settings." });
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) });
    }
  };

  return (
    <div className="form">
      <p className="muted">
        One participant per computer. Participants never see the arm: all arms look and sound the same apart from the
        processing, and all are normalised to the same loudness.
      </p>
      <label>
        Participant ID
        <input value={p.participant_id} onChange={(e) => setP({ ...p, participant_id: e.target.value })} placeholder="e.g. P012" />
      </label>
      <label>
        Arm
        <select value={p.arm} onChange={(e) => setP({ ...p, arm: e.target.value as Profile["arm"] })}>
          <option value="notch">Notch at tinnitus pitch (treatment)</option>
          <option value="passthrough">Unprocessed music (active control)</option>
          <option value="sham">Notch elsewhere (sham control)</option>
        </select>
      </label>
      <div className="row">
        <label>
          Tinnitus pitch (Hz)
          <input value={p.pitch_hz ?? ""} onChange={(e) => setP({ ...p, pitch_hz: num(e.target.value) })} />
        </label>
        <button className="ghost" onClick={onRunMatching}>
          Run matching
        </button>
        {lastMatch && (
          <button className="ghost" onClick={useLast}>
            Use last result ({Math.round(lastMatch.pitch_hz)} Hz)
          </button>
        )}
      </div>
      {p.arm === "sham" && (
        <label>
          Sham notch centre (Hz), well away from the tinnitus band
          <input value={p.sham_hz ?? ""} onChange={(e) => setP({ ...p, sham_hz: num(e.target.value) })} />
        </label>
      )}
      <div className="row">
        <label>
          Notch width (octaves)
          <input value={p.width_oct} onChange={(e) => setP({ ...p, width_oct: num(e.target.value) ?? 1 })} />
        </label>
        <label>
          Daily listening goal (minutes)
          <input value={p.daily_goal_min} onChange={(e) => setP({ ...p, daily_goal_min: Math.round(num(e.target.value) ?? 60) })} />
        </label>
      </div>
      {p.matching_report && <p className="muted">Matching report: {p.matching_report}</p>}
      {msg && <div className={msg.ok ? "banner ok" : "banner warn"}>{msg.text}</div>}
      <div className="row end">
        <button className="primary" onClick={saveProfile}>
          Save participant
        </button>
      </div>
    </div>
  );
}

function LibraryTab({ status, refresh }: { status: Status; refresh: () => void }) {
  const [dir, setDir] = useState(status.library_dir);
  const [rows, setRows] = useState<VerifyRow[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const browse = async () => {
    const picked = await open({ directory: true, defaultPath: dir });
    if (typeof picked === "string") setDir(picked);
  };
  const apply = async () => {
    setErr(null);
    try {
      await api.librarySetDir(dir);
      setRows(null);
      refresh();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const verify = async () => {
    setBusy(true);
    setErr(null);
    try {
      setRows(await api.libraryVerify());
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="form">
      <p className="muted">
        A folder of WAV files. Files already processed by <code>tmusic process</code> (names containing “_notch”) are
        ignored; the app processes each track itself when it plays, and saves a verification report every time.
      </p>
      <div className="row">
        <input className="grow" value={dir} onChange={(e) => setDir(e.target.value)} />
        <button className="ghost" onClick={browse}>
          Browse…
        </button>
        <button className="primary" onClick={apply}>
          Use folder
        </button>
      </div>
      <p>
        {status.tracks.length} tracks.{" "}
        <button className="ghost" disabled={busy || status.tracks.length === 0} onClick={verify}>
          {busy ? "Verifying…" : "Verify all with this participant's settings"}
        </button>
      </p>
      {err && <div className="banner warn">{err}</div>}
      {rows && (
        <table className="table">
          <thead>
            <tr>
              <th>Track</th>
              <th>Length</th>
              <th>Input LUFS</th>
              <th>Notch depth</th>
              <th>Checks</th>
              <th>Warnings</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.file}>
                <td title={r.file}>{r.title}</td>
                <td>{fmtTime(r.duration_s)}</td>
                <td>{r.input_lufs.toFixed(1)}</td>
                <td>{r.attenuation_db === null ? "—" : `${r.attenuation_db.toFixed(1)} dB`}</td>
                <td className={r.all_pass ? "pass" : "fail"} title={r.failed_checks.join("\n")}>
                  {r.all_pass ? "PASS" : "FAIL"}
                </td>
                <td>{r.warnings.length === 0 ? "—" : r.warnings.join("; ")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function LogTab() {
  const [data, setData] = useState<{ entries: LogEntry[]; days: DayTotal[] } | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    api.logsGet().then(setData);
  }, []);

  const exportCsv = async () => {
    const path = await save({ defaultPath: "listening_log.csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!path) return;
    try {
      const n = await api.logsExport(path);
      setMsg(`Exported ${n} entries to ${path}`);
    } catch (e) {
      setMsg(errorText(e));
    }
  };

  if (!data) return <p className="muted">Loading…</p>;
  const total = data.days.reduce((s, d) => s + d.minutes, 0);
  return (
    <div className="form">
      <div className="row between">
        <p>
          {data.entries.length} listening sessions, {Math.round(total)} minutes in total, over {data.days.length} days.
        </p>
        <button className="primary" onClick={exportCsv}>
          Export CSV…
        </button>
      </div>
      {msg && <div className="banner ok">{msg}</div>}
      <h3>Per day</h3>
      <table className="table">
        <thead>
          <tr>
            <th>Date</th>
            <th>Minutes</th>
          </tr>
        </thead>
        <tbody>
          {data.days.slice(0, 60).map((d) => (
            <tr key={d.date}>
              <td>{d.date}</td>
              <td>{d.minutes.toFixed(1)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3>Latest sessions</h3>
      <table className="table">
        <thead>
          <tr>
            <th>Started</th>
            <th>Track</th>
            <th>Duration</th>
            <th>Volume</th>
          </tr>
        </thead>
        <tbody>
          {data.entries.slice(0, 200).map((e) => (
            <tr key={e.started_at + e.track}>
              <td>{new Date(e.started_at).toLocaleString()}</td>
              <td>{e.track}</td>
              <td>{fmtTime(e.seconds)}</td>
              <td>{e.volume_db} dB</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SecurityTab() {
  const [pin, setPin] = useState("");
  const [dir, setDir] = useState("");
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  useEffect(() => {
    api.dataDir().then(setDir);
  }, []);
  const change = async () => {
    try {
      await api.setPin(pin);
      setPin("");
      setMsg({ ok: true, text: "PIN changed." });
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) });
    }
  };
  return (
    <div className="form">
      <label>
        New clinician PIN
        <input type="password" value={pin} onChange={(e) => setPin(e.target.value)} />
      </label>
      <div className="row end">
        <button className="primary" onClick={change}>
          Change PIN
        </button>
      </div>
      {msg && <div className={msg.ok ? "banner ok" : "banner warn"}>{msg.text}</div>}
      <p className="muted">
        All data stays on this computer, in <code>{dir}</code>: the participant profile, listening log, matching reports
        and a verification report for every track played. The PIN is a soft lock. Someone who opens that folder can
        read the profile, including the arm.
      </p>
    </div>
  );
}

export default function Clinician({
  status,
  lastMatch,
  onRunMatching,
  refresh,
}: {
  status: Status;
  lastMatch: MatchSummary | null;
  onRunMatching: () => void;
  refresh: () => void;
}) {
  const [tab, setTab] = useState<Tab>("participant");
  const tabs: [Tab, string][] = [
    ["participant", "Participant"],
    ["library", "Music library"],
    ["log", "Listening log"],
    ["security", "PIN & data"],
  ];
  return (
    <section className="card wide">
      <h1>Clinician tools</h1>
      <nav className="tabs">
        {tabs.map(([id, label]) => (
          <button key={id} className={tab === id ? "tab active" : "tab"} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </nav>
      {tab === "participant" && <ParticipantTab lastMatch={lastMatch} onRunMatching={onRunMatching} />}
      {tab === "library" && <LibraryTab status={status} refresh={refresh} />}
      {tab === "log" && <LogTab />}
      {tab === "security" && <SecurityTab />}
    </section>
  );
}
