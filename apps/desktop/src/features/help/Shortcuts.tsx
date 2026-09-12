import { useEffect, useState } from "react";

import { api, type Settings } from "../../lib/api";

/**
 * The shortcut sheet.
 *
 * Every chord here is read from the running configuration rather than written
 * into the markup. The capture shortcut is the one thing in this app a user is
 * expected to rebind — other always-on voice tools install their own low-level
 * hooks and collide with ours — so a sheet that confidently prints Ctrl+Win to
 * somebody who moved it to F9 is worse than no sheet at all.
 */
export function Shortcuts({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<Settings | null>(null);

  useEffect(() => {
    let live = true;
    void api
      .settings()
      .then((s) => live && setSettings(s))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  return (
    <Sheet title="Shortcuts" onClose={onClose}>
      <p className="sheet-lead">
        Capture works anywhere in Windows, over any application. The Hub is only where what you
        captured lands.
      </p>

      <Group label="Speaking">
        <Row keys={settings ? chord(settings.hotkey) : ["…"]} held>
          Hold to capture. Speak while it is held, let go when you are done — the app hears the
          whole phrase, including what you said before the overlay appeared.
        </Row>
        {settings?.dictate_hotkey && (
          <Row keys={chord(settings.dictate_hotkey)} held>
            Hold to dictate. The words are typed into whatever you are working in — nothing is
            routed and nothing is saved. The transcript is punctuated and laid out by our own
            service; if that is unreachable, the raw words are typed instead.
          </Row>
        )}
        <Row keys={["Esc"]}>Drop what is being captured without saving it.</Row>
      </Group>

      <Group label="Answering">
        <Row keys={["1", "2", "3"]}>
          Pick a destination when the app asks which collection you meant.
        </Row>
        <Row keys={["Enter"]}>Take the first suggestion.</Row>
      </Group>

      <Group label="Saying">
        <Row keys={["“undo”"]} spoken>
          Reverses the last thing that was done. Spoken, not typed — and only ever by the
          grammar, never guessed by a model.
        </Row>
      </Group>

      {settings && (
        <p className="sheet-foot">
          The capture shortcut is <b>{settings.hotkey}</b>, held for{" "}
          <b>{settings.hold_threshold_ms} ms</b> before it engages. Change both in Settings.
        </p>
      )}
    </Sheet>
  );
}

/**
 * Split `"ctrl+win"` into the caps to draw.
 *
 * Capitalised for display only. The stored form is what the parser accepts and
 * is left exactly as it is — see `config.rs`.
 */
function chord(spec: string): string[] {
  return spec.split("+").map((part) => {
    const p = part.trim();
    const named: Record<string, string> = {
      ctrl: "Ctrl",
      lctrl: "Left Ctrl",
      rctrl: "Right Ctrl",
      alt: "Alt",
      lalt: "Left Alt",
      ralt: "Right Alt",
      shift: "Shift",
      lshift: "Left Shift",
      rshift: "Right Shift",
      win: "Win",
      lwin: "Left Win",
      rwin: "Right Win",
      space: "Space",
    };
    return named[p.toLowerCase()] ?? p.toUpperCase();
  });
}

export function Group({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <>
      <div className="sheet-h">{label}</div>
      <div className="keys">{children}</div>
    </>
  );
}

function Row({
  keys,
  held,
  spoken,
  children,
}: {
  keys: string[];
  /** Marks a chord that is held rather than pressed. */
  held?: boolean;
  /** Marks something said aloud, which is not a key at all. */
  spoken?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="key-row">
      <div className="key-caps">
        {keys.map((k, i) => (
          <span key={k + i}>
            {i > 0 && <span className="plus">{spoken ? "" : "+"}</span>}
            <kbd className={spoken ? "spoken" : undefined}>{k}</kbd>
          </span>
        ))}
        {held && <span className="hold">hold</span>}
      </div>
      <p>{children}</p>
    </div>
  );
}

/**
 * The sheet the help screens are drawn on.
 *
 * Shared by both of them, and closable three ways — the button, the backdrop,
 * and Escape — because a modal with one way out is one people get stuck in.
 */
export function Sheet({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <>
      <div className="backdrop" onClick={onClose} />
      <div className="sheet" role="dialog" aria-modal="true" aria-label={title}>
        <header className="sheet-top">
          <h2>{title}</h2>
          <button type="button" className="sheet-x" onClick={onClose} aria-label="Close">
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <path d="M1.5 1.5l9 9M10.5 1.5l-9 9" />
            </svg>
          </button>
        </header>
        <div className="sheet-body">{children}</div>
      </div>
    </>
  );
}
