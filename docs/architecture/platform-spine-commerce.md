# Platform Spine — Commerce & Entitlements

**Companion doc to:** [`architecture-v2.md`](./architecture-v2.md) and [`platform-modules-addendum.md`](./platform-modules-addendum.md)
**Status:** Draft
**Date:** June 7, 2026

---

## Purpose

This is one of **three shared spines** the platform is built on. It is the **commerce / entitlements spine**: the single mechanism by which the platform sells and grants access to *anything* — audition packs (Module 1), teacher curricula with 70/30 marketplace splits (Module 4), and B2B seat licensing (Module 5). The other two spines are written elsewhere: **Unified Content Format** (what gets sold) and **Personalization / Cross-Genre** (how it's tailored). This doc defines **one entitlements model** that all paid surfaces gate against — no per-module billing silos.

---

## Layman's overview

> Picture a single turnstile in front of everything the platform offers. It doesn't matter whether you bought a one-off audition pack, subscribed monthly, bought a teacher's "16 Weeks to All-State" course, or your school bought 30 seats — you walk up, the turnstile checks one list of "what this person is allowed to use," and lets you through or not. There is exactly one turnstile and one list. Stripe handles taking the money; our job is only to keep that one list correct and to check it in the Rust core where it can't be faked. Teachers selling on the marketplace get paid through Stripe's own payout rails (Connect) — we don't run a bank.

---

## The entitlements model

The whole spine is **two ideas**: *products* (what's for sale) and *entitlements* (who may use what). Everything else — purchases, subscriptions, splits, seats — is a way an entitlement row gets created or expired. We resist the temptation to model four commerce systems; there is one.

### Data model (Supabase / Postgres, extends the existing schema)

Tables follow the conventions already shipped in `supabase/migrations/` (uuid PKs, `auth.users` / `profiles` references, RLS enabled, `(select auth.uid())` policies).

```
products            (id, kind, sku, title, price_cents, currency,
                     stripe_price_id,
                     seller_id,          -- profiles.id of teacher/partner; NULL = first-party
                     payout_bps,         -- seller share in basis points (7000 = 70%); NULL = first-party
                     content_ref,        -- pointer into the Unified Content Format (pack/curriculum/SDK build)
                     active boolean)
        kind ∈ ('pack','subscription','curriculum','b2b_seatpack')

purchases           (id, buyer_id,       -- profiles.id (B2B: the org admin's account)
                     product_id,
                     stripe_object,      -- checkout session / subscription / invoice id
                     stripe_object_kind, -- 'checkout' | 'subscription' | 'invoice'
                     seat_count int default 1,   -- >1 only for b2b_seatpack
                     status,             -- 'active' | 'past_due' | 'canceled' | 'refunded'
                     created_at)

entitlements        (id, holder_id,      -- profiles.id of the person who may USE it
                     product_id,
                     purchase_id,        -- provenance; null for granted/comp
                     scope_ref,          -- the content this grants (mirrors products.content_ref)
                     expires_at,         -- NULL = perpetual (one-off pack); set = subscription/seat term
                     source,             -- 'purchase' | 'seat_assignment' | 'grant'
                     revoked_at)         -- soft-revoke for refunds / seat reclaim / B2B churn

teacher_payouts     (id, purchase_id,
                     seller_id,
                     gross_cents, seller_cents, platform_cents,
                     stripe_transfer_id, -- Stripe Connect transfer; we do not move money ourselves
                     status)             -- 'pending' | 'paid' | 'reversed'
```

**How one model covers all four cases** — the difference is entirely in how an `entitlements` row is born and when it dies:

| Sale type | `products.kind` | Entitlement created by | `expires_at` | Split? |
|---|---|---|---|---|
| One-off audition pack (M1) | `pack` | one webhook on checkout | NULL (perpetual) | no |
| Subscription (premium tier) | `subscription` | webhook on first invoice; refreshed each renewal | end of period | no |
| Teacher curriculum (M4) | `curriculum` | webhook on checkout → entitlement **+** `teacher_payouts` row | NULL or term | **yes, Connect** |
| B2B seat pack (M5) | `b2b_seatpack` | one `purchases` row (N seats); **one entitlement per assigned seat** | org term | per contract |

A B2B seat is not a new concept — it's just an `entitlement` whose `source = 'seat_assignment'` and whose `holder_id` is the seat occupant, created from the org's single `purchases` row. Reclaiming a seat is `revoked_at = now()`. This is the same revocation primitive the Teacher Dashboard already uses for unlinking (RLS closes instantly; see below).

### Access check, end-to-end

```
Buy                         Stripe                    Supabase                 Rust core (the gate)
 │  checkout (Stripe-hosted) │                          │                         │
 │ ────────────────────────► │  charge / subscribe      │                         │
 │                           │ ── webhook (signed) ───►  Edge Function            │
 │                           │                          │  verify sig             │
 │                           │                          │  upsert purchase        │
 │                           │                          │  insert entitlement     │
 │                           │                          │  (+ payout row for M4)  │
 │                                                       │                         │
 Later, opening paid content:                            │                         │
 Face ── Tauri IPC: "open pack X" ─────────────────────────────────────────────►  │
                                                         │ ◄── select entitlement ─┤  has_entitlement(holder, scope)?
                                                         │ ── row / none ────────► │  open content  OR  return Locked
```

1. **Checkout** happens on Stripe-hosted pages (we never touch card data). For marketplace items, checkout is created with the seller's connected account and an application fee = `100% − payout_bps`.
2. **Webhook → Edge Function** (Supabase Edge Function, server-side) is the *only* writer of `entitlements`. It verifies the Stripe signature, then writes the `purchases` row, the `entitlements` row(s), and — for M4 — the `teacher_payouts` row. Connect handles the actual transfer; we record its id.
3. **The gate lives in the Rust core** (`crates/brain`), not the frontend. A thin `has_entitlement(holder_id, scope_ref)` check queries `entitlements` (perpetual or unexpired, not revoked) before any paid content loads. IPC stays thin JSON: the Face asks "open X," the core answers content or `Locked`. Business logic stays in Rust per the project rules.

---

## Integration with existing layers

- **Ears:** none. Commerce never touches the real-time audio path. No allocations, no checks in the hot loop — entitlements are resolved at *content-open* time, never per-frame.
- **Brain (Rust core):** owns the gate. `has_entitlement` sits alongside the import pipeline / content loader: a `pack` / `curriculum` / `b2b` content load is preceded by one entitlement check. Offline, the core caches the holder's active entitlement rows in the existing SQLite session store (read-through), so paid content opened while online stays openable offline for the cached term — consistent with architecture-v2 §6 "no cloud dependency for the core practice loop."
- **Face:** renders Locked vs. Unlocked state and launches Stripe-hosted checkout in the system browser. **No price math, no split math, no entitlement decisions in TypeScript** — it only displays what the core reports.
- **Supabase / RLS:** entitlements ride the schema and RLS patterns already shipped (`profiles`, `sessions`, `teacher_student_links`). New tables reuse the exact idioms: RLS enabled, `(select auth.uid())` self-policies, seller/holder joins mirroring the `teacher_student_links` accepted-link join.
- **Stripe:** Checkout for purchases/subscriptions; **Connect** for marketplace payouts (the platform never custodies seller funds); Billing for subscription lifecycle. Webhooks are the system of record's only write path.

### RLS posture (reuses the shipped pattern)

```sql
-- A holder reads only their own entitlements (the profiles_select_own idiom).
create policy entitlements_select_own on public.entitlements
  for select using ((select auth.uid()) = holder_id);

-- A seller reads payouts for their own products (the teacher_student_links
-- "either party" idiom, narrowed to the seller).
create policy payouts_select_seller on public.teacher_payouts
  for select using ((select auth.uid()) = seller_id);

-- NOBODY writes entitlements over the client API. Writes happen only in the
-- webhook Edge Function via the service-role key (held server-side, never shipped
-- — same posture as handle_new_user / migration 0002). No insert/update policy
-- for anon or authenticated == no client-side grant path.
```

---

## Security / trust

- **Server-authoritative, always.** The Rust core gate is the truth; the frontend's Locked/Unlocked UI is a hint, never the enforcement. A tampered client can hide the lock icon and still not load content the core won't serve.
- **Entitlements are write-once from the webhook.** No client-facing insert/update policy exists on `entitlements` or `teacher_payouts`. The only writer is the signature-verified Edge Function using the service-role key, which (per the existing posture in the privacy doc §6 and migration 0002) never ships to the client.
- **Money truth is Stripe's.** We never compute balances or hold funds. Splits are an application fee on a Connect charge; payouts are Connect transfers. `teacher_payouts` is a *mirror* for display/audit, reconciled from Stripe, not an authority.
- **Refund / revoke is one primitive.** A `charge.refunded` / `customer.subscription.deleted` / seat-reclaim webhook sets `revoked_at` or `expires_at`; the next gate check fails closed. Same instantaneous, RLS-backed revocation the Teacher Dashboard uses for unlinking.
- **Fail closed.** Unknown product, expired/revoked entitlement, or no row → `Locked`. The free core practice loop is never gated, so a commerce outage never blocks practice.

---

## Phased delivery

Maps onto the addendum's roadmap; the spine arrives exactly when the first paid surface needs it (Phase 2) and is reused thereafter — nothing is built speculatively ahead of a module.

| Phase | Arch phase | Spine increment |
|---|---|---|
| **2 — Audition Prep** | Smart Import + Tone | `products` + `purchases` + `entitlements` (kinds `pack`, `subscription`); Stripe Checkout + Billing; webhook Edge Function; Rust core gate + SQLite entitlement cache. First customer: M1 audition packs and the premium tier. |
| **4 — Marketplace** | Teacher Platform | Add `teacher_payouts` + Stripe **Connect**; `curriculum` kind; seller onboarding on the existing Teacher Dashboard. Pure addition — the gate and entitlements table are unchanged. |
| **5 — B2B + Scale** | Teacher Platform+ | `b2b_seatpack` kind + seat-assignment entitlements (one `purchases` row → N entitlements). License management = the same tables, queried by org. No parallel licensing system. |

---

## What we are deliberately NOT building

- **No custom billing engine.** No invoicing, dunning, proration, tax, or card vaulting of our own — Stripe Billing/Checkout/Tax own all of it. We store ids and mirrors, not money state.
- **No payout custody / wallet.** We never hold seller funds. Marketplace money moves seller-direct via Connect with an application fee; we are not a money transmitter.
- **No in-app currency, points, coins, or tokens.** (Also: no gamification — consistent with architecture-v2 §8.)
- **No crypto, NFTs, or on-chain anything.**
- **No second entitlements system per module.** M1/M4/M5 do not get their own access tables. One `entitlements` table or it doesn't ship.
- **No client-side commerce logic.** No price/split/grant math in TypeScript; the Face displays, the core decides.
- **No DRM beyond the server gate.** Content opened legitimately is usable offline for its term; we gate access, we don't fight the user's own device.
- **No per-seat real-time license heartbeat.** Seats are entitlement rows checked at content-open, not a phone-home daemon.

---

## Open questions

1. **Subscription ↔ pack interaction.** Does the premium subscription implicitly entitle some/all `pack` content, or are packs always separately owned? Affects whether the gate checks one scope or a subscription-implies-scope rule.
2. **B2B account shape.** Org/admin identity isn't modeled yet (`profiles` is student/teacher). Do we add an `org` role + membership table, or model the admin as a `teacher`-like buyer with seat assignment? (Smallest seam preferred.)
3. **Payout reconciliation cadence.** Is `teacher_payouts` reconciled live from Connect webhooks only, or also via a periodic Stripe sweep to catch reversals/disputes?
4. **Gift / grant / institutional comp.** `source='grant'` exists in the model — what's the authorized path to create one without opening a client write hole? (Likely an admin-only Edge Function.)
5. **Refund window vs. perpetual packs.** For perpetual `pack` entitlements, do we hard-revoke on refund, or grace-period? Interacts with the offline SQLite cache TTL.
6. **Tax/VAT surface** for international and B2B — defer to Stripe Tax, but confirm before any non-US sale (mirrors the residency flag in the privacy doc §6).
```