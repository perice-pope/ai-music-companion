import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  within,
  waitFor,
} from "@testing-library/react";
import ConnectionsPrivacy from "./ConnectionsPrivacy";
import { usePracticeStore } from "../stores/practiceStore";
import { useConnectionsStore } from "../stores/connectionsStore";
import { useUpdateStore } from "../stores/updateStore";

// #58 MF4: the manual check button talks to the updater plugin — mocked.
const mockUpdateCheck = vi.fn();
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: () => mockUpdateCheck(),
}));

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
      dashboardSyncEnabled: false,
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
    expect(switches.length).toBe(5);

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
    // #449 T2: the dashboard projection — off by default, like everything.
    expect(
      (
        screen.getByRole("switch", {
          name: /Share practice details with your teacher/i,
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
    expect(s.dashboardSyncEnabled).toBe(false);
    expect(useConnectionsStore.getState().autoUpdateCheckEnabled).toBe(false);
  });

  // #449 T2 AC8: the dashboard-sync disclosure — what leaves, to whom, what
  // never leaves — with the toggle gated on cloud sync and off by default.
  it("discloses the teacher-dashboard projection: payload, audience, and the never-leaves list", () => {
    render(<ConnectionsPrivacy />);
    const sw = screen.getByRole("switch", {
      name: /Share practice details with your teacher/i,
    }) as HTMLInputElement;
    expect(sw.checked).toBe(false);
    // Meaningless without cloud sync — disabled until it's on.
    expect(sw.disabled).toBe(true);

    const row = sw.closest("div.rounded-lg");
    expect(row).not.toBeNull();
    const inRow = within(row as HTMLElement);
    // What leaves: session facts, phrase timings, exercise labels, tool use.
    expect(
      inRow.getByText(
        /minutes of actual playing vs\. minutes the app was open/i,
      ),
    ).toBeTruthy();
    expect(inRow.getByText(/start and end times of each phrase/i)).toBeTruthy();
    expect(inRow.getByText(/names and scores of exercises/i)).toBeTruthy();
    expect(inRow.getByText(/metronome on\/off and tempo/i)).toBeTruthy();
    // To whom.
    expect(
      inRow.getByText(/teacher you joined through a classroom code/i),
    ).toBeTruthy();
    // What never leaves — the question parents actually ask.
    expect(
      inRow.getByText(
        /never sends your audio recording, the actual notes you played, or your saved exercise recipes and seeds/i,
      ),
    ).toBeTruthy();
  });

  it("the dashboard toggle enables with cloud sync and withdraws when sync turns off", () => {
    useConnectionsStore.setState({ cloudSyncEnabled: true });
    render(<ConnectionsPrivacy />);
    const sw = screen.getByRole("switch", {
      name: /Share practice details with your teacher/i,
    }) as HTMLInputElement;
    expect(sw.disabled).toBe(false);

    fireEvent.click(sw);
    expect(useConnectionsStore.getState().dashboardSyncEnabled).toBe(true);

    // Withdrawing cloud sync withdraws the projection that rides on it.
    useConnectionsStore.getState().setCloudSyncEnabled(false);
    expect(useConnectionsStore.getState().dashboardSyncEnabled).toBe(false);
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
      within(row).getByText(
        /Unless automatic checks are on, the app never checks on startup/i,
      ),
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

  // #58 review MF4: the manual "Check for updates" the copy has always
  // promised now exists — user-initiated, independent of the toggle, with
  // a calm up-to-date answer. Fails if the button disappears or stops
  // calling the plugin.
  it("the manual check button works regardless of the auto-check toggle", async () => {
    mockUpdateCheck.mockResolvedValue(null); // up to date
    useUpdateStore.setState({
      phase: "idle",
      availableVersion: null,
      notice: null,
      dismissedVersion: null,
    });
    useConnectionsStore.setState({ autoUpdateCheckEnabled: false });
    render(<ConnectionsPrivacy />);
    fireEvent.click(screen.getByTestId("check-updates-now"));
    await waitFor(() =>
      expect(screen.getByTestId("check-updates-result").textContent).toContain(
        "latest version",
      ),
    );
    expect(mockUpdateCheck).toHaveBeenCalledTimes(1);
  });

  // #58 round-2 MF1(a): offline, the button must NOT fabricate
  // "You're on the latest version" — it says it couldn't check.
  it("the manual check answers honestly when the check fails", async () => {
    mockUpdateCheck.mockRejectedValue(new Error("offline"));
    useUpdateStore.setState({
      phase: "idle",
      availableVersion: null,
      notice: null,
      dismissedVersion: null,
    });
    render(<ConnectionsPrivacy />);
    fireEvent.click(screen.getByTestId("check-updates-now"));
    await waitFor(() =>
      expect(screen.getByTestId("check-updates-result").textContent).toContain(
        "Couldn't check",
      ),
    );
    expect(
      screen.getByTestId("check-updates-result").textContent,
    ).not.toContain("latest version");
  });

  // #58 round-2 MF1(b): a pill dismissal quiets the AUTOMATIC surface only —
  // an explicit manual check overrides it and re-surfaces the update.
  it("a manual check overrides a dismissed version", async () => {
    mockUpdateCheck.mockResolvedValue({
      version: "9.9.9",
      downloadAndInstall: vi.fn(),
    });
    useUpdateStore.setState({
      phase: "idle",
      availableVersion: null,
      notice: null,
      dismissedVersion: "9.9.9",
    });
    render(<ConnectionsPrivacy />);
    fireEvent.click(screen.getByTestId("check-updates-now"));
    await waitFor(() =>
      expect(screen.getByTestId("check-updates-result").textContent).toContain(
        "Update found",
      ),
    );
    expect(useUpdateStore.getState().phase).toBe("available");
    expect(useUpdateStore.getState().availableVersion).toBe("9.9.9");
  });

  // #58 round-2 SF2: the button is disabled while a check is in flight.
  it("the manual check button disables while checking", async () => {
    let resolveCheck: (u: unknown) => void = () => {};
    mockUpdateCheck.mockImplementation(
      () => new Promise((res) => (resolveCheck = res)),
    );
    useUpdateStore.setState({
      phase: "idle",
      availableVersion: null,
      notice: null,
      dismissedVersion: null,
    });
    render(<ConnectionsPrivacy />);
    fireEvent.click(screen.getByTestId("check-updates-now"));
    await waitFor(() =>
      expect(screen.getByTestId("check-updates-now")).toBeDisabled(),
    );
    await waitFor(() => {
      resolveCheck(null);
      expect(screen.getByTestId("check-updates-now")).not.toBeDisabled();
    });
  });
});
