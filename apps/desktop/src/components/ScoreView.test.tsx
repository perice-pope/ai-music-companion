import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import ScoreView from "./ScoreView";
import type { ScorePosition } from "../types/brain";

describe("ScoreView", () => {
  const mockMusicXml =
    '<?xml version="1.0" encoding="UTF-8"?><score-partwise></score-partwise>';

  it("renders a container for the score", () => {
    render(<ScoreView musicXml={mockMusicXml} cursorPosition={null} />);
    const container = screen.getByTestId("score-view-container");
    expect(container).toBeTruthy();
  });

  it("renders with fixed height styling", () => {
    render(<ScoreView musicXml={mockMusicXml} cursorPosition={null} />);
    const container = screen.getByTestId("score-view-container");
    const style = container.getAttribute("style");
    expect(style).toContain("min-height");
  });

  it("applies correct CSS classes for styling", () => {
    render(<ScoreView musicXml={mockMusicXml} cursorPosition={null} />);
    const wrapper = screen.getByTestId("score-view-container").parentElement;
    expect(wrapper?.className).toContain("bg-gray-800");
    expect(wrapper?.className).toContain("flex-col");
  });

  it("handles null cursorPosition", () => {
    const { rerender } = render(
      <ScoreView musicXml={mockMusicXml} cursorPosition={null} />,
    );
    const container = screen.getByTestId("score-view-container");
    expect(container).toBeTruthy();

    const cursorPos: ScorePosition = { measure_number: 1, beat: 1 };
    rerender(
      <ScoreView musicXml={mockMusicXml} cursorPosition={cursorPos} />,
    );
    expect(container).toBeTruthy();
  });

  it("accepts different musicXml props", () => {
    const xml1 = "<score-partwise></score-partwise>";
    const xml2 = "<score-partwise><part></part></score-partwise>";

    const { rerender } = render(
      <ScoreView musicXml={xml1} cursorPosition={null} />,
    );
    expect(screen.getByTestId("score-view-container")).toBeTruthy();

    rerender(<ScoreView musicXml={xml2} cursorPosition={null} />);
    expect(screen.getByTestId("score-view-container")).toBeTruthy();
  });
});
