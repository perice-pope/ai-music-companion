import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import ToneSummary from "./ToneSummary";
import type { ToneDescriptor } from "../types/brain";

const TONE: ToneDescriptor = {
  brightness: 0.6,
  warmth: 0.5,
  air_noise: 0.2,
  core_clarity: 0.8,
  vibrato_quality: 0.55,
};

describe("ToneSummary", () => {
  it("renders all five tone dimensions as labelled meters", () => {
    render(<ToneSummary tone={TONE} />);
    expect(screen.getByTestId("recap-tone")).toBeTruthy();
    for (const label of ["Brightness", "Warmth", "Core clarity", "Air / breath", "Vibrato"]) {
      expect(screen.getByText(label)).toBeTruthy();
    }
    // Five accessible meters, qualitative labels — no numeric grade leaks.
    expect(screen.getAllByRole("meter")).toHaveLength(5);
  });

  it("describes values qualitatively, never as a numeric score", () => {
    render(<ToneSummary tone={TONE} />);
    // core_clarity 0.8 → "pronounced"; air_noise 0.2 → "subtle".
    expect(screen.getByLabelText("Core clarity: pronounced")).toBeTruthy();
    expect(screen.getByLabelText("Air / breath: subtle")).toBeTruthy();
    // No "%" or "/10"-style grade anywhere in the rendered text.
    expect(screen.getByTestId("recap-tone").textContent).not.toMatch(/\d\s*%|\/\s*10/);
  });
});
