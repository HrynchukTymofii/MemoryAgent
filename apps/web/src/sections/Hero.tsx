import { useEffect, useState } from "react";

import { useLoop, still } from "../lib/motion";

/**
 * The first screen: the claim, and the product doing its whole job beside it.
 *
 * The stage on the right is not a screenshot and not a gradient. It replays the
 * four seconds the product exists for — a key held, a sentence said, the words
 * arriving, the thing filed — because that loop is the only thing a visitor
 * actually has to understand, and it is faster to watch than to read.
 */

const SAID = "remember that wine — Occhipinti SP68, the one from dinner";

/** Held, listening, arriving, kept. The stage's four beats, in milliseconds. */
const BEATS = [1200, 1900, 2100, 3400];

export function Hero() {
  const step = useLoop(BEATS);

  return (
    <section className="hero">
      <div className="hero-say">
        <h1>
          You saved it.
          <br />
          Then you lost it.
        </h1>
        <p className="lede">
          The link a friend sent, the wine you liked, the thing you swore you would come back
          to. Say it out loud instead — one shortcut, one sentence — and find it later in
          whatever words you happen to remember.
        </p>
        <div className="hero-acts">
          <a className="btn" href="#get">
            Download for Windows
          </a>
          <a className="btn ghost" href="#how">
            Watch it work
          </a>
        </div>
        <p className="fine">Free while it is in beta. Nothing you say leaves your machine.</p>
      </div>

      <Stage step={step} />
    </section>
  );
}

/**
 * The capture, in four beats.
 *
 * The beat drives everything through a data attribute rather than through a
 * class per element, so the CSS holds the whole sequence in one place and the
 * component holds only where in it we are.
 */
function Stage({ step }: { step: number }) {
  return (
    <div className="stage" data-beat={step} aria-hidden="true">
      <div className="stage-glass">
        <div className="keycap">
          <kbd>Ctrl</kbd>
          <kbd>Space</kbd>
        </div>

        <div className="wave">
          {[0.35, 0.7, 1, 0.55, 0.8, 0.4].map((h, i) => (
            <i key={i} style={{ ["--h" as string]: h, ["--d" as string]: `${i * 90}ms` }} />
          ))}
        </div>

        <p className={step >= 2 ? "said" : "said hint"}>
          {step >= 2 ? <Typed text={SAID} /> : "hold the key and talk"}
        </p>
      </div>

      <div className="filed">
        <div className="filed-mark" />
        <div className="filed-text">
          <strong>Occhipinti SP68</strong>
          <span>the one from dinner</span>
        </div>
        <span className="filed-where">Wine</span>
      </div>
    </div>
  );
}

/** The words arriving as they are recognised, a few characters at a time. */
function Typed({ text }: { text: string }) {
  const [n, setN] = useState(still ? text.length : 0);

  useEffect(() => {
    if (still) return;
    const id = window.setInterval(() => {
      setN((c) => {
        if (c >= text.length) {
          window.clearInterval(id);
          return c;
        }
        return c + 2;
      });
    }, 34);
    return () => window.clearInterval(id);
  }, [text]);

  return (
    <>
      {text.slice(0, n)}
      <i className="caret" />
    </>
  );
}
