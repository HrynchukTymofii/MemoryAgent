# Rest the pill at the top, in black and white

- apps/desktop/overlay.html — one monochrome palette as variables, light and
  dark by `prefers-color-scheme`; capsule radius everywhere; shorter pill
- apps/desktop/src-tauri/src/main.rs — default resting position moves to
  top-centre; `rest_state` reports top when nothing was ever dragged;
  `save_pill_anchor` flips its threshold to match the new resting edge
