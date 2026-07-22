import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TeacherGate } from "./TeacherGate";
import { makeClient } from "../test/mockSupabase";

const profile = (role: string) => ({
  id: "u1",
  role,
  display_name: "Pat",
  age_tier: null,
  created_at: "2026-01-01T00:00:00Z",
});

describe("TeacherGate (spec AC1)", () => {
  it("admits a teacher role to the dashboard", async () => {
    const mock = makeClient({ profiles: [profile("teacher")] });
    render(
      <TeacherGate client={mock.client} userId="u1" onSignOut={() => {}}>
        <div>the dashboard shell</div>
      </TeacherGate>,
    );
    expect(await screen.findByText("the dashboard shell")).toBeInTheDocument();
  });

  it("blocks a non-teacher role with the polite dead-end", async () => {
    const mock = makeClient({ profiles: [profile("student")] });
    render(
      <TeacherGate client={mock.client} userId="u1" onSignOut={() => {}}>
        <div>the dashboard shell</div>
      </TeacherGate>,
    );
    expect(
      await screen.findByText("This dashboard is for teachers"),
    ).toBeInTheDocument();
    expect(screen.queryByText("the dashboard shell")).not.toBeInTheDocument();
  });

  it("issues no data queries for a non-teacher — only the own-profile read", async () => {
    const mock = makeClient({ profiles: [profile("student")] });
    render(
      <TeacherGate client={mock.client} userId="u1" onSignOut={() => {}}>
        <div>the dashboard shell</div>
      </TeacherGate>,
    );
    await screen.findByText("This dashboard is for teachers");
    expect([...new Set(mock.calls)]).toEqual(["profiles"]);
  });

  it("a failed profile read is an error, not a silent pass-through", async () => {
    const mock = makeClient({ profiles: new Error("network down") });
    render(
      <TeacherGate client={mock.client} userId="u1" onSignOut={() => {}}>
        <div>the dashboard shell</div>
      </TeacherGate>,
    );
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/network down/);
    expect(screen.queryByText("the dashboard shell")).not.toBeInTheDocument();
  });
});
