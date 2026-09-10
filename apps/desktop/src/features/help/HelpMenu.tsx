import { useEffect, useRef, useState } from "react";

import { api } from "../../lib/api";
import { Sheet, Shortcuts } from "./Shortcuts";
import { VoiceCommands } from "./VoiceCommands";
import { RELEASES } from "./whats-new";
import {
  BookIcon,
  BugIcon,
  GlobeIcon,
  HelpIcon,
  MailIcon,
  MicIcon,
  SparkIcon,
  StarIcon,
} from "../../components/icons";

/** Where "Documentation" and "Email support" actually go. */
const DOCS = "https://github.com/Tim090909/MemAgent#readme";
const SUPPORT = "mailto:support@memoryos.app?subject=Memory%20OS";

type Panel = "shortcuts" | "say" | "new" | null;

/**
 * Help, at the very bottom of the sidebar.
 *
 * The last row rather than a page, because none of what is behind it is
 * something you go and sit in — it is looked up, read, and shut. And a menu
 * rather than a link out: three of these five answers are in the app already,
 * and sending someone to a website to be told their own shortcut would be
 * absurd.
 *
 * It opens upward. It is the bottom-most row on screen, and a popover that
 * drops from it would open into the taskbar.
 */
export function HelpMenu({ railed }: { railed: boolean }) {
  const [open, setOpen] = useState(false);
  const [sheet, setSheet] = useState<Panel>(null);
  const [note, setNote] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);

  // Escape shuts the menu; clicking elsewhere is handled by the scrim.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  const go = (s: Panel) => {
    setOpen(false);
    setSheet(s);
  };

  /** Writes the command log out and says where it went. */
  const report = async () => {
    setOpen(false);
    try {
      const path = await api.exportCommandLog();
      setNote(`Log written to ${path}. Attach it to your message.`);
    } catch (e) {
      setNote(String(e).replace(/^Error:\s*/, ""));
    }
  };

  return (
    <>
      <div className="help" ref={box}>
        {open && <div className="scrim" onClick={() => setOpen(false)} />}

        {open && (
          <div className="help-menu" role="menu">
            <div className="help-h">Learn</div>
            <Row icon={<StarIcon />} label="What's new" onClick={() => go("new")} />
            <Row icon={<SparkIcon />} label="What you can say" onClick={() => go("say")} />

            <div className="help-sep" />
            <div className="help-h">Essentials</div>
            <Row icon={<HelpIcon />} label="Shortcuts" onClick={() => go("shortcuts")} />
            <Row
              icon={<MicIcon />}
              label="Microphone"
              onClick={() => {
                setOpen(false);
                jump("capture");
              }}
            />
            <Row
              icon={<GlobeIcon />}
              label="Models"
              onClick={() => {
                setOpen(false);
                jump("models");
              }}
            />

            <div className="help-sep" />
            <div className="help-h">Get in touch</div>
            <Row icon={<BookIcon />} label="Documentation" href={DOCS} />
            <Row icon={<MailIcon />} label="Email support" href={SUPPORT} />
            <Row icon={<BugIcon />} label="Report a problem" onClick={() => void report()} />
          </div>
        )}

        <button
          type="button"
          className={open ? "help-btn on" : "help-btn"}
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          title={railed ? "Help" : undefined}
        >
          <span className="g">
            <HelpIcon />
          </span>
          <span className="lbl">Help</span>
        </button>
      </div>

      {note && (
        <div className="help-note" role="status">
          {note}
          <button type="button" onClick={() => setNote(null)} aria-label="Dismiss">
            ×
          </button>
        </div>
      )}

      {sheet === "shortcuts" && <Shortcuts onClose={() => setSheet(null)} />}
      {sheet === "say" && <VoiceCommands onClose={() => setSheet(null)} />}
      {sheet === "new" && <WhatsNew onClose={() => setSheet(null)} />}
    </>
  );
}

/**
 * Scroll a Settings section into view, from anywhere.
 *
 * The Hub is one page per nav item and Settings is long; "Microphone" that
 * lands you at the top of a page with the microphone three screens down has not
 * answered the question. The click first switches to Settings, then finds the
 * anchor once it has rendered.
 */
function jump(anchor: string) {
  const nav = document.querySelector<HTMLElement>('[data-page="settings"]');
  nav?.click();
  // One frame for the page to mount, then find it. If the section is not there
  // — an older Settings, a build without a microphone panel — the page still
  // opened, which is the useful half of the answer.
  window.setTimeout(() => {
    document.getElementById(anchor)?.scrollIntoView({ behavior: "smooth", block: "start" });
  }, 60);
}

function Row({
  icon,
  label,
  onClick,
  href,
}: {
  icon: React.ReactNode;
  label: string;
  onClick?: () => void;
  href?: string;
}) {
  const body = (
    <>
      <span className="g">{icon}</span>
      {label}
    </>
  );
  return href ? (
    <a className="help-row" href={href} target="_blank" rel="noreferrer" role="menuitem">
      {body}
    </a>
  ) : (
    <button type="button" className="help-row" onClick={onClick} role="menuitem">
      {body}
    </button>
  );
}

function WhatsNew({ onClose }: { onClose: () => void }) {
  return (
    <Sheet title="What's new" onClose={onClose}>
      {RELEASES.map((r) => (
        <section key={r.version} className="release">
          <div className="sheet-h">
            {r.version}
            <span className="when">{new Date(r.date).toLocaleDateString()}</span>
          </div>
          <ul>
            {r.changes.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        </section>
      ))}
    </Sheet>
  );
}
