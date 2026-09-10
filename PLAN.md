# A four-cube mark, a spinning tetrahedron loader, and the public site

- scripts/gen_icons.py — render the mark (three cubes in a triangle, one
  centred, edges between them) in place of the four bars.
- apps/desktop/src/components/Logo.tsx — `Logo`, the flat mark; `Loader`, the
  same four cubes as a tetrahedron rotated in 3D and projected each frame.
- apps/desktop/src/hub/App.tsx — the boot blank becomes the loader.
- apps/desktop/src/features/home/Home.tsx — the loader beside "loading the
  model…".
- apps/desktop/src/features/account/SignIn.tsx — the mark above the title, the
  loader in the button while waiting.
- apps/desktop/src/styles/hub.css — sizing for both.
- apps/web/ — Vite + React landing site: hero, demo video, feature cards, a
  recall chart, CTA, plus /terms and /privacy.
