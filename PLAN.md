# Pin the hub shell to the window height so the sidebar stops growing

- apps/desktop/src/styles/hub.css — `.shell` gets a fixed `height:100vh` plus
  `overflow:hidden` instead of `min-height:100vh`, so the sidebar is exactly as
  tall as the window and `main` does the scrolling.
