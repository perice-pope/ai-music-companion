# Spec: Reveal S2 — LLM-enriched "why" (grounded, opt-in) (#253)

> Slice 2 of the Reveal loop (#253). S1 shipped curated reveals; S2 lets the LLM reword the *why*
> line when the user has opted into online coaching — the artist/piece stays grounded in the table.

## 1. Summary
When online coaching is enabled, a reveal's `why` may be reworded by the Claude API into something
warmer/more engaging, while the `connection` (artist/piece) still comes verbatim from the curated
table. Any failure or offline state falls back to the curated `why`. Reuses the existing coaching
LLM client + airplane switch; adds no new endpoint.

## 2. Problem / why
S1's curated `why` lines are fixed and a little dry. The founder wants reveals to feel alive. But a
kids' tool must never invent a wrong artist/piece — so only the *why* is model-generated, and only
when the user has already opted into the same online coaching that powers tips.

## 3. Non-goals
- No new outbound endpoint, API key, or opt-in — reuse coaching's `NetworkPolicy` + key.
- The model must **not** change `concept` or `connection`; it only rewrites `why`.
- No change to S1 selection, cadence, or the S3 collection.

## 4. Contract / interface
`brain::coaching::CoachingEngine`:
```rust
/// Reword the `why` for a grounded reveal. Returns None offline / on any failure
/// (never fabricates). The model is told the concept + the fixed real-world
/// connection and asked only for one short sentence of "why".
pub async fn enrich_reveal_why(&mut self, concept: &str, connection: &str, curated_why: &str)
    -> Option<String>;
```
`CoachingService` (commands.rs) gains:
```rust
/// Enrich a grounded reveal's `why` when online; return it unchanged otherwise.
/// Never alters `concept`/`connection`. Default impl (mock) returns input as-is.
async fn enrich_reveal(&self, reveal: Reveal) -> Reveal { reveal }
```
`AppState::enrich_reveal` delegates to the service; the `get_reveal` command calls it on the
S1 `reveal_for` result. On success the returned `Reveal` has the new `why` and
`source == RevealSource::LlmGrounded`.

## 5. Acceptance criteria (numbered, testable)
1. `enrich_reveal_why` returns `None` when the engine is **offline** and makes **no** HTTP call
   (assert the client is not invoked).
2. When online and the API returns a valid `why`, `enrich_reveal_why` returns `Some(that_why)`.
3. On API error or unparseable response, `enrich_reveal_why` returns `None` (fallback).
4. `CoachingService::enrich_reveal`: online → returns a Reveal whose `why` is the enriched text,
   `source == LlmGrounded`, and whose `concept`/`connection` are **unchanged**; offline/none →
   returns the input Reveal unchanged (`source` stays `Grounded`).
5. The mock service's `enrich_reveal` returns the input unchanged (no network in tests/preview).

## 6. Edge cases & failure modes
- Offline (default) → curated `why`, `Grounded`, no call.
- Model returns an empty/whitespace `why` → treat as failure → fallback.
- Model tries to change the connection → we ignore it; we only read the `why` field and keep the
  original `connection`/`concept` by construction (we never re-read them from the response).
- Enrichment latency: reveals are already ≤1 per ~3 phrases, so no extra rate limiting needed.

## 7. Test plan
| AC | Test | Asserts |
|---|---|---|
| AC1 | `coaching::tests::enrich_reveal_why_offline_makes_no_call` | offline → None, client untouched |
| AC2 | `coaching::tests::enrich_reveal_why_online_returns_why` | mock 200 → Some(why) |
| AC3 | `coaching::tests::enrich_reveal_why_api_error_returns_none` | error/garbage → None |
| AC4 | `commands::tests::enrich_reveal_online_replaces_why_keeps_connection` | why+source change, connection fixed |
| AC5 | `commands::tests::mock_enrich_reveal_is_identity` | mock returns input |

## 8. Architecture / approach
`enrich_reveal_why` mirrors `get_tip`: airplane-switch check first (no path from Offline → call),
build a grounded prompt (concept + connection given as fixed facts; ask for JSON `{ "why": "…" }`),
`post_json`, parse, fallback to `None` on any error. The app-layer `enrich_reveal` assembles the new
`Reveal` (keeping `connection`/`concept`, swapping `why`, flipping `source`). Offline-first: gated by
the existing coaching opt-in/`NetworkPolicy`; **disclosed** — reveals now also use the coaching LLM,
recorded in `docs/architecture/offline-first-and-network-transparency.md` and surfaced in
`ConnectionsPrivacy.tsx` (same endpoint/key as coaching, so no new destination).

## 9. Slice breakdown
Single slice: engine method + service method + command wiring + disclosure + tests.

## 10. Risks / open questions
- Prompt could still produce an off-tone line; mitigated by grounding + short-sentence instruction +
  fallback. Quality tuned later.

## 11. References
- `docs/specs/253-reveal-loop.md` (S2 row), `crates/brain/src/coaching.rs` (get_tip pattern),
  `apps/desktop/src-tauri/src/commands.rs` (`CoachingService`, `get_reveal`), `connections::Reveal`.
