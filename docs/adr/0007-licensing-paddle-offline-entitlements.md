# ADR-0007: Paddle as merchant of record, with offline entitlement tokens

**Status:** Accepted (vendor pending verification) · **Date:** 2026-09-07
**Supersedes:** an earlier draft of this ADR that named Polar

## Context

The product is sold commercially. The vendor is based in Ukraine, where Stripe
does not offer merchant accounts, so direct Stripe integration is unavailable.

## Decision

### Vendor: Paddle, behind an adapter

Paddle is a merchant of record: it is the legal seller, so it handles EU VAT,
US sales tax and invoicing across every jurisdiction, and the vendor receives
payouts rather than operating a payment processor and a tax practice.

It is the most established option of the group. It has a long track record with
desktop software specifically, mature subscription handling, and proper dunning
for failed renewals — which matters more than it sounds, because involuntary
churn from expired cards is a meaningful share of lost revenue in consumer
subscription products.

Alternatives considered:

| Option | Verdict |
|--------|---------|
| Stripe direct | Unavailable for Ukrainian entities |
| Polar | Newer MoR; ships license keys as a primitive, which Paddle does not. Kept as the primary fallback |
| Creem | Similar model to Polar, smaller track record |
| Lemon Squeezy | Acquired by Stripe; new signups steered to Stripe, so it inherits the original problem |

### The licensing boundary

Paddle does **not** provide license keys or entitlement checking. Polar does.
This is the one real cost of the choice, and it is smaller than it appears,
because the offline entitlement design below is something we would build under
any vendor — a merchant-of-record's hosted license check is a network call, and
a network call cannot gate a local-first product.

The split is therefore:

```
Paddle owns          transaction, subscription state, renewals, dunning, tax
memos-license owns   key issuance, device activation, offline verification
```

Worth recording explicitly so the boundary is known before M4 rather than
discovered during it.

### Enforcement: signed offline entitlement tokens

Licensing must not break Principle 4. A local-first product that stops working
without network has broken its core promise.

```
purchase (Paddle)
  -> webhook to our API
  -> license key issued by us
  -> device activation (device-bound, N seats)
  -> signed entitlement token (Ed25519, short-lived)
  -> cached locally, verified offline against an embedded public key
  -> refreshed opportunistically; ~14-day grace period
```

Properties:

- **Offline-verifiable.** The client holds only a public key; no network call is
  needed to validate an unexpired token.
- **Grace period.** Roughly 14 days offline before degradation, so travel and
  flaky connections never lock a paying user out.
- **Degrades, never bricks.** An expired entitlement disables Pro features
  (cloud sync, Tier 2 cloud models, research agent, cloud STT). It never blocks
  access to data the user already captured. Holding a user's own memory hostage
  is unacceptable and would be the fastest possible route to a chargeback.

### Tiering

| Tier | Contents |
|------|----------|
| Free | 50 voice captures per week. Browsing, searching and exporting what is already saved stay unlimited forever. Account required. |
| Pro | Unlimited captures, sync, Tier 2 models, research agent, cloud STT, multi-device |

The meter counts **voice captures** — one shortcut press, one command, one
action, whatever it turned into. That is the unit the user actually experiences,
so the counter reads "32 of 50 captures" and needs no explanation.

Two properties are load-bearing:

- **Weekly, not monthly.** A monthly allowance is exhausted in days and then
  locks the user out for weeks, which is long enough for the habit to die — and
  a dead habit uninstalls rather than converting. Weekly reset keeps the wall
  never more than a few days from lifting.
- **The archive is never gated.** Running out restricts *new* capture only.
  Browsing, searching, opening and exporting existing memories stay unlimited on
  the free plan permanently. A product that locks people out of their own notes
  to extract a payment has stopped being a memory system.

The counter increments locally so capture still succeeds offline, and reconciles
on sync. This permits slight over-run during long offline stretches, which is the
correct way to be wrong.

## Open items

1. **Payout eligibility for a Ukrainian entity is not verified.** Confirm with
   Paddle directly before building M4.
2. **Onboarding review is a lead-time risk.** Paddle vets sellers before
   approving an account, and review can be slow and occasionally strict for new
   AI products. Start the application early, well before M4 is code-complete,
   and keep a Polar application warm as the fallback.

Both are why the integration sits behind a `BillingProvider` trait.

## Consequences

**Good**

- No VAT or sales-tax registration burden across dozens of jurisdictions.
- The most mature subscription and dunning handling of the options considered.
- The app works fully offline while remaining licensed.
- Vendor risk is contained behind one trait.

**Bad**

- We build and operate the licensing layer ourselves, including key issuance and
  a webhook endpoint that must be idempotent and replay-safe.
- MoR fees exceed raw payment-processing fees. Accepted as the cost of tax
  compliance not being a project.
- Approval is not guaranteed and is not on our schedule.
- Device-binding needs a stable device ID that survives hardware changes without
  becoming a fingerprinting concern. Use a salted install-time UUID stored
  locally, not hardware serials.

**Neutral**

- Entitlement checks are pure local function calls, so they add no latency to
  the capture path.
