# Unified window chrome, collapsible sidebar, simpler history rows

- apps/desktop/src-tauri/tauri.conf.json — main window: decorations off, larger
- apps/desktop/src-tauri/capabilities/default.json — drag/minimise/maximise/close
- apps/desktop/src/components/TitleBar.tsx — new: drawer toggle, account, bell,
  window buttons
- apps/desktop/src/hub/App.tsx — title bar, sidebar collapse, Account out of the
  nav list, Settings moved under the meter
- apps/desktop/src/components/ItemList.tsx — row is time plus text; metadata gone
- apps/desktop/src/styles/hub.css — chrome, collapse, new row shape, larger scale
