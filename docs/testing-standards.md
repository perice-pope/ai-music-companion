# Testing Standards — AI Music Companion

> Every test must answer: "What bug would this catch?"
> If you can't answer that, delete the test.

## The Golden Rule

**Tests exist to catch bugs, not to increase coverage numbers.**

A test that can never fail is worse than no test — it gives false confidence and slows CI.

## Before Writing a Test: The Checklist

Ask these 5 questions. If any answer is "no," rewrite or skip:

- [ ] **Would this test fail if I introduced a real bug?** Try mentally breaking the code. Does the test catch it?
- [ ] **Does this test check behavior, not implementation?** Refactoring internals should NOT break tests.
- [ ] **Is the assertion specific enough?** `assert!(result.is_ok())` is almost always too weak. What's inside the Ok?
- [ ] **Am I testing the code, or testing the test framework?** `expect(screen.getByText("X")).toBeDefined()` — getByText already throws if missing. The toBeDefined is testing vitest, not your code.
- [ ] **Could the production code be completely wrong and still pass?** If yes, your test is useless.

## Anti-Patterns (Don't Do These)

### 1. The Tautology Test
```rust
// BAD: This test can never fail
#[test]
fn it_works() {
    assert!(true);
}
```
**Fix:** Delete it. If you need a "crate loads" test, call a real function.

### 2. The Existence Check
```tsx
// BAD: getByText already throws if element is missing
// toBeDefined() adds zero value
it("renders", () => {
  render(<App />);
  expect(screen.getByText("Hello")).toBeDefined();
});
```
**Fix:** Use `getByText` for the assertion (it throws), or better — test what the component *does*, not that it *exists*.

```tsx
// GOOD: Tests actual behavior
it("shows backend response after successful ping", async () => {
  render(<App />);
  const el = await screen.findByTestId("backend-response");
  expect(el.textContent).toBe("Backend says: pong");
});
```

### 3. The String-Contains Serialization Test
```rust
// BAD: Checking that JSON contains a key name tells you nothing
// about whether the VALUE is correct
let json = serde_json::to_string(&score).unwrap();
assert!(json.contains("\"verdict\":\"green\""));
```
**Fix:** Roundtrip deserialize and check actual field values.

```rust
// GOOD: Actually validates the data survives serialization
let json = serde_json::to_string(&score).unwrap();
let parsed: NoteScore = serde_json::from_str(&json).unwrap();
assert_eq!(parsed.verdict, Verdict::Green);
assert_eq!(parsed.note_index, 0);
assert!((parsed.cents_deviation - (-5.0)).abs() < f64::EPSILON);
```

### 4. The Happy-Path-Only Test
```rust
// BAD: Only tests the success case
#[test]
fn loads_profile() {
    let p = load_profile("trumpet").unwrap();
    assert_eq!(p.name, "trumpet");
}
```
**Fix:** Also test failure paths — what happens with bad input?

```rust
// GOOD: Tests both success AND meaningful failure
#[test]
fn loads_valid_profile() { /* ... */ }

#[test]
fn rejects_nonexistent_profile() {
    let err = load_profile("kazoo").unwrap_err();
    assert!(matches!(err, ProfileError::NotFound(_)));
}

#[test]
fn rejects_path_traversal() {
    let err = load_profile("../etc/passwd").unwrap_err();
    assert!(matches!(err, ProfileError::InvalidName(_)));
}
```

### 5. The Mock That Matches Production
```tsx
// BAD: Mock returns exactly what the test expects
// You're testing that your mock works, not that your code works
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue("pong"),
}));

it("shows pong", async () => {
  render(<App />);
  // Of course it shows pong — you hardcoded it
});
```
**Fix:** Test what the component *does with* the data, and also test the error path:

```tsx
// GOOD: Test error handling too
it("shows error state when backend fails", async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error("timeout"));
  render(<App />);
  expect(await screen.findByText(/connection failed/i)).toBeInTheDocument();
});
```

## What to Test (Priority Order)

### Must Test (every PR)
1. **Core business logic** — phrase detection, scoring, pitch math, frequency conversion
2. **Edge cases** — empty input, boundary values, zero, negative, NaN, Infinity
3. **Error handling** — what happens when things go wrong (API down, bad file, invalid config)
4. **State transitions** — Zustand store changes, Rust struct state machines
5. **Floating-point boundaries** — use epsilon comparison, never `==` on floats

### Should Test (most PRs)
6. **Serialization roundtrips** — data survives JSON encode/decode with correct values
7. **Integration flows** — audio event → phrase → coaching tip → UI
8. **Component behavior** — user interactions, conditional rendering based on state

### Nice to Have
9. **Performance** — latency benchmarks (covered by CI bench suite)
10. **Visual regression** — screenshot tests (future)

## Rust-Specific Rules

### Float Comparison
```rust
// BAD
assert_eq!(result, 0.3);

// GOOD
assert!((result - 0.3).abs() < 1e-6, "expected ~0.3, got {result}");

// ALSO GOOD for well-known values
assert!((result - expected).abs() < f64::EPSILON);
```

### Testing Error Types
```rust
// BAD: Only checks that SOME error occurred
assert!(result.is_err());

// GOOD: Checks the RIGHT error occurred
let err = result.unwrap_err();
assert!(matches!(err, PhraseError::InvalidSilenceGap(_)));
```

### Audio Thread Safety
Any code in `crates/ears` real-time path:
- Test that it NEVER allocates (no Vec::push, no String::from, no Box::new)
- Test buffer overflow behavior (ring buffer drops excess, not panics)
- Test with empty/full buffers

## TypeScript/React-Specific Rules

### Component Tests
```tsx
// BAD: Testing that React renders
it("renders", () => {
  render(<MyComponent />);
  // ...no assertion, or trivial assertion
});

// GOOD: Testing what the component DOES
it("disables start button when already listening", () => {
  useAudioStore.setState({ isListening: true });
  render(<PracticeSession />);
  expect(screen.getByRole("button", { name: /start/i })).toBeDisabled();
});
```

### Store Tests
```tsx
// BAD: Only testing that setState works (you're testing Zustand, not your code)
it("updates state", () => {
  store.setState({ x: 1 });
  expect(store.getState().x).toBe(1);
});

// GOOD: Testing derived state and business logic
it("derives note info when event has valid pitch", () => {
  store.getState().setEvent({ pitch_hz: 440, ... });
  const { currentNote } = store.getState();
  expect(currentNote?.name).toBe("A");
  expect(currentNote?.octave).toBe(4);
  expect(Math.abs(currentNote!.cents_deviation)).toBeLessThan(1);
});
```

### Error Boundaries
Every component that can fail must have a test for the failure case:
```tsx
it("shows fallback when store throws", () => { /* ... */ });
it("handles null/undefined gracefully", () => { /* ... */ });
```

## Test Naming Convention

Test names should describe the **scenario** and **expected outcome**:

```text
// BAD
test_serialization
test_profile
renders_component

// GOOD
silence_gap_equal_to_threshold_does_not_split_phrase
rejects_profile_with_inverted_frequency_range
shows_error_state_when_backend_connection_fails
```

Format: `[given_condition]_[expected_behavior]`

## Minimum Test Requirements Per Story

Before a PR can merge:
- [ ] Every acceptance criterion from the issue has a corresponding test
- [ ] At least one test per public function/method
- [ ] At least one error/edge case test per module
- [ ] No `assert!(true)` or existence-only checks
- [ ] Floating-point comparisons use epsilon tolerance
- [ ] Serialization tests do full roundtrip with value checks (not string contains)
- [ ] Mock-dependent tests also cover the failure path

## Audit Checklist (Run Before PR)

Before submitting, mentally run each test through this filter:

| Question | If No... |
|----------|----------|
| Can I make this test fail by introducing a bug? | Delete the test |
| Does this test check a REAL behavior? | Rewrite to test behavior |
| Would a junior dev understand what this tests? | Improve the test name |
| Am I testing MY code or the framework? | Remove framework assertions |
| Does this test cover an edge case or error? | Add edge case tests |
| Could I delete the production code and still pass? | Strengthen assertions |
