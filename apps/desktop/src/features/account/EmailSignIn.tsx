import { useState } from "react";

import { api, type Account } from "../../lib/api";

/**
 * Sign in with an email address and a six-digit code.
 *
 * A code rather than a magic link, because a link has to arrive back at *this*
 * process on *this* machine, and the desktop is exactly where that is hard —
 * clicking it on a phone would sign in the wrong device. A code the person
 * reads and types works from wherever they happen to read their mail, which is
 * the point of offering email at all.
 */
export function EmailSignIn({ onSignedIn }: { onSignedIn: (a: Account) => void }) {
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  // The address the code was actually sent to, which is also the flag for
  // which of the two steps we are on. Holding the sent address separately
  // matters: editing the field after sending must not silently verify a code
  // against a different address.
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const send = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.emailStart(email);
      setSentTo(email.trim());
    } catch (e) {
      setError(clean(e));
    } finally {
      setBusy(false);
    }
  };

  const verify = async () => {
    if (!sentTo) return;
    setBusy(true);
    setError(null);
    try {
      onSignedIn(await api.emailVerify(sentTo, code));
    } catch (e) {
      setError(clean(e));
    } finally {
      setBusy(false);
    }
  };

  if (!sentTo) {
    return (
      <form
        className="email-step"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <input
          className="email-field"
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@example.com"
          autoComplete="email"
          spellCheck={false}
          disabled={busy}
        />
        {error && <p className="signin-error">{error}</p>}
        <button type="submit" className="provider" disabled={busy || !email.trim()}>
          {busy ? "Sending…" : "Email me a code"}
        </button>
      </form>
    );
  }

  return (
    <form
      className="email-step"
      onSubmit={(e) => {
        e.preventDefault();
        void verify();
      }}
    >
      <p className="email-sent">
        Code sent to <b>{sentTo}</b>. It expires in five minutes.
      </p>
      <input
        className="email-field code"
        value={code}
        onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
        placeholder="000000"
        inputMode="numeric"
        autoComplete="one-time-code"
        autoFocus
        disabled={busy}
      />
      {error && <p className="signin-error">{error}</p>}
      <button type="submit" className="provider" disabled={busy || code.length < 6}>
        {busy ? "Checking…" : "Sign in"}
      </button>
      <button
        type="button"
        className="skip"
        disabled={busy}
        onClick={() => {
          setSentTo(null);
          setCode("");
          setError(null);
        }}
      >
        Use a different address
      </button>
    </form>
  );
}

/** Tauri prefixes command errors; the user did not ask for that word. */
function clean(e: unknown): string {
  return String(e).replace(/^Error:\s*/, "");
}
