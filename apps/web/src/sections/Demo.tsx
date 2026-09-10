import { useState } from "react";

/**
 * The recorded walkthrough.
 *
 * Nothing autoplays. A video that starts by itself on a landing page is the
 * reason people scroll past videos, and this one is two minutes of somebody
 * talking; it waits to be asked.
 *
 * Until the file exists the frame shows the still it would open on — the Hub,
 * mid-search — and says so plainly rather than showing a broken player. Drop
 * `demo.mp4` into `apps/web/public/` and the same frame plays it.
 */
export function Demo() {
  const [playing, setPlaying] = useState(false);
  const [missing, setMissing] = useState(false);

  return (
    <section className="demo" id="how">
      <div className="demo-say">
        <h2>Two minutes, start to finish</h2>
        <p>
          Capture something with your hands full, then go and find it a week later without
          remembering a single word you used.
        </p>
      </div>

      <div className="frame">
        {playing && !missing ? (
          <video
            className="frame-video"
            src="/demo.mp4"
            controls
            autoPlay
            playsInline
            onError={() => {
              setMissing(true);
              setPlaying(false);
            }}
          />
        ) : (
          <>
            <Still />
            <button
              type="button"
              className="play"
              onClick={() => setPlaying(true)}
              disabled={missing}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M9 6.5 18 12l-9 5.5z" />
              </svg>
              {missing ? "Recording in progress" : "Play the walkthrough"}
            </button>
          </>
        )}
      </div>
    </section>
  );
}

/**
 * The frame the video opens on, drawn rather than photographed: the Hub with a
 * question typed into it and the answer already found.
 *
 * Drawn because a screenshot of an application still being built goes stale
 * every week, and because this one stays sharp on a phone and in dark mode.
 */
function Still() {
  return (
    <div className="still" aria-hidden="true">
      <aside className="still-side">
        <span className="still-brand" />
        {["Home", "Library", "Collections", "Tasks"].map((row, i) => (
          <span key={row} className={i === 1 ? "still-row on" : "still-row"}>
            {row}
          </span>
        ))}
      </aside>
      <div className="still-main">
        <div className="still-ask">
          <span>that wine from dinner in the spring</span>
        </div>
        <ul className="still-hits">
          <li className="hit">
            <b>Occhipinti SP68</b>
            <i>the one from dinner · said 4 May</i>
            <em>92%</em>
          </li>
          <li className="hit">
            <b>Dinner, Brera</b>
            <i>note · 4 May</i>
            <em>71%</em>
          </li>
          <li className="hit">
            <b>Frappato, generally</b>
            <i>page kept · 6 May</i>
            <em>64%</em>
          </li>
        </ul>

        {/* The strip along the bottom of the window, which is where the
            application says what it is holding and where it is holding it. */}
        <div className="still-foot">
          <span>412 captures</span>
          <span>indexed on this machine</span>
          <span className="still-dot" />
          <span>nothing queued to upload</span>
        </div>
      </div>
    </div>
  );
}
