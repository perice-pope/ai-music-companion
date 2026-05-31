import { describe, it, expect, beforeEach, vi } from "vitest";
import type { Session } from "@supabase/supabase-js";

type AuthCallback = (event: string, session: Session | null) => void;

// Capture the onAuthStateChange callback so tests can drive auth events.
let authCallback: AuthCallback | null = null;
const mockOnAuthStateChange = vi.fn((cb: AuthCallback) => {
  authCallback = cb;
  return { data: { subscription: { unsubscribe: vi.fn() } } };
});
const mockSignUp = vi.fn();
const mockSignInWithPassword = vi.fn();
const mockSignOut = vi.fn();

vi.mock("../lib/supabase", () => ({
  supabase: {
    auth: {
      onAuthStateChange: (cb: AuthCallback) => mockOnAuthStateChange(cb),
      signUp: (...a: unknown[]) => mockSignUp(...a),
      signInWithPassword: (...a: unknown[]) => mockSignInWithPassword(...a),
      signOut: (...a: unknown[]) => mockSignOut(...a),
    },
  },
}));

async function freshStore() {
  vi.resetModules();
  const mod = await import("./authStore");
  return mod.useAuthStore;
}

function fakeSession(email: string): Session {
  return {
    access_token: "tok",
    refresh_token: "ref",
    expires_in: 3600,
    token_type: "bearer",
    user: { id: "u-1", email } as Session["user"],
  } as Session;
}

describe("authStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    authCallback = null;
  });

  it("subscribes on init and reflects a null session as signed_out", async () => {
    const useStore = await freshStore();
    await useStore.getState().init();
    expect(mockOnAuthStateChange).toHaveBeenCalledTimes(1);

    authCallback?.("INITIAL_SESSION", null);
    expect(useStore.getState().status).toBe("signed_out");
    expect(useStore.getState().user).toBeNull();
  });

  it("flips to signed_in when a session arrives", async () => {
    const useStore = await freshStore();
    await useStore.getState().init();

    authCallback?.("SIGNED_IN", fakeSession("a@b.com"));
    const s = useStore.getState();
    expect(s.status).toBe("signed_in");
    expect(s.user?.email).toBe("a@b.com");
  });

  it("surfaces a sign-in error and stays signed_out", async () => {
    mockSignInWithPassword.mockResolvedValue({
      error: { message: "Invalid login credentials" },
    });
    const useStore = await freshStore();

    await useStore.getState().signIn("a@b.com", "wrong");
    const s = useStore.getState();
    expect(s.status).toBe("signed_out");
    expect(s.error).toBe("Invalid login credentials");
  });

  it("nudges to confirm email when sign-up returns no session", async () => {
    mockSignUp.mockResolvedValue({ error: null });
    const useStore = await freshStore();

    await useStore.getState().signUp("new@b.com", "pw123456", "Newbie");
    expect(mockSignUp).toHaveBeenCalledWith({
      email: "new@b.com",
      password: "pw123456",
      options: { data: { display_name: "Newbie" } },
    });
    const s = useStore.getState();
    expect(s.status).toBe("signed_out");
    expect(s.error).toContain("Check your email");
  });

  it("clears error state on demand", async () => {
    mockSignInWithPassword.mockResolvedValue({
      error: { message: "boom" },
    });
    const useStore = await freshStore();
    await useStore.getState().signIn("a@b.com", "x");
    expect(useStore.getState().error).toBe("boom");

    useStore.getState().clearError();
    expect(useStore.getState().error).toBeNull();
  });
});
