import { useSeen } from "../lib/motion";

/**
 * The forgetting curve, and the line that does not fall.
 *
 * A chart earns its place here because the claim is about time, and time is the
 * one thing a paragraph is bad at. The grey line is Ebbinghaus's — measured in
 * 1885, replicated ever since — and the amber one is what a file on a disk does
 * over the same month, which is nothing.
 *
 * It draws itself once, when it is first reached, and only then: the drawing is
 * the reading order, left to right, the same direction as the time axis.
 */

const W = 660;
const H = 290;
const PAD = { l: 46, r: 18, t: 22, b: 38 };

/** Where each labelled moment sits across the axis, and how much survives it. */
const MARKS: { at: number; keep: number; label: string }[] = [
  { at: 0, keep: 1.0, label: "now" },
  { at: 0.15, keep: 0.58, label: "20 min" },
  { at: 0.3, keep: 0.44, label: "an hour" },
  { at: 0.55, keep: 0.33, label: "a day" },
  { at: 0.78, keep: 0.25, label: "a week" },
  { at: 1, keep: 0.21, label: "a month" },
];

const x = (at: number) => PAD.l + at * (W - PAD.l - PAD.r);
const y = (keep: number) => PAD.t + (1 - keep) * (H - PAD.t - PAD.b);

/** A path through the marks, rounded at each one rather than kinked. */
function curve() {
  const p = MARKS.map((m) => [x(m.at), y(m.keep)] as const);
  let d = `M${p[0][0]} ${p[0][1]}`;
  for (let i = 1; i < p.length; i++) {
    const [px, py] = p[i - 1];
    const [cx, cy] = p[i];
    const mx = (px + cx) / 2;
    d += `C${mx} ${py} ${mx} ${cy} ${cx} ${cy}`;
  }
  return d;
}

export function Curve() {
  const [ref, seen] = useSeen<HTMLElement>();

  return (
    <section className="curve" ref={ref}>
      <div className="curve-say">
        <h2>You will forget this by Thursday</h2>
        <p>
          Not a figure of speech. Most of what you take in is gone within a day, and the part
          that survives is the part you happened to repeat. The point of saying something out
          loud into this is that the second line is what happens to it afterwards.
        </p>
        <dl className="facts">
          <div>
            <dt>Held to capture</dt>
            <dd>one key</dd>
          </div>
          <div>
            <dt>Time to save a thought</dt>
            <dd>3 seconds</dd>
          </div>
          <div>
            <dt>Uploaded</dt>
            <dd>nothing</dd>
          </div>
        </dl>
      </div>

      <figure className={seen ? "plot drawn" : "plot"}>
        <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="What you remember over a month, against what the app still holds">
          {[0, 0.5, 1].map((k) => (
            <g key={k}>
              <line className="rule" x1={PAD.l} y1={y(k)} x2={W - PAD.r} y2={y(k)} />
              <text className="tick" x={PAD.l - 10} y={y(k) + 4} textAnchor="end">
                {k * 100}%
              </text>
            </g>
          ))}

          {MARKS.map((m) => (
            <text key={m.label} className="tick" x={x(m.at)} y={H - 12} textAnchor="middle">
              {m.label}
            </text>
          ))}

          <path className="held" d={`${curve()} L${x(1)} ${y(0)} L${x(0)} ${y(0)} Z`} />
          <path className="you" d={curve()} />
          <path className="app" d={`M${x(0)} ${y(1)} L${x(1)} ${y(1)}`} />

          <g className="notes">
            <text x={x(0.46)} y={y(0.3) + 26}>
              what you keep
            </text>
            <text x={x(0.46)} y={y(1) - 14}>
              what it keeps
            </text>
          </g>
        </svg>
        <figcaption>
          Retention after a single exposure, following Ebbinghaus (1885). The flat line is a
          file on your disk.
        </figcaption>
      </figure>
    </section>
  );
}
