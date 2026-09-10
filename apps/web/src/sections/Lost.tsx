import { useTravel } from "../lib/motion";

/**
 * The problem, demonstrated rather than argued.
 *
 * Everything on this wall dims and drifts as the section goes past — the reader
 * loses them by scrolling, which is exactly how they were lost the first time.
 * One card does not move, because it was said out loud.
 *
 * This is the page's second and last piece of unprompted motion, and it is tied
 * to the reader's own scrolling rather than to a timer, so nothing happens
 * unless they make it happen.
 */

type Thing = { what: string; where: string; span?: number; kept?: boolean };

const THINGS: Thing[] = [
  { what: "That thread about sleep", where: "liked, some time in April" },
  { what: "The shoes", where: "screenshot 4,102" },
  { what: "The money you lent Alex", where: "agreed on a call, written down nowhere" },
  { what: "Occhipinti SP68", where: "said out loud, Tuesday", kept: true, span: 2 },
  { what: "The restaurant Dan swore by", where: "somewhere in the group chat" },
  { what: "That paper you were going to read", where: "Downloads, 1.2 GB in" },
  { what: "The keyboard shortcut from the workshop", where: "a note app you stopped opening" },
  { what: "A tab you kept open for five weeks", where: "closed by a restart" },
];

export function Lost() {
  const [ref, t] = useTravel<HTMLElement>();

  return (
    <section className="lost" id="proof" ref={ref}>
      <div className="lost-say">
        <h2>How many things have you liked, and then never found again?</h2>
        <p>
          They are not gone. They are in a tab, a chat, a downloads folder, a bookmark bar with
          nine hundred things in it. Which is the same as gone.
        </p>
      </div>

      <ul className="wall">
        {THINGS.map((thing, i) => {
          // Each card starts fading a little later than the one before, so the
          // wall empties in a wave rather than all at once.
          const start = 0.44 + (i % 4) * 0.05 + Math.floor(i / 4) * 0.035;
          const gone = thing.kept ? 0 : Math.max(0, Math.min(1, (t - start) / 0.28));
          return (
            <li
              key={thing.what}
              className={thing.kept ? "card kept" : "card"}
              style={{
                gridColumn: `span ${thing.span ?? 1}`,
                opacity: 1 - gone * 0.86,
                filter: gone > 0 ? `blur(${(gone * 3).toFixed(2)}px)` : undefined,
                transform: `translateY(${(gone * 26).toFixed(1)}px)`,
              }}
            >
              <strong>{thing.what}</strong>
              <span>{thing.where}</span>
              {thing.kept && <em>you said this one out loud</em>}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
