import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import ConnectionsPrivacy from "./ConnectionsPrivacy";
import { usePracticeStore } from "../stores/practiceStore";
import { useConnectionsStore } from "../stores/connectionsStore";

// practiceStore pulls in @tauri-apps/api/core (invoke); stub it so the store
// imports cleanly under jsdom. The panel itself never calls invoke.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

// The version badge at the panel's foot (#384) reads the bundle version
// through the app API; give it a stable value here.
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: () => Promise.resolve("2.29.0"),
}));

// localStorage polyfill (jsdom in CI doesn't always persist). Critical: the
// store reads it at module init for the "off by default" guarantee, so we make
// sure it starts empty.
const store: Record<string, string> = {};
const localStorageMock = {
  getItem: vi.fn((k: string) => store[k] ?? null),
  setItem: vi.fn((k: string, v: string) => {
    store[k] = v;
  }),
  removeItem: vi.fn((k: string) => {
    delete store[k];
  }),
  clear: vi.fn(() => {
    for (const k of Object.keys(store)) delete store[k];
  }),
  get length() {
    return Object.keys(store).length;
  },
  key: vi.fn(() => null),
};
Object.defineProperty(window, "localStorage", { value: localStorageMock });

describe("ConnectionsPrivacy", () => {
  beforeEach(() => {
    localStorageMock.clear();
    // Reset the connections opt-ins to their off-by-default state.
    useConnectionsStore.setState({
      cloudSyncEnabled: false,
      teacherSharingEnabled: false,
    });
  });

  it("shows the running app version so testers can quote their build (#384)", async () => {
    render(<ConnectionsPrivacy />);
    expect(await screen.findByTestId("app-version")).toHaveTextContent(
      "v2.29.0",
    );
  });

  it("defaults EVERY networked toggle to OFF", () => {
    // AI coaching narration now defaults OFF in practiceStore (the offline-first
    // default flip). The store already reads off-by-default at module init; we
    // set it here only to keep the test independent of cross-test state leakage.
    usePracticeStore.setState({ coachingEnabled: false });

    render(<ConnectionsPrivacy />);

    // Every switch in the panel reports OFF.
    const switches = screen.getAllByRole("switch") as HTMLInputElement[];
    expect(switches.length).toBeGreaterThanOrEqual(3);
    for (const sw of switches) {
      expect(sw.checked).toBe(false);
    }

    // The app-updates disclosure stays an INFO row (no switch of its own —
    // a non-functional switch would be a dark pattern); #58 added a fourth
    // REAL opt-in above it: automatic update checks. Count is pinned so a
    // networked toggle can't appear without landing in this test.
    expect(screen.queryByRole("switch", { name: /^App updates/i })).toBeNull();
    expect(switches.length).toBe(4);

    // Named, so a regression that flips one on is caught by feature.
    expect(
      (
        screen.getByRole("switch", {
          name: /AI coaching narration/i,
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);
    expect(
      (screen.getByRole("switch", { name: /Cloud sync/i }) as HTMLInputElement)
        .checked,
    ).toBe(false);
    expect(
      (
        screen.getByRole("switch", {
          name: /Share with a teacher/i,
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);
    // #58: automatic update checks — the shipped promise is off-by-default.
    expect(
      (
        screen.getByRole("switch", {
          name: /Check for updates automatically/i,
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);
  });

  it("the connections store opts every networked feature out by default", () => {
    // Independent of the rendered defaults: the persistence layer itself must
    // start off, so a fresh install sends nothing.
    const s = useConnectionsStore.getState();
    expect(s.cloudSyncEnabled).toBe(false);
    expect(s.teacherSharingEnabled).toBe(false);
  });

  it("renders the disclosure copy: what is sent, to whom, and the offline reassurance", () => {
    render(<ConnectionsPrivacy />);

    // The standing, always-present reassurance.
    expect(screen.getByTestId("offline-reassurance").textContent).toMatch(
      /Everything else works offline/i,
    );

    // Each networked feature discloses what it sends (getByText throws if
    // absent, so a successful lookup is the assertion).
    expect(screen.getByText(/sends your performance numbers/i)).toBeTruthy();
    expect(screen.getByText(/to the AI provider/i)).toBeTruthy();
    expect(screen.getByText(/to our secure cloud/i)).toBeTruthy();

    // The "no audio" promise — the question parents actually ask — appears.
    expect(
      screen.getAllByText(/never sends your audio recording/i).length,
    ).toBeGreaterThanOrEqual(1);

    // The "coach, don't judge" stance: no account required, nothing in the
    // background, no default tracking.
    expect(
      screen.getByText(/never require an account to practice/i),
    ).toBeTruthy();
  });

  it("discloses the app auto-updater as an info row: contacts GitHub only on request, never on startup, fully offline", () => {
    render(<ConnectionsPrivacy />);

    const row = screen.getByTestId("info-app-updates");
    // What is sent, to whom, and when — in plain language.
    expect(within(row).getByText(/contacts GitHub/i)).toBeTruthy();
    expect(
      within(row).getByText(/never checks on startup and works fully offline/i),
    ).toBeTruthy();
    // The "no personal data" promise, the question parents ask.
    expect(
      within(row).getByText(
        /never sends your audio, your practice history, or any personal data/i,
      ),
    ).toBeTruthy();
    // It is disclosure, not a control: no switch in this row.
    expect(within(row).queryByRole("switch")).toBeNull();
  });

  it("explains that AI coaching off means on-device feedback (the offline fallback)", () => {
    render(<ConnectionsPrivacy />);
    const aiSwitch = screen.getByRole("switch", {
      name: /AI coaching narration/i,
    });
    // Walk up to the row and assert its "when off" copy describes the
    // on-device fallback rather than a broken/disabled coach.
    const row = aiSwitch.closest("div.rounded-lg");
    expect(row).not.toBeNull();
    expect(
      within(row as HTMLElement).getByText(/fully on-device feedback/i),
    ).toBeTruthy();
  });

  it("turning a networked toggle on then off returns it to off (opt-in is reversible)", () => {
    render(<ConnectionsPrivacy />);
    const cloud = screen.getByRole("switch", {
      name: /Cloud sync/i,
    }) as HTMLInputElement;

    expect(cloud.checked).toBe(false);
    fireEvent.click(cloud);
    expect(useConnectionsStore.getState().cloudSyncEnabled).toBe(true);
    fireEvent.click(cloud);
    expect(useConnectionsStore.getState().cloudSyncEnabled).toBe(false);
    // Withdrawing sync also withdraws the teacher-sharing feature it carries.
    expect(useConnectionsStore.getState().teacherSharingEnabled).toBe(false);
  });
});
