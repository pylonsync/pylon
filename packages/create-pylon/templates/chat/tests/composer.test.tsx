import { afterEach, expect, mock, test } from "bun:test";
// This project uses the classic JSX transform, so .tsx tests import React.
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Composer } from "../components/chat/composer";

afterEach(cleanup);

function setup() {
  const onSend = mock((_text: string) => {});
  const onTyping = mock(() => {});
  const onStopTyping = mock(() => {});
  render(
    <Composer
      channelName="launch"
      channelKey="c1"
      onSend={onSend}
      onTyping={onTyping}
      onStopTyping={onStopTyping}
    />,
  );
  const field = screen.getByPlaceholderText("Message #launch") as HTMLTextAreaElement;
  return { field, onSend, onTyping, onStopTyping };
}

test("Enter sends the trimmed text and clears the field", () => {
  const { field, onSend, onStopTyping } = setup();
  fireEvent.change(field, { target: { value: "  ship it  " } });
  fireEvent.keyDown(field, { key: "Enter" });
  expect(onSend).toHaveBeenCalledWith("ship it");
  expect(field.value).toBe("");
  expect(onStopTyping).toHaveBeenCalled();
});

test("Shift+Enter does not send", () => {
  const { field, onSend } = setup();
  fireEvent.change(field, { target: { value: "line one" } });
  fireEvent.keyDown(field, { key: "Enter", shiftKey: true });
  expect(onSend).not.toHaveBeenCalled();
});

test("whitespace-only text does not send and the button stays disabled", () => {
  const { field, onSend } = setup();
  fireEvent.change(field, { target: { value: "   " } });
  fireEvent.keyDown(field, { key: "Enter" });
  expect(onSend).not.toHaveBeenCalled();
  expect((screen.getByRole("button", { name: "Send message" }) as HTMLButtonElement).disabled).toBe(true);
});

test("typing reports activity", () => {
  const { field, onTyping } = setup();
  fireEvent.change(field, { target: { value: "h" } });
  expect(onTyping).toHaveBeenCalled();
});
