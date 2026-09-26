import { expect, test } from "bun:test";
import React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { Markdown } from "../components/chat/markdown";
import { AssistantMessage } from "../components/chat/message";
import { Sidebar } from "../components/chat/sidebar";
import type { ConversationRow } from "../lib/chat";

test("Markdown renders code blocks with a copy button and no raw HTML", () => {
  const { container } = render(<Markdown text={"Hi <img src=x onerror=alert(1)>\n\n```ts\nconst a = 1;\n```"} />);
  expect(container.querySelector("img")).toBeNull();
  expect(container.querySelector("pre code")?.textContent).toBe("const a = 1;");
  expect(screen.getByRole("button", { name: "Copy code" })).toBeDefined();
});

test("a missing provider shows setup steps inside the failed reply", () => {
  render(<AssistantMessage content="" status="error" errorCode="LLM_NOT_CONFIGURED" canRegenerate={false} providerConfigured={false} />);
  expect(screen.getByText("Add a model provider key")).toBeDefined();
});

test("the newest reply offers Regenerate", () => {
  let clicked = 0;
  render(<AssistantMessage content="Done." status="done" canRegenerate onRegenerate={() => clicked++} />);
  fireEvent.click(screen.getByRole("button", { name: "Regenerate" }));
  expect(clicked).toBe(1);
});

test("a streaming reply with no text yet shows the writing indicator", () => {
  render(<AssistantMessage content="" status="streaming" canRegenerate={false} />);
  expect(screen.getByRole("status", { name: "Writing a reply" })).toBeDefined();
});

test("Sidebar groups conversations and marks examples", () => {
  const now = new Date(2026, 8, 26, 12).getTime();
  const rows: ConversationRow[] = [
    { id: "a", userId: "u", title: "Today chat", createdAt: new Date(2026, 8, 26, 9).toISOString() },
    { id: "b", userId: "u", title: "Sample", example: true, createdAt: new Date(2026, 8, 25, 9).toISOString() },
  ];
  render(
    <Sidebar
      conversations={rows}
      currentId="a"
      now={now}
      account={{ kind: "guest" }}
      onSelect={() => {}}
      onNew={() => {}}
      onRename={() => {}}
      onDelete={() => {}}
      onSignIn={() => {}}
      onSignOut={() => {}}
    />,
  );
  expect(screen.getByText("Today")).toBeDefined();
  expect(screen.getByText("Yesterday")).toBeDefined();
  expect(screen.getByText("Example")).toBeDefined();
  expect(screen.getByText("Sign in or create account")).toBeDefined();
});
