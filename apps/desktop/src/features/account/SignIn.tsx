import { useState } from "react";

import { api, type Account } from "../../lib/api";

/**
 * The sign-in screen, shown once.
 *
 * It is a full screen rather than a row in Settings because that is what it is
 * for: the one moment the product asks who you are, phrased so the answer can
 * be "not now" without the asking feeling like a wall. Skipping is recorded, so
 * it is asked once and then never again unless you go looking for it.
 *
 * What it must never become is a gate. Everything behind it works signed out —
 * capture, search, the archive — and ADR-0007 says the archive is never gated.
 * So "Continue without an account" is a peer of the sign-in buttons, not a
 * grey link hidden underneath them.
 */
export function SignIn({
  onDone,
  onSignedIn,
}: {
  onDone: () => void;
  onSignedIn: (a: Account) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const signIn = async () => {
    setBusy(true);
    setError(null);
    try {
      const account = await api.signIn();
      onSignedIn(account);
      onDone();
    } catch (e) {
      // Cancelling in the browser lands here and is not a failure. Said plainly
      // and without alarm: nothing changed, and nothing was lost.
      setError(String(e).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  const skip = async () => {
    try {
      await api.dismissSignInPrompt();
    } catch {
      // Worst case it is offered once more next launch. Not worth blocking on.
    }
    onDone();
  };

  return (
    <div className="signin">
      <div className="signin-card">
        <span className="bars" aria-hidden="true">
          <i style={{ height: 9 }} />
          <i style={{ height: 17 }} />
          <i style={{ height: 13 }} />
          <i style={{ height: 20 }} />
        </span>

        <h1>Memory OS</h1>
        <p className="lead">
          An account is how your memories will reach your other machines when sync arrives, and
          how we know who is actually using this.
        </p>

        <button
          type="button"
          className="provider"
          onClick={() => void signIn()}
          disabled={busy}
        >
          <span className="mark" aria-hidden="true">
            G
          </span>
          {busy ? "Waiting for your browser…" : "Continue with Google"}
        </button>

        {error && <p className="signin-error">{error}</p>}

        <button type="button" className="skip" onClick={() => void skip()} disabled={busy}>
          Continue without an account
        </button>

        <p className="fine">
          Capture, search and everything you save work without an account, and always will.
          Nothing you have captured leaves this machine either way.
        </p>
      </div>
    </div>
  );
}
