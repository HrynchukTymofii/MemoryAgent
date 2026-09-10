import { useEffect, useRef } from "react";

import { BRAND } from "../brand";

/**
 * The mark, and the loader that is the same mark set turning.
 *
 * The geometry is a copy of `apps/desktop/src/components/Logo.tsx`, kept in
 * step by hand: the site and the application are separate builds with separate
 * dependency trees, and reaching across the workspace for one component would
 * tie the site's build to the desktop app's React version to save a file.
 * Change the numbers in one and change them in the other.
 *
 * Four cubes: three at the corners of a triangle and one in the middle, with a
 * line between every pair. Those six lines are not decoration — three corners
 * and a centre joined pairwise is a tetrahedron seen flat on, which is what the
 * loader spins. The still mark and the moving one are the same object, so the
 * application never shows two different logos.
 *
 * Everything is drawn from `cube()` rather than from a pasted path, because the
 * loader needs the same cube at a different size and position sixty times a
 * second, and a second copy of the geometry would drift from this one.
 */

/** The isometric projection, as a half-width and a half-depth. */
const W = 0.866;
const H = 0.5;

/**
 * The three visible faces of an isometric cube of radius `r` centred on x, y.
 *
 * `r` is half the cube's drawn height, so a cube reaches r above and below its
 * centre and 0.866r either side — the arithmetic the layout below relies on to
 * keep the corner cubes clear of the middle one.
 */
function cube(x: number, y: number, r: number) {
  const p = (dx: number, dy: number) => `${(x + dx).toFixed(2)},${(y + dy).toFixed(2)}`;
  return {
    top: [p(0, -r), p(W * r, -H * r), p(0, 0), p(-W * r, -H * r)].join(" "),
    left: [p(-W * r, -H * r), p(0, 0), p(0, r), p(-W * r, H * r)].join(" "),
    right: [p(0, 0), p(W * r, -H * r), p(W * r, H * r), p(0, r)].join(" "),
  };
}

/** Face shading. One light source: the top brightest, the left in shadow. */
const FACE = { top: 1, right: 0.66, left: 0.42 };

function Cube({ x, y, r, color }: { x: number; y: number; r: number; color?: string }) {
  const f = cube(x, y, r);
  const fill = color ?? "currentColor";
  return (
    <g>
      <polygon points={f.top} fill={fill} fillOpacity={FACE.top} />
      <polygon points={f.right} fill={fill} fillOpacity={FACE.right} />
      <polygon points={f.left} fill={fill} fillOpacity={FACE.left} />
    </g>
  );
}

/* The flat layout, on a 64 grid. The triangle's centre sits below the middle of
   the box, because the finished mark hangs further below that centre than above
   it and an icon is cropped to the square, not to the triangle. */
const C: [number, number][] = [
  [32, 17.84],
  [48.35, 46.16],
  [15.65, 46.16],
  [32, 36.72],
];
const R = 7.55;

/** Every pair, which for four corners is six lines. */
const PAIRS: [number, number][] = [
  [0, 1],
  [1, 2],
  [2, 0],
  [0, 3],
  [1, 3],
  [2, 3],
];

/**
 * The still mark.
 *
 * The colour comes from the text around it, so the mark can sit on any of the
 * grounds the Hub uses. Only the middle cube is fixed, in the accent: the thing
 * in the middle is the point of the picture.
 */
export function Logo({ size = 28, className }: { size?: number; className?: string }) {
  return (
    <svg
      viewBox="0 0 64 64"
      width={size}
      height={size}
      className={className}
      role="img"
      aria-label={BRAND.name}
    >
      <g stroke="currentColor" strokeOpacity={0.3} strokeWidth={1.75} strokeLinecap="round">
        {PAIRS.map(([a, b]) => (
          <line key={`${a}${b}`} x1={C[a][0]} y1={C[a][1]} x2={C[b][0]} y2={C[b][1]} />
        ))}
      </g>
      {/* The middle cube is drawn last: it is what the lines converge on, and it
          must not be cut into by a corner that happens to come after it. */}
      {C.slice(0, 3).map(([x, y]) => (
        <Cube key={`${x}-${y}`} x={x} y={y} r={R} />
      ))}
      <Cube x={C[3][0]} y={C[3][1]} r={R} color="var(--acc, #E8850C)" />
    </svg>
  );
}

/* ------------------------------------------------------------------ loader */

/** The tetrahedron, as four corners of a cube — the shortest way to write it. */
const V: [number, number, number][] = (
  [
    [1, 1, 1],
    [1, -1, -1],
    [-1, 1, -1],
    [-1, -1, 1],
  ] as [number, number, number][]
).map(([x, y, z]) => [x / Math.SQRT2, y / Math.SQRT2, z / Math.SQRT2] as [number, number, number]);

/** How far the eye is, in the same units. Near enough for depth to show. */
const EYE = 3.4;
/** A fixed lean, so the shape is never seen straight down its own axis. */
const TILT = 0.42;

/**
 * The mark turning: four cubes at the corners of a tetrahedron, rotated about
 * the vertical, projected, and painted back to front.
 *
 * Each frame is written straight onto the DOM nodes rather than through state.
 * At sixty frames a second a `setState` per frame re-renders whatever the
 * loader is inside — a page mid-load, which is the one moment the machine has
 * nothing spare.
 *
 * The depth is real: the corner nearest the eye is drawn larger and last, so
 * the shape reads as a solid turning rather than as four discs sliding over one
 * another. Under a reduced-motion setting the clock stops after the first
 * frame, which leaves the shape parked at an angle that shows all four cubes.
 */
export function Loader({ size = 44, className }: { size?: number; className?: string }) {
  const edges = useRef<SVGPathElement>(null);
  const stage = useRef<SVGGElement>(null);

  useEffect(() => {
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const half = size / 2;
    const reach = size * 0.3;
    const rad = size * 0.115;
    let raf = 0;
    let start = 0;

    const frame = (now: number) => {
      if (!start) start = now;
      const t = ((now - start) / 2600) * Math.PI * 2;

      // Turn about the vertical, then lean the whole thing towards the eye.
      const pts = V.map(([x, y, z]) => {
        const rx = x * Math.cos(t) + z * Math.sin(t);
        const rz = z * Math.cos(t) - x * Math.sin(t);
        const ry = y * Math.cos(TILT) - rz * Math.sin(TILT);
        const dz = y * Math.sin(TILT) + rz * Math.cos(TILT);
        const s = EYE / (EYE - dz);
        return { x: half + rx * reach * s, y: half + ry * reach * s, s, z: dz };
      });

      edges.current?.setAttribute(
        "d",
        PAIRS.map(
          ([a, b]) =>
            `M${pts[a].x.toFixed(2)} ${pts[a].y.toFixed(2)}L${pts[b].x.toFixed(2)} ${pts[b].y.toFixed(2)}`,
        ).join(""),
      );

      const g = stage.current;
      if (g) {
        // Read the nodes by the index they were created with before moving any
        // of them, then re-append them nearest last: appending is what decides
        // which cube covers which, and SVG has no z-index to do it with.
        const nodes = new Map<number, SVGGElement>();
        for (const n of Array.from(g.children) as SVGGElement[]) {
          nodes.set(Number(n.dataset.i), n);
        }
        const order = pts.map((_, i) => i).sort((a, b) => pts[a].z - pts[b].z);
        for (const i of order) {
          const node = nodes.get(i);
          if (!node) continue;
          const faces = cube(pts[i].x, pts[i].y, rad * pts[i].s);
          node.children[0].setAttribute("points", faces.top);
          node.children[1].setAttribute("points", faces.right);
          node.children[2].setAttribute("points", faces.left);
          g.appendChild(node);
        }
      }

      if (!still) raf = requestAnimationFrame(frame);
    };

    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [size]);

  return (
    <svg
      viewBox={`0 0 ${size} ${size}`}
      width={size}
      height={size}
      className={className ? `loader ${className}` : "loader"}
      role="status"
      aria-label="Working"
    >
      <path
        ref={edges}
        stroke="currentColor"
        strokeOpacity={0.28}
        strokeWidth={size * 0.045}
        strokeLinecap="round"
        fill="none"
      />
      <g ref={stage}>
        {V.map((_, i) => (
          <g key={i} data-i={i}>
            <polygon fill="currentColor" fillOpacity={FACE.top} />
            <polygon fill="currentColor" fillOpacity={FACE.right} />
            <polygon fill="currentColor" fillOpacity={FACE.left} />
          </g>
        ))}
      </g>
    </svg>
  );
}
