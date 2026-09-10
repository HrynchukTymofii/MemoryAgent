import { useCallback, useEffect, useState } from "react";

import { api, type Referrals, type ReferralStatus } from "../../lib/api";
import { Sheet } from "../help/Shortcuts";
import { Logo } from "../../components/Logo";

type Tab = "refer" | "past" | "apply";

/**
 * Refer and earn rewards.
 *
 * Three tabs because there are three genuinely different things a person comes
 * here to do — send the link, see whether it worked, or redeem one somebody
 * sent them — and folding the third into the first is what makes a referral
 * screen confusing: half the people opening it have a code in their clipboard
 * rather than a friend in mind.
 *
 * Nothing here decides anything. The code, the counts and the entitlement all
 * come from the API, and the rules that stop this being a way to mint free
 * months live in the database — see `cloud/migrations/003_referrals.sql`.
 */
export function Referral({ onClose, onSignIn }: { onClose: () => void; onSignIn: () => void }) {
  const [state, setState] = useState<Referrals | null>(null);
  const [tab, setTab] = useState<Tab>("refer");

  const load = useCallback(async () => {
    try {
      setState(await api.referralStatus());
    } catch (e) {
      setState({
        available: true,
        signed_in: true,
        status: null,
        error: String(e).replace(/^Error:\s*/, ""),
      });
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Sheet title="Refer and earn rewards" onClose={onClose}>
      {state === null ? null : !state.available ? (
        <p className="sheet-lead">
          This build has no API configured, so there is nothing to refer anyone to. Referrals need
          an account, and accounts need the service in <code>cloud/</code> to be running.
        </p>
      ) : !state.signed_in ? (
        <>
          <p className="sheet-lead">
            A referral belongs to an account — it is how we know whose month to extend. Sign in and
            this becomes your link.
          </p>
          <button type="button" className="btn primary" onClick={onSignIn}>
            Sign in
          </button>
        </>
      ) : state.status === null ? (
        <p className="sheet-lead bad">{state.error ?? "The API did not answer."}</p>
      ) : (
        <Loaded status={state.status} tab={tab} onTab={setTab} onChanged={load} />
      )}
    </Sheet>
  );
}

function Loaded({
  status,
  tab,
  onTab,
  onChanged,
}: {
  status: ReferralStatus;
  tab: Tab;
  onTab: (t: Tab) => void;
  onChanged: () => Promise<void>;
}) {
  const months = status.months_per_referral;
  return (
    <>
      <p className="sheet-lead">
        Give a month of Pro and get <b>{months === 1 ? "1 month" : `${months} months`}</b> for each
        person you refer.
      </p>

      <div className="tabs" role="tablist">
        <Tabbed on={tab === "refer"} onClick={() => onTab("refer")}>
          Refer
        </Tabbed>
        <Tabbed on={tab === "past"} onClick={() => onTab("past")}>
          Past referrals ({status.referrals.length})
        </Tabbed>
        <Tabbed on={tab === "apply"} onClick={() => onTab("apply")}>
          Apply referral
        </Tabbed>
      </div>

      {tab === "refer" && <Refer status={status} />}
      {tab === "past" && <Past status={status} />}
      {tab === "apply" && <Apply status={status} onChanged={onChanged} />}

      <p className="sheet-foot">
        Rewards are applied to your plan as soon as the person you referred has dictated{" "}
        {status.qualify_words.toLocaleString()} words. Nothing is charged and nothing is cancelled
        — a month is added to whatever you already have.
      </p>
    </>
  );
}

function Refer({ status }: { status: ReferralStatus }) {
  const [copied, setCopied] = useState(false);
  const [emails, setEmails] = useState("");
  const [sending, setSending] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(status.link);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1_800);
    } catch {
      // A clipboard that refuses is not worth an error message; the link is on
      // screen and selectable, which is the fallback anyway.
    }
  };

  const send = async () => {
    // Commas, semicolons, spaces and newlines all separate addresses. People
    // paste from a mail client, and which of those it uses is not something
    // they should have to think about.
    const list = emails
      .split(/[\s,;]+/)
      .map((e) => e.trim())
      .filter(Boolean);
    if (list.length === 0) return;

    setSending(true);
    setNote(null);
    try {
      const out = await api.sendInvites(list);
      setEmails(out.failed.join(", "));
      setNote(
        out.failed.length === 0
          ? `Sent to ${out.sent.length === 1 ? "1 address" : `${out.sent.length} addresses`}.`
          : `Sent ${out.sent.length}. ${out.failed.length} did not go — they are still in the box.`,
      );
    } catch (e) {
      setNote(String(e).replace(/^Error:\s*/, ""));
    } finally {
      setSending(false);
    }
  };

  return (
    <>
      {/* The thing being given, drawn as the thing it is. A link with a
          paragraph over it is a form; this is a gift someone is handing over. */}
      <div className="gift">
        <svg className="wave" viewBox="0 0 320 60" aria-hidden="true" preserveAspectRatio="none">
          <path d="M0 34C58 4 104 46 168 30 226 16 268 2 320 8" />
        </svg>
        <div className="gift-mark">
          <Logo size={24} />
          Memory OS
          <span className="tier">Pro</span>
        </div>
        <div className="gift-what">
          UNLIMITED CAPTURES FOR {status.months_per_referral === 1 ? "1 MONTH" : `${status.months_per_referral} MONTHS`}
        </div>
      </div>

      <div className="sheet-h">How it works</div>
      <ol className="how">
        <li>Share your invite link.</li>
        <li>
          They sign up and get <b>a free month of Pro</b>.
        </li>
        <li>
          You get <b>a free month</b> when they dictate {status.qualify_words.toLocaleString()}{" "}
          words.
        </li>
      </ol>

      <div className="sheet-h">Your invite link</div>
      <div className="field">
        <input className="cred" readOnly value={status.link} onFocus={(e) => e.target.select()} />
        <button type="button" className="btn" onClick={() => void copy()}>
          {copied ? "Copied" : "Copy"}
        </button>
      </div>

      <div className="sheet-h">Send invites</div>
      <div className="field">
        <input
          className="cred"
          type="text"
          placeholder="email@example.com"
          value={emails}
          onChange={(e) => setEmails(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void send()}
        />
        <button
          type="button"
          className="btn primary"
          disabled={sending || emails.trim() === ""}
          onClick={() => void send()}
        >
          {sending ? "Sending…" : "Send"}
        </button>
      </div>
      {note && <p className="fieldnote">{note}</p>}
    </>
  );
}

function Past({ status }: { status: ReferralStatus }) {
  if (status.referrals.length === 0) {
    return (
      <div className="empty">
        <strong>Nobody yet</strong>
        Send your link from the Refer tab. People you bring in appear here, with whether their
        month — and yours — has been earned.
      </div>
    );
  }
  return (
    <>
      <div className="tally">
        <div className="fig">
          <b>{status.referrals.filter((r) => r.status === "qualified").length}</b>
          <span>qualified</span>
        </div>
        <div className="fig">
          <b>{status.referrals.filter((r) => r.status !== "qualified").length}</b>
          <span>still to use it</span>
        </div>
        <div className="fig">
          <b>{status.months_earned}</b>
          <span>months earned</span>
        </div>
      </div>
      <div className="refs">
        {status.referrals.map((r) => (
          <div key={r.who + r.created_at} className="ref">
            <span className="bd">
              <span className="t">{r.who}</span>
              <span className="s">
                Invited {new Date(r.created_at).toLocaleDateString()}
                {r.qualified_at && ` · earned ${new Date(r.qualified_at).toLocaleDateString()}`}
              </span>
            </span>
            <span className={r.status === "qualified" ? "chip ok" : "chip"}>
              {r.status === "qualified" ? "Earned" : "Pending"}
            </span>
          </div>
        ))}
      </div>
    </>
  );
}

function Apply({ status, onChanged }: { status: ReferralStatus; onChanged: () => Promise<void> }) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (status.applied_code) {
    return (
      <div className="empty">
        <strong>You were referred with {status.applied_code}</strong>
        A code can only be applied once, and yours is already in. Your month is added as soon as
        you have dictated {status.qualify_words.toLocaleString()} words.
      </div>
    );
  }

  if (!status.can_apply) {
    return (
      <div className="empty">
        <strong>This account is past the window</strong>
        A code can only be applied in the first weeks after signing up — it is a claim that
        somebody brought you in, and by now you brought yourself.
      </div>
    );
  }

  const apply = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.applyReferral(code);
      await onChanged();
    } catch (e) {
      setError(String(e).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <p className="sheet-lead">
        Somebody sent you a code? Put it in and your month of Pro starts now — theirs follows once
        you have used it properly.
      </p>
      <div className="field">
        <input
          className="cred"
          type="text"
          placeholder="TYMOFII28QK"
          value={code}
          spellCheck={false}
          onChange={(e) => setCode(e.target.value.toUpperCase())}
          onKeyDown={(e) => e.key === "Enter" && void apply()}
        />
        <button
          type="button"
          className="btn primary"
          disabled={busy || code.trim().length < 4}
          onClick={() => void apply()}
        >
          {busy ? "Applying…" : "Apply"}
        </button>
      </div>
      {error && <p className="fieldnote bad">{error}</p>}
    </>
  );
}

function Tabbed({
  on,
  onClick,
  children,
}: {
  on: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button type="button" className={on ? "tab on" : "tab"} role="tab" aria-selected={on} onClick={onClick}>
      {children}
    </button>
  );
}
