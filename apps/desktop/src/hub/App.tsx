import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { listen } from "@tauri-apps/api/event";

import {
  api,
  CAPTURE_LIMIT,
  type Account as AccountData,
  type HookStats,
  isPro,
  type Entitlement,
  type LibrarySummary,
  type Notification,
} from "../lib/api";
import { Home } from "../features/home/Home";
import { Library } from "../features/library/Library";
import { Collections } from "../features/collections/Collections";
import { Tasks } from "../features/tasks/Tasks";
import { Account } from "../features/account/Account";
import { AccountMenu } from "../features/account/AccountMenu";
import { Referral } from "../features/referral/Referral";
import { SignIn } from "../features/account/SignIn";
import { Settings } from "../features/settings/Settings";
import { HelpMenu } from "../features/help/HelpMenu";
import { Notifications } from "../features/notifications/Notifications";
import { Toasts } from "../features/notifications/Toast";
import { TitleBar } from "../components/TitleBar";
import { Logo, Loader } from "../components/Logo";
import {
  CollectionsIcon,
  GiftIcon,
  DictionaryIcon,
  HistoryIcon,
  HomeIcon,
  LibraryIcon,
  SettingsIcon,
  TasksIcon,
} from "../components/icons";

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
  /** The sidebar is a drawer; the title bar's first button opens and shuts it. */
  const [drawer, setDrawer] = useState(true);
  const [bell, setBell] = useState(false);
  const [unread, setUnread] = useState(0);
  /** The two things the title bar's account button and the sidebar open. */
  const [acct, setAcct] = useState(false);
  const [referral, setReferral] = useState(false);
  const [plan, setPlan] = useState<Entitlement | null>(null);
  /** Bumped when a milestone lands, so the open panel re-fetches at once. */
  const [notes, setNotes] = useState(0);
  /** What just arrived, for the toast. Cleared as each one times out. */
  const [earned, setEarned] = useState<Notification[]>([]);

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
        const [count, sum, stats, tasks, waiting] = await Promise.all([
          api.captureCount(),
          api.summary(),
          api.hookStats(),
          api.openTaskCount(),
          api.unreadNotifications(),
        ]);
        if (!live) return;
        setUsed(count);
        setSummary(sum);
        setHook(stats);
        setOpenTasks(tasks);
        setUnread(waiting);
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

  // The backend announces milestones as they land. Without this the badge
  // waits on the poll, which means a congratulation arrives up to a second
  // after the thing being congratulated — long enough to read as unrelated.
  useEffect(() => {
    const un = listen<Notification[]>("notification:new", (e) => {
      setEarned(e.payload);
      setUnread((n) => n + e.payload.length);
      setNotes((n) => n + 1);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  /**
   * Re-read the plan, and let the API pay out a referral if one is due.
   *
   * Called on launch and when the referral sheet is shut — not on a timer. This
   * is a number that changes twice a year, and polling it would be a request
   * per user per interval to discover that nothing had happened.
   */
  const refreshPlan = useCallback(async () => {
    try {
      setPlan(await api.entitlement());
      setPlan(await api.refreshEntitlement());
    } catch {
      // No API, or nobody signed in. The cached plan stands, which is the
      // entire point of caching it.
    }
  }, []);

  useEffect(() => {
    void refreshPlan();
  }, [refreshPlan]);

  const openCollection = useCallback((path: string | null) => {
    setFilter(path);
    setPage("library");
  }, []);

  /** Every navigation also shuts the bell, which is a popover over the page. */
  const go = useCallback((p: Page) => {
    setPage(p);
    setBell(false);
  }, []);

  const quota = useMemo(
    () => Math.min(100, (used / CAPTURE_LIMIT) * 100),
    [used],
  );

  // Held back until the question is answered — see `askToSignIn`. A blank
  // window for as long as that takes reads as a hang, so the mark turns.
  if (askToSignIn === null) {
    return (
      <div className="shell">
        <TitleBar bare />
        <div className="boot">
          <Loader size={56} />
        </div>
      </div>
    );
  }
  if (askToSignIn && signInAccount) {
    return (
      <div className="shell">
        {/* Chrome only — but it has to be here, or the first screen anyone
            sees is a window with no way to move or close it. */}
        <TitleBar bare />
        <SignIn
          account={signInAccount}
          onDone={() => setAskToSignIn(false)}
          onSignedIn={() => refresh()}
        />
      </div>
    );
  }

  return (
    <div className={drawer ? "shell" : "shell shut"}>
      <TitleBar
        drawerOpen={drawer}
        onToggleDrawer={() => setDrawer((d) => !d)}
        onAccount={() => setAcct((a) => !a)}
        accountOn={acct || page === "account"}
        notifications={unread + openTasks + (offline ? 1 : 0)}
        onNotifications={() => setBell((b) => !b)}
        bellOn={bell}
      />

      {bell && (
        <Notifications
          openTasks={openTasks}
          offline={offline}
          revision={notes}
          onGo={(p) => {
            if (p === "referral") {
              setBell(false);
              setReferral(true);
            } else {
              go(p as Page);
            }
          }}
          onClose={() => setBell(false)}
          onRead={() => setUnread(0)}
        />
      )}

      {acct && (
        <AccountMenu
          used={used}
          onClose={() => setAcct(false)}
          onManage={() => {
            setAcct(false);
            go("account");
          }}
          onRefer={() => {
            setAcct(false);
            setReferral(true);
          }}
        />
      )}

      {referral && (
        <Referral
          onClose={() => {
            setReferral(false);
            void refreshPlan();
          }}
          onSignIn={() => {
            setReferral(false);
            go("account");
          }}
        />
      )}

      <Toasts
        earned={earned}
        onGo={(p) => {
          setEarned([]);
          if (p === "referral") setReferral(true);
          else go(p as Page);
        }}
      />

      <div className="body">
        <aside className="side">
          <div className="brand">
            <Logo size={22} />
            <span className="lbl">Memory OS</span>
          </div>
          <nav>
            <ul>
              <NavItem page="home" current={page} onGo={go} icon={<HomeIcon />} label="Home" />
              <NavItem
                page="library"
                current={page}
                onGo={(p) => {
                  setFilter(null);
                  go(p);
                }}
                icon={<LibraryIcon />}
                label="Library"
              />
              <NavItem
                page="collections"
                current={page}
                onGo={go}
                icon={<CollectionsIcon />}
                label="Collections"
              />
              <NavItem
                page="tasks"
                current={page}
                onGo={go}
                icon={<TasksIcon />}
                label="Tasks"
                // Only when there is something to do. A badge that sits at zero
                // is a permanent request for attention that has nothing to say.
                badge={openTasks > 0 ? openTasks : undefined}
              />
            </ul>
            <div className="navh">Tools</div>
            <ul>
              <li className="muted" title="Dictionary">
                <span className="g">
                  <DictionaryIcon />
                </span>
                <span className="lbl">Dictionary</span>
              </li>
              <li className="muted" title="History">
                <span className="g">
                  <HistoryIcon />
                </span>
                <span className="lbl">History</span>
              </li>
            </ul>
          </nav>

          {/* The foot of the drawer: the plan first, then the settings it leads
              to. Account is not here — it is the second button in the title
              bar, next to the one that hides this drawer. */}
          <div className="foot">
            {/* Pro has no limit rather than a larger one, so there is no bar
                to draw: the meter becomes the one sentence that is true. */}
            {isPro(plan) ? (
              <div className="meter pro">
                <div className="n">Pro</div>
                <div className="s">
                  Unlimited captures
                  {plan?.pro_until &&
                    ` · until ${new Date(plan.pro_until).toLocaleDateString()}`}
                </div>
              </div>
            ) : (
              <div className="meter">
                <div className="n">
                  {used} of {CAPTURE_LIMIT} captures
                </div>
                <div className="s">Free plan · resets Monday</div>
                <div className="bar">
                  <i style={{ width: `${quota}%` }} />
                </div>
              </div>
            )}
            <nav>
              <ul>
                {/* Above Settings, and not a page: it opens the modal. Worth a
                    permanent row rather than only living in the account menu —
                    it is the one thing here a user can act on that costs them
                    nothing. */}
                <li className="gift-row" onClick={() => setReferral(true)}>
                  <span className="g">
                    <GiftIcon />
                  </span>
                  <span className="lbl">Get a free month</span>
                </li>
                <NavItem
                  page="settings"
                  current={page}
                  onGo={go}
                  icon={<SettingsIcon />}
                  label="Settings"
                />
              </ul>
            </nav>
            {/* Below Settings, and last of everything: it is the row you reach
                for when the rest of the sidebar has not answered you. */}
            <HelpMenu railed={!drawer} />
          </div>
        </aside>

        <main>
          {page === "home" && (
            <Home summary={summary} offline={offline} revision={revision} onGo={go} />
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
    </div>
  );
}

function NavItem({
  page,
  current,
  onGo,
  icon,
  label,
  badge,
}: {
  page: Page;
  current: Page;
  onGo: (p: Page) => void;
  icon: React.ReactNode;
  label: string;
  badge?: number;
}) {
  return (
    <li
      className={page === current ? "on" : ""}
      data-page={page}
      onClick={() => onGo(page)}
      // Collapsed to the rail there is no label to read, and the icon is all
      // there is to go on. Kept when expanded too rather than swapped in and
      // out: a tooltip that only sometimes appears is its own surprise.
      title={label}
    >
      <span className="g">{icon}</span>
      <span className="lbl">{label}</span>
      {badge !== undefined && <span className="badge">{badge}</span>}
    </li>
  );
}
