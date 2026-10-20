import { useCallback, useEffect, useState } from "react";
import { api, errorText, type MatchSummary, type Status } from "./api";
import Participant from "./Participant";
import Clinician from "./Clinician";
import Matching from "./Matching";
import PinDialog from "./PinDialog";

type View = { name: "participant" } | { name: "clinician" } | { name: "matching"; mode: "self" | "clinician" };

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [view, setView] = useState<View>({ name: "participant" });
  const [pinOpen, setPinOpen] = useState(false);
  const [lastMatch, setLastMatch] = useState<MatchSummary | null>(null);
  const [fatal, setFatal] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.status());
      setFatal(null);
    } catch (e) {
      setFatal(errorText(e));
    }
  }, []);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 500);
    return () => clearInterval(id);
  }, [refresh]);

  if (!status) {
    return <div className="loading">{fatal ?? "Starting…"}</div>;
  }

  const lock = async () => {
    await api.lock();
    setView({ name: "participant" });
    refresh();
  };

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="logo" aria-hidden>
            ◖◗
          </span>
          Tinnitus Music
        </div>
        <div className="topbar-right">
          {status.participant_id && <span className="chip">ID {status.participant_id}</span>}
          {view.name !== "matching" &&
            (status.clinician_unlocked ? (
              <>
                {view.name === "participant" ? (
                  <button className="ghost" onClick={() => setView({ name: "clinician" })}>
                    Clinician tools
                  </button>
                ) : (
                  <button className="ghost" onClick={() => setView({ name: "participant" })}>
                    Participant view
                  </button>
                )}
                <button className="ghost" onClick={lock}>
                  Lock
                </button>
              </>
            ) : (
              <button className="ghost" onClick={() => setPinOpen(true)}>
                Clinician
              </button>
            ))}
        </div>
      </header>

      {status.error && view.name !== "matching" && <div className="banner warn">{status.error}</div>}

      <main>
        {view.name === "participant" && (
          <Participant status={status} onSetup={() => setView({ name: "matching", mode: "self" })} refresh={refresh} />
        )}
        {view.name === "clinician" && status.clinician_unlocked && (
          <Clinician
            status={status}
            lastMatch={lastMatch}
            onRunMatching={() => setView({ name: "matching", mode: "clinician" })}
            refresh={refresh}
          />
        )}
        {view.name === "matching" && (
          <Matching
            mode={view.mode}
            onExit={(summary) => {
              if (summary) setLastMatch(summary);
              setView(view.mode === "clinician" ? { name: "clinician" } : { name: "participant" });
              refresh();
            }}
          />
        )}
      </main>

      {pinOpen && (
        <PinDialog
          pinSet={status.pin_set}
          onClose={(ok) => {
            setPinOpen(false);
            if (ok) setView({ name: "clinician" });
            refresh();
          }}
        />
      )}
    </div>
  );
}
