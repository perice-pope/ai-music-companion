import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import App from "./App";
import { useAuthStore } from "./stores/authStore";

describe("App shell (spec AC10)", () => {
  beforeEach(() => {
    useAuthStore.setState({
      status: "signed_out",
      user: null,
      session: null,
      error: null,
    });
  });

  it("signed out shows the sign-in screen (same Supabase email+password flow)", async () => {
    render(<App />);
    expect(
      await screen.findByText(
        /Sign in with your AI Music Companion teacher account/,
      ),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Email")).toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
  });
});
