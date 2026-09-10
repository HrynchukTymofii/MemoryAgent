# The public site

Three static pages: the landing page, the terms, and the privacy policy.

```
npm install
npm run dev      # http://localhost:5173
npm run build    # → dist/, ready for any static host
```

Vite builds three documents rather than one application with a router, so
`/terms/` and `/privacy/` are real files and need no server rewrites.

## Before it goes live

- **The name.** Everything reads it from `src/brand.ts` — name, legal entity,
  contact address, and the date on both legal pages. The only copies outside
  that file are the `<title>` tags in `index.html`, `terms/index.html` and
  `privacy/index.html`, because a document title exists before any script runs.
- **The walkthrough.** The demo frame shows a drawn still until a file called
  `demo.mp4` exists in `public/`. Drop one in and the same frame plays it; no
  code changes.
- **The download link.** The button points at `/download/windows`, which does
  not exist yet — wire it to wherever the installer ends up.
- **The governing-law clause** in `src/terms.tsx` is marked unfinished, and the
  legal pages have not been through a lawyer.

## The mark

`src/components/Logo.tsx` is a copy of the desktop app's component of the same
name, and `public/mark.svg` is the same figure again as a favicon. All three
carry the same numbers; change one and change the others.
