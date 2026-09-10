import { useEffect, useRef, useState } from "react";

/**
 * The two things the page needs to know about scrolling, and nothing else.
 *
 * No animation library. The site has three moving parts — a looping demo, a
 * section that fades as it passes, and a chart that draws itself once — and a
 * general-purpose motion runtime to drive three effects would weigh more than
 * the rest of the site put together.
 */

/** Whether this visitor has asked for less movement. Read once, at import. */
export const still =
  typeof window !== "undefined" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * True once the element has been seen, and true from then on.
 *
 * Reveals do not replay. Scrolling back up to re-trigger an animation is a
 * thing pages do to themselves, not something a reader ever asks for.
 */
export function useSeen<T extends Element>(margin = "0px 0px -15% 0px") {
  const ref = useRef<T>(null);
  const [seen, setSeen] = useState(still);

  useEffect(() => {
    const el = ref.current;
    if (!el || still) return;
    const io = new IntersectionObserver(
      ([e]) => {
        if (e.isIntersecting) {
          setSeen(true);
          io.disconnect();
        }
      },
      { rootMargin: margin },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [margin]);

  return [ref, seen] as const;
}

/**
 * How far an element has travelled across the viewport: 0 as its top reaches
 * the bottom of the screen, 1 as its bottom leaves the top.
 *
 * Measured on scroll behind a frame request, because the read of
 * `getBoundingClientRect` forces layout and a scroll event can fire far more
 * often than the screen refreshes.
 */
export function useTravel<T extends Element>() {
  const ref = useRef<T>(null);
  const [t, setT] = useState(0);

  useEffect(() => {
    const el = ref.current;
    if (!el || still) return;
    let raf = 0;
    const read = () => {
      raf = 0;
      const r = el.getBoundingClientRect();
      const span = window.innerHeight + r.height;
      setT(Math.max(0, Math.min(1, (window.innerHeight - r.top) / span)));
    };
    const onScroll = () => {
      if (!raf) raf = requestAnimationFrame(read);
    };
    read();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    return () => {
      if (raf) cancelAnimationFrame(raf);
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
    };
  }, []);

  return [ref, t] as const;
}

/**
 * A step that advances on a timer and starts over — the hero's capture, played
 * again for whoever arrives mid-loop.
 *
 * Stops entirely under reduced motion, parked on the last step, which is the
 * one that shows the finished result.
 */
export function useLoop(durations: number[]) {
  const [step, setStep] = useState(still ? durations.length - 1 : 0);

  useEffect(() => {
    if (still) return;
    const id = window.setTimeout(
      () => setStep((s) => (s + 1) % durations.length),
      durations[step],
    );
    return () => window.clearTimeout(id);
  }, [step, durations]);

  return step;
}
