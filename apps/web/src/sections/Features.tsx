import type { ReactNode } from "react";

/**
 * What it actually does, one claim per card, each with the mechanism drawn
 * beside it.
 *
 * The drawings are the point: four cards of text with an icon on top would say
 * the same thing four times. Each of these shows the shape of the thing it
 * describes — a key and a voice, two questions arriving at one answer, a
 * sentence sorting itself, a closed machine.
 *
 * They respond to a pointer and to nothing else. Cards that animate themselves
 * into view are the house style of every page that has nothing to show.
 */
export function Features() {
  return (
    <section className="feats">
      <h2 className="feats-head">Four things, and no more than four</h2>
      <div className="feat-grid">
        <Feat
          title="Hands full is the normal case"
          body="Hold one shortcut and talk. No window to find, no field to click into, no app in front of what you were doing. Let go and it is saved."
          art={<KeyArt />}
        />
        <Feat
          title="Ask in the words you actually remember"
          body="Search on meaning, not spelling. “That thing about sleep” finds the note where you said circadian, and the page you kept about waking up at four."
          art={<MeaningArt />}
        />
        <Feat
          title="It sorts itself out"
          body="A sentence with a date in it becomes a task. A page you kept lands in the collection it belongs to. You get a library, not a pile."
          art={<FileArt />}
        />
        <Feat
          title="Your machine, and only your machine"
          body="Speech and search run on your laptop — the models ship with the app. There is no upload step, no account required, and nothing to leak."
          art={<LocalArt />}
        />
      </div>
    </section>
  );
}

function Feat({ title, body, art }: { title: string; body: string; art: ReactNode }) {
  return (
    <article className="feat">
      <div className="feat-art">{art}</div>
      <h3>{title}</h3>
      <p>{body}</p>
    </article>
  );
}

/* The drawings share a 120 × 72 field, one stroke weight, and one accented
   element each — the part that moves when the pointer is on the card. */

function KeyArt() {
  return (
    <svg viewBox="0 0 120 72" className="art">
      <rect className="a-key" x="12" y="24" width="42" height="24" rx="6" />
      <path d="M22 36h22" />
      <g className="a-live">
        {[0, 1, 2, 3, 4].map((i) => (
          <path key={i} d={`M${68 + i * 11} ${36 - [7, 14, 20, 12, 6][i]}v${[14, 28, 40, 24, 12][i]}`} />
        ))}
      </g>
    </svg>
  );
}

function MeaningArt() {
  return (
    <svg viewBox="0 0 120 72" className="art">
      <path d="M10 18h34" />
      <path d="M10 54h34" />
      <path className="a-join" d="M44 18c22 0 14 18 32 18M44 54c22 0 14-18 32-18" />
      <circle className="a-dot" cx="82" cy="36" r="7" />
      <path d="M96 36h14" />
    </svg>
  );
}

function FileArt() {
  return (
    <svg viewBox="0 0 120 72" className="art">
      <path d="M8 16h46" />
      <path d="M8 26h30" />
      <path className="a-drop" d="M62 21h20l6 8" />
      <rect className="a-bin" x="74" y="30" width="36" height="26" rx="5" />
      <path d="M74 38h36" />
    </svg>
  );
}

function LocalArt() {
  return (
    <svg viewBox="0 0 120 72" className="art">
      <rect className="a-box" x="26" y="16" width="68" height="40" rx="6" />
      <path d="M40 62h40" />
      <path className="a-loop" d="M46 36a14 10 0 1 0 28 0 14 10 0 1 0-28 0" />
      <path className="a-out" d="M94 30h12" />
      <path className="a-cut" d="M104 24 116 36M116 24 104 36" />
    </svg>
  );
}
