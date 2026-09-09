import { useEffect, useState } from "react";

import { api, type Account as AccountData } from "../../lib/api";
import { Empty } from "../../components/ItemList";

/**
 * The Account page: who you are signed in as, and how to stop being.
 *
 * A page of its own rather than a Settings row, because the brief lists it as
 * one and because it is where devices and plan will land. What it shows when no
 * provider is configured is deliberate too: an explanation, not a dead button.
 */
export function Account({ revision, onChanged }: { revision: number; onChanged: () => void }) {
  const [account, setAccount] = useState<AccountData | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    void api
      .account()
      .then((a) => live && setAccount(a))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [revision]);

  const signIn = async () => {
    setBusy(true);
    setNote(null);
    try {
      setAccount(await api.signIn());
      onChanged();
    } catch (e) {
      setNote(String(e).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  const signOut = async () => {
    try {
      setAccount(await api.signOut());
      setNote(null);
      onChanged();
    } catch (e) {
      setNote(String(e));
    }
  };

  if (!account) return null;

  return (
    <div className="panel">
      <h1>Account</h1>
      <p className="sub">
        Optional, and it stays that way. Capture, search and everything you have already saved
        work signed out — an account is what will carry them between machines when sync arrives.
      </p>

      {!account.available ? (
        <Empty title="This build has no sign-in credentials">
          The app's own Google client is compiled in from <code>.env</code> at the repository
          root — see <code>.env.example</code>. A checkout without one builds a working app with
          sign-in switched off.
        </Empty>
      ) : (
        <div className="card">
          <h2>{account.signed_in ? "Signed in" : "Not signed in"}</h2>

          <div className="row last">
            <span className="bd">
              <span className="k">
                {account.signed_in
                  ? (account.display_name ?? account.email ?? "Signed in")
                  : "No account connected"}
              </span>
              <span className="v">
                {account.signed_in
                  ? (account.email ?? "This account has no email address attached.")
                  : "Sign in with Google. It opens your real browser rather than a window " +
                    "inside the app, so you can see the address of the site you are trusting."}
              </span>
              {account.signed_in && account.signed_in_at && (
                <span className="v" style={{ marginTop: 4, display: "block" }}>
                  Since {new Date(account.signed_in_at).toLocaleDateString()}
                </span>
              )}
              {note && (
                <span className="v" style={{ marginTop: 4, display: "block", color: "var(--bad)" }}>
                  {note}
                </span>
              )}
            </span>

            {account.signed_in ? (
              <button type="button" className="btn" onClick={() => void signOut()}>
                Sign out
              </button>
            ) : (
              <button
                type="button"
                className="btn primary"
                disabled={busy}
                onClick={() => void signIn()}
              >
                {busy ? "Waiting for browser…" : "Sign in with Google"}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
