import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  api,
  CAPTURE_LIMIT,
  type Account as AccountData,
  type HookStats,
  type LibrarySummary,
} from "../lib/api";
import { Home } from "../features/home/Home";
import { Library } from "../features/library/Library";
import { Collections } from "../features/collections/Collections";
import { Tasks } from "../features/tasks/Tasks";
import { Account } from "../features/account/Account";
import { SignIn } from "../features/account/SignIn";
import { Settings } from "../features/settings/Settings";

export type Page = "home" | "library" | "collections" | "tasks" | "account" | "settings";

/**
 * How often the shell polls the backend.
 *
 * Two rates, not one. The shortcut recorder reads the keyboard hook's *peak*
 * held state, so it needs samples faster than a person can press and release a
 * chord — miss the peak and Ctrl+Win records as Ctrl. Nothing else on screen
 * changes faster than a person reads, and polling eight IPC calls at 120 ms
 * forever is a background cost paid by an app that is meant to sit in the tray
 * all day.
 */
const FAST_MS = 120;
const CALM_MS = 1000;

export function App() {
  const [page, setPage] = useState<Page>("home");
  // Set when a collection is picked in Collections; the Library opens filtered.
  const [filter, setFilter] = useState<string | null>(null);

  const [used, setUsed] = useState(0);
  const [summary, setSummary] = useState<LibrarySummary | null>(null);
  const [hook, setHook] = useState<HookStats | null>(null);
  const [openTasks, setOpenTasks] = useState(0);
  // The count as of the previous poll. Tasks are the one thing on screen that
  // arrives from outside the Hub — you speak one while the window is open — so
  // a change in the count is the signal that the list behind it is stale.
  const seenTasks = useRef<number | null>(null);
  /**
   * Whether to show the sign-in screen instead of the shell.
   *
   * `null` while we are still asking, so the Hub does not flash its Home page
   * for a frame before replacing it with a sign-in — which reads as a glitch
   * rather than as a first run.
   */
  const [askToSignIn, setAskToSignIn] = useState<boolean | null>(null);
  const [signInAccount, setSignInAccount] = useState<AccountData | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const [seen, account] = await Promise.all([api.signInPromptSeen(), api.account()]);
        setSignInAccount(account);
        // Only when there is something to sign in to — either provider counts
        // — and only when the question has never been answered.
        setAskToSignIn(
          (account.available || account.email_available) && !account.signed_in && !seen,
        );
      } catch {
        setAskToSignIn(false);
      }
    })();
  }, []);
  const [offline, setOffline] = useState(false);
  /** Bumped after anything that changes the store, to re-fetch lists. */
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision((r) => r + 1), []);

  // The recorder is the only fast consumer, and it lives on Settings.
  const interval = page === "settings" ? FAST_MS : CALM_MS;

  useEffect(() => {
    let live = true;
    const tick = async () => {
      try {
        const [count, sum, stats, tasks] = await Promise.all([
          api.captureCount(),
          api.summary(),
          api.hookStats(),
          api.openTaskCount(),
        ]);
        if (!live) return;
        setUsed(count);
        setSummary(sum);
        setHook(stats);
        setOpenTasks(tasks);
        if (seenTasks.current !== null && seenTasks.current !== tasks) refresh();
        seenTasks.current = tasks;
        setOffline(false);
      } catch {
        // Never fail silently. When the backend stopped answering, every tile
        // kept its last value and read as a confident zero — indistinguishable
        // from a dead hook, and it sent us chasing a bug that did not exist.
        if (live) setOffline(true);
      }
    };
    void tick();
    const t = window.setInterval(tick, interval);
    return () => {
      live = false;
      window.clearInterval(t);
    };
  }, [interval, revision, refresh]);

  const openCollection = useCallback((path: string | null) => {
    setFilter(path);
    setPage("library");
  }, []);

  const quota = useMemo(
    () => Math.min(100, (used / CAPTURE_LIMIT) * 100),
    [used],
  );

  // Held back until the question is answered — see `askToSignIn`.
  if (askToSignIn === null) return null;
  if (askToSignIn && signInAccount) {
    return (
      <SignIn
        account={signInAccount}
        onDone={() => setAskToSignIn(false)}
        onSignedIn={() => refresh()}
      />
    );
  }

  return (
    <div className="shell">
      <aside className="side">
        <div className="brand">
          <span className="bars" aria-hidden="true">
            <i style={{ height: 7 }} />
            <i style={{ height: 13 }} />
            <i style={{ height: 10 }} />
            <i style={{ height: 15 }} />
          </span>
          Memory OS
        </div>
        <nav>
          <ul>
            <NavItem page="home" current={page} onGo={setPage} glyph="◈" label="Home" />
            <NavItem
              page="library"
              current={page}
              onGo={(p) => {
                setFilter(null);
                setPage(p);
              }}
              glyph="▤"
              label="Library"
            />
            <NavItem
              page="collections"
              current={page}
              onGo={setPage}
              glyph="◱"
              label="Collections"
            />
            <NavItem
              page="tasks"
              current={page}
              onGo={setPage}
              glyph="◷"
              label="Tasks"
              // Only when there is something to do. A badge that sits at zero
              // is a permanent request for attention that has nothing to say.
              badge={openTasks > 0 ? openTasks : undefined}
            />
          </ul>
          <div className="navh">Tools</div>
          <ul>
            <li className="muted">
              <span className="g">Aa</span> Dictionary
            </li>
            <li className="muted">
              <span className="g">↺</span> History
            </li>
            <NavItem page="account" current={page} onGo={setPage} glyph="◍" label="Account" />
            <NavItem page="settings" current={page} onGo={setPage} glyph="⚙" label="Settings" />
          </ul>
        </nav>
        <div className="meter">
          <div className="n">
            {used} of {CAPTURE_LIMIT} captures
          </div>
          <div className="s">Free plan · resets Monday</div>
          <div className="bar">
            <i style={{ width: `${quota}%` }} />
          </div>
        </div>
      </aside>

      <main>
        {page === "home" && (
          <Home summary={summary} offline={offline} revision={revision} onGo={setPage} />
        )}
        {page === "library" && (
          <Library
            filter={filter}
            onFilter={setFilter}
            revision={revision}
            onChanged={refresh}
          />
        )}
        {page === "collections" && (
          <Collections revision={revision} onOpen={openCollection} />
        )}
        {page === "tasks" && <Tasks revision={revision} onChanged={refresh} />}
        {page === "account" && <Account revision={revision} onChanged={refresh} />}
        {page === "settings" && <Settings hook={hook} offline={offline} />}
      </main>
    </div>
  );
}

function NavItem({
  page,
  current,
  onGo,
  glyph,
  label,
  badge,
}: {
  page: Page;
  current: Page;
  onGo: (p: Page) => void;
  glyph: string;
  label: string;
  badge?: number;
}) {
  return (
    <li
      className={page === current ? "on" : ""}
      data-page={page}
      onClick={() => onGo(page)}
    >
      <span className="g">{glyph}</span> {label}
      {badge !== undefined && <span className="badge">{badge}</span>}
    </li>
  );
}
