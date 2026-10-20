import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorText, type Answer, type Ear, type MatchSummary, type MatchView, type TinnitusSound } from "./api";

export default function Matching({ mode, onExit }: { mode: "self" | "clinician"; onExit: (s: MatchSummary | null) => void }) {
  const [view, setView] = useState<MatchView | null>(null);
  const [sound, setSound] = useState<TinnitusSound>("tonal");
  const [ear, setEar] = useState<Ear>("both");
  const [repeats, setRepeats] = useState(3);
  const [playing, setPlaying] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const playId = useRef(0);

  const play = useCallback(async () => {
    const id = ++playId.current;
    setPlaying(true);
    try {
      await api.matchPlay();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      if (id === playId.current) setPlaying(false);
    }
  }, []);

  // Each new question plays its sound automatically.
  const trial = view?.kind === "prompt" ? view.trial : null;
  useEffect(() => {
    if (trial !== null) play();
  }, [trial, play]);

  const answer = useCallback(
    async (a: Answer) => {
      if (playing) return;
      try {
        setView(await api.matchAnswer(a));
      } catch (e) {
        setErr(errorText(e));
      }
    },
    [playing],
  );

  // Number keys pick an answer, R replays.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (view?.kind !== "prompt") return;
      if (e.key.toLowerCase() === "r" && !playing) play();
      const i = Number(e.key) - 1;
      if (i >= 0 && i < view.options.length) answer(view.options[i].answer);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [view, playing, play, answer]);

  const start = async () => {
    setErr(null);
    try {
      setView(await api.matchStart({ ear, sound, repeats }));
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const cancel = async () => {
    if (view?.kind === "prompt" && !confirm("Stop matching? Your answers so far will be discarded.")) return;
    await api.matchCancel();
    onExit(view?.kind === "done" ? view.summary : null);
  };

  if (!view) {
    return (
      <section className="card narrow">
        <h1>Tinnitus matching</h1>
        <ol className="steps">
          <li>Sit somewhere quiet and put on headphones.</li>
          <li>
            Set the Windows volume to about half and <b>do not change it</b> during the test. Sounds start very soft.
          </li>
          <li>You will hear short sounds and answer simple questions about them. There are no wrong answers.</li>
          <li>Stop at any time if a sound is uncomfortable.</li>
        </ol>
        <fieldset>
          <legend>My tinnitus sounds most like</legend>
          <label className="radio">
            <input type="radio" checked={sound === "tonal"} onChange={() => setSound("tonal")} /> a tone: ringing, whistling
            or beeping
          </label>
          <label className="radio">
            <input type="radio" checked={sound === "noise_like"} onChange={() => setSound("noise_like")} /> a noise: hissing,
            rushing or buzzing
          </label>
        </fieldset>
        {mode === "clinician" && (
          <div className="row">
            <label>
              Test ear
              <select value={ear} onChange={(e) => setEar(e.target.value as Ear)}>
                <option value="both">Both</option>
                <option value="left">Left only</option>
                <option value="right">Right only</option>
              </select>
            </label>
            <label>
              Pitch rounds
              <input type="number" min={1} max={5} value={repeats} onChange={(e) => setRepeats(Number(e.target.value))} />
            </label>
          </div>
        )}
        {err && <div className="banner warn">{err}</div>}
        <div className="row end">
          <button className="ghost" onClick={() => onExit(null)}>
            Cancel
          </button>
          <button className="primary" onClick={start}>
            Start
          </button>
        </div>
      </section>
    );
  }

  if (view.kind === "done") {
    const s = view.summary;
    return (
      <section className="card narrow">
        <h1>Result</h1>
        <div className="result">
          <div className="big-number">{Math.round(s.pitch_hz).toLocaleString()} Hz</div>
          <div className="muted">Estimated tinnitus pitch</div>
        </div>
        {mode === "clinician" && (
          <table className="kv">
            <tbody>
              <tr>
                <th>Pitch matches</th>
                <td>
                  {s.pitch_matches_hz.map((f) => Math.round(f)).join(", ")} Hz (spread {s.pitch_spread_oct.toFixed(2)} oct)
                </td>
              </tr>
              <tr>
                <th>Octave check</th>
                <td>{s.octave_shift === 0 ? "no change" : `${s.octave_shift > 0 ? "+" : ""}${s.octave_shift} octave`}</td>
              </tr>
              <tr>
                <th>Loudness at pitch</th>
                <td>{s.loudness_sl_db === null ? "n/a" : `${s.loudness_sl_db.toFixed(0)} dB SL`}</td>
              </tr>
              <tr>
                <th>Minimum masking level</th>
                <td>{s.mml_sl_db === null ? "n/a" : `${s.mml_sl_db.toFixed(0)} dB SL`}</td>
              </tr>
              <tr>
                <th>Trials</th>
                <td>{s.trials}</td>
              </tr>
              <tr>
                <th>Report file</th>
                <td>{s.report_file}</td>
              </tr>
            </tbody>
          </table>
        )}
        {s.retest_recommended && (
          <div className="banner warn">The pitch was not consistent. Repeating the test on another day is recommended.</div>
        )}
        {mode === "clinician" && s.warnings.length > 0 && (
          <ul className="warnings">
            {s.warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        )}
        <div className="row end">
          <button className="primary" onClick={() => onExit(s)}>
            {mode === "self" ? "Start listening" : "Back to clinician tools"}
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="card narrow matching">
      <div className="stage-head">
        <div className="dots" aria-label={`step ${view.step} of ${view.steps}`}>
          {Array.from({ length: view.steps }, (_, i) => (
            <span key={i} className={i + 1 < view.step ? "dot done" : i + 1 === view.step ? "dot now" : "dot"} />
          ))}
        </div>
        <span className="muted">Question {view.trial}</span>
      </div>
      <h2>{view.stage}</h2>
      <p className="question">{view.question}</p>
      <div className={`answers n${view.options.length}`}>
        {view.options.map((o, i) => (
          <button key={o.answer} className="answer" disabled={playing} onClick={() => answer(o.answer)}>
            <kbd>{i + 1}</kbd> {o.label}
          </button>
        ))}
      </div>
      <div className="row between">
        <button className="ghost" onClick={cancel}>
          Stop
        </button>
        <button className="ghost" disabled={playing} onClick={play}>
          {playing ? "Playing…" : "Play again (R)"}
        </button>
      </div>
      {err && <div className="banner warn">{err}</div>}
    </section>
  );
}
