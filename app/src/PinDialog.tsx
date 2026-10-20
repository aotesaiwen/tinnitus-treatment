import { useState } from "react";
import { api, errorText } from "./api";

export default function PinDialog({ pinSet, onClose }: { pinSet: boolean; onClose: (ok: boolean) => void }) {
  const [pin, setPin] = useState("");
  const [again, setAgain] = useState("");
  const [err, setErr] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErr(null);
    try {
      if (pinSet) {
        if (await api.unlock(pin)) onClose(true);
        else setErr("Wrong PIN.");
      } else {
        if (pin !== again) return setErr("The PINs do not match.");
        await api.setPin(pin);
        onClose(true);
      }
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div className="overlay" onClick={() => onClose(false)}>
      <form className="card dialog" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <h2>{pinSet ? "Clinician access" : "Create a clinician PIN"}</h2>
        {!pinSet && (
          <p className="muted">
            The PIN keeps participants out of the clinician tools, which show the study arm. Use at least 4 digits.
          </p>
        )}
        <input type="password" inputMode="numeric" autoFocus placeholder="PIN" value={pin} onChange={(e) => setPin(e.target.value)} />
        {!pinSet && (
          <input type="password" inputMode="numeric" placeholder="Repeat PIN" value={again} onChange={(e) => setAgain(e.target.value)} />
        )}
        {err && <div className="banner warn">{err}</div>}
        <div className="row end">
          <button type="button" className="ghost" onClick={() => onClose(false)}>
            Cancel
          </button>
          <button type="submit" className="primary">
            {pinSet ? "Unlock" : "Create PIN"}
          </button>
        </div>
      </form>
    </div>
  );
}
