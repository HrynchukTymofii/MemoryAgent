import { useEffect, useState } from "react";

import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * The window's own chrome.
 *
 * The main window is undecorated, so this strip is the title bar: it carries
 * the drag region and the minimise/maximise/close buttons, and it is drawn on
 * the same ground as the sidebar so the Hub reads as one surface rather than an
 * application sitting inside a system frame.
 *
 * The app controls live here too — the drawer toggle, the account, the bell —
 * because a second bar under this one to hold three buttons is the seam the
 * whole arrangement exists to remove.
 */
export function TitleBar({
  bare,
  drawerOpen,
  onToggleDrawer,
  onAccount,
  accountOn,
  notifications = 0,
  onNotifications,
  bellOn,
}: {
  /**
   * Chrome only — no drawer, no account, no bell.
   *
   * The sign-in screen has none of those to offer, but it is still shown in an
   * undecorated window, and a window with no way to move or close it is not a
   * screen anyone should be shown first.
   */
  bare?: boolean;
  drawerOpen?: boolean;
  onToggleDrawer?: () => void;
  onAccount?: () => void;
  accountOn?: boolean;
  /** How many things want attention; the bell only marks itself above zero. */
  notifications?: number;
  onNotifications?: () => void;
  bellOn?: boolean;
}) {
  const [maximized, setMaximized] = useState(false);

  // The button has to say which of the two states it will produce, and the
  // window can be maximised by a double-click on this very bar or by Windows'
  // own snap, neither of which comes through our click handler.
  useEffect(() => {
    const w = getCurrentWindow();
    let live = true;
    const sync = async () => {
      try {
        const m = await w.isMaximized();
        if (live) setMaximized(m);
      } catch {
        // An undecorated window that cannot answer is not worth a broken bar.
      }
    };
    void sync();
    const un = w.onResized(() => void sync());
    return () => {
      live = false;
      void un.then((f) => f());
    };
  }, []);

  const w = getCurrentWindow();

  return (
    <header className="titlebar" data-tauri-drag-region>
      {!bare && (
        <div className="tb-left">
          <button
            type="button"
            className="tb-btn"
            onClick={onToggleDrawer}
            title={drawerOpen ? "Collapse the sidebar" : "Expand the sidebar"}
            aria-label={drawerOpen ? "Collapse the sidebar" : "Expand the sidebar"}
            aria-expanded={drawerOpen}
          >
            <SidebarIcon />
          </button>
          <button
            type="button"
            className={accountOn ? "tb-btn on" : "tb-btn"}
            onClick={onAccount}
            title="Account"
            aria-label="Account"
          >
            <AccountIcon />
          </button>
        </div>
      )}

      {/* The drag region is the empty middle. Giving it the whole bar and
          letting the buttons opt out drags the window on a missed click. */}
      <div className="tb-drag" data-tauri-drag-region />

      <div className="tb-right">
        {!bare && (
          <button
            type="button"
            className={bellOn ? "tb-btn on" : "tb-btn"}
            onClick={onNotifications}
            title={notifications > 0 ? `${notifications} waiting` : "Notifications"}
            aria-label="Notifications"
          >
            <BellIcon />
            {notifications > 0 && <span className="tb-dot" aria-hidden="true" />}
          </button>
        )}

        <div className="tb-win">
          <button
            type="button"
            className="tb-wbtn"
            onClick={() => void w.minimize()}
            title="Minimise"
            aria-label="Minimise"
          >
            <svg viewBox="0 0 10 10" aria-hidden="true">
              <path d="M0 5h10" />
            </svg>
          </button>
          <button
            type="button"
            className="tb-wbtn"
            onClick={() => void w.toggleMaximize()}
            title={maximized ? "Restore" : "Maximise"}
            aria-label={maximized ? "Restore" : "Maximise"}
          >
            {maximized ? (
              <svg viewBox="0 0 10 10" aria-hidden="true">
                <path d="M2.5 2.5V.8h6.7v6.7H7.5" />
                <rect x="0.8" y="2.5" width="6.7" height="6.7" rx="1" />
              </svg>
            ) : (
              <svg viewBox="0 0 10 10" aria-hidden="true">
                <rect x="0.8" y="0.8" width="8.4" height="8.4" rx="1" />
              </svg>
            )}
          </button>
          <button
            type="button"
            className="tb-wbtn close"
            onClick={() => void w.close()}
            title="Close to the tray"
            aria-label="Close"
          >
            <svg viewBox="0 0 10 10" aria-hidden="true">
              <path d="M.9.9l8.2 8.2M9.1.9L.9 9.1" />
            </svg>
          </button>
        </div>
      </div>
    </header>
  );
}

function SidebarIcon() {
  return (
    <svg viewBox="0 0 18 18" className="ic" aria-hidden="true">
      <rect x="1.6" y="2.6" width="14.8" height="12.8" rx="2.6" />
      <path d="M6.6 2.6v12.8" />
    </svg>
  );
}

/* The shoulders end on the rim rather than past it. Drawn wide enough to read
   as a person and no wider — the arc used to run out either side of the circle,
   which at 17px looked like a mistake rather than a crop. */
function AccountIcon() {
  return (
    <svg viewBox="0 0 18 18" className="ic" aria-hidden="true">
      <circle cx="9" cy="9" r="7.2" />
      <circle cx="9" cy="7.4" r="2.6" />
      <path d="M5.1 14.8a4 4 0 0 1 7.8 0" />
    </svg>
  );
}

function BellIcon() {
  return (
    <svg viewBox="0 0 18 18" className="ic" aria-hidden="true">
      <path d="M4.4 12.6V7.9a4.6 4.6 0 0 1 9.2 0v4.7l1.2 1.6H3.2z" />
      <path d="M7.2 15.2a1.9 1.9 0 0 0 3.6 0" />
    </svg>
  );
}
