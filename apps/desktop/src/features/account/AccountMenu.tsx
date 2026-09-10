import { useEffect, useState } from "react";

import {
  api,
  isPro,
  CAPTURE_LIMIT,
  type Account as AccountData,
  type Entitlement,
} from "../../lib/api";
import { GiftIcon } from "../../components/icons";

/**
 * The account popover, off the title bar.
 *
 * The button used to navigate to the Account page. It now opens this, because
 * the three things people actually want from an avatar — who am I signed in as,
 * what plan is that, and how do I get more of it — are one glance each, and
 * replacing whatever page they were reading to answer them is a poor trade.
 * "Manage account" is what still opens the page.
 */
export function AccountMenu({
  used,
  onClose,
  onManage,
  onRefer,
}: {
  used: number;
  onClose: () => void;
  onManage: () => void;
  onRefer: () => void;
}) {
  const [account, setAccount] = useState<AccountData | null>(null);
  const [plan, setPlan] = useState<Entitlement | null>(null);

  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const [who, held] = await Promise.all([api.account(), api.entitlement()]);
        if (!live) return;
        setAccount(who);
        setPlan(held);
      } catch {
        /* An account panel that cannot answer draws nothing rather than an error. */
      }
    })();
    return () => {
      live = false;
    };
  }, []);

  const pro = isPro(plan);
  const left = Math.max(0, CAPTURE_LIMIT - used);

  return (
    <>
      <div className="scrim" onClick={onClose} />
      <div className="acct" role="dialog" aria-label="Account">
        <div className="acct-who">
          <div className="acct-mark" aria-hidden="true">
            {initial(account)}
          </div>
          <div className="bd">
            <span className="t">
              {account?.display_name ?? account?.email ?? "Not signed in"}
            </span>
            {account?.email && account.display_name && <span className="s">{account.email}</span>}
          </div>
        </div>

        <div className="acct-row">
          <div className="bd">
            <span className="t">{pro ? "You are on Pro" : "You are on the free plan"}</span>
            <span className="s">
              {pro
                ? `Unlimited captures${planEnds(plan)}`
                : `${left} of ${CAPTURE_LIMIT} captures left this week`}
            </span>
          </div>
          {!pro && (
            <button type="button" className="btn primary" onClick={onRefer}>
              Get Pro
            </button>
          )}
        </div>

        <div className="acct-row">
          <div className="bd">
            <span className="t">
              {plan && plan.months_earned > 0
                ? `${plan.months_earned} month${plan.months_earned === 1 ? "" : "s"} earned so far`
                : "Get a free month of Pro"}
            </span>
            <span className="s">Refer friends, earn rewards</span>
          </div>
          <button type="button" className="btn" onClick={onRefer}>
            <span className="g">
              <GiftIcon />
            </span>
            Refer a friend
          </button>
        </div>

        <button type="button" className="acct-manage" onClick={onManage}>
          Manage account
        </button>
      </div>
    </>
  );
}

/** How long there is left, said only when it is worth saying. */
function planEnds(plan: Entitlement | null): string {
  if (!plan?.pro_until) return "";
  const days = Math.ceil((new Date(plan.pro_until).getTime() - Date.now()) / 86_400_000);
  // Beyond a couple of months the date is the useful form; below it, the
  // countdown is — "43 days left" is a number nobody does anything with.
  if (days > 60) return ` until ${new Date(plan.pro_until).toLocaleDateString()}`;
  return ` for ${days} more day${days === 1 ? "" : "s"}`;
}

/**
 * One letter, from whatever we have.
 *
 * A letter rather than an avatar image: the provider gives us a URL, fetching
 * it is a network request on every open, and a broken image where somebody's
 * face should be is worse than a monogram that always works.
 */
function initial(account: AccountData | null): string {
  const source = account?.display_name ?? account?.email ?? "";
  return source.trim().charAt(0).toUpperCase() || "·";
}
