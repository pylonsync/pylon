import { expect, test } from "bun:test";
import { firstName, initials, nameFromEmail, personName } from "../lib/names";

test("a name built from an email drops the +tag and digits", () => {
  expect(nameFromEmail("dana+s13596@example.com")).toBe("Dana");
  expect(nameFromEmail("dana.reyes@example.com")).toBe("Dana Reyes");
  expect(nameFromEmail("sam_okafor-2@example.com")).toBe("Sam Okafor");
  expect(nameFromEmail("1234@example.com")).toBe("");
  expect(nameFromEmail(null)).toBe("");
});

test("the saved display name wins", () => {
  expect(personName({ displayName: "Dana Reyes", email: "d@example.com" })).toBe("Dana Reyes");
  expect(personName({ displayName: "  ", email: "leo.park@example.com" })).toBe("Leo Park");
});

test("a name that is an email address counts as no name", () => {
  expect(personName({ name: "priya@example.com", email: "priya@example.com" })).toBe("Priya");
});

test("an email with nothing readable falls back to the email", () => {
  expect(personName({ email: "42@example.com" })).toBe("42@example.com");
});

test("greetings use the first word; avatars use two initials", () => {
  expect(firstName({ displayName: "Dana Reyes" })).toBe("Dana");
  expect(initials("Dana Reyes")).toBe("DR");
  expect(initials("Priya Anand Raman")).toBe("PR");
  expect(initials("Leo")).toBe("L");
  expect(initials("")).toBe("?");
});

test("names longer than the UI limit are cut", () => {
  expect(personName({ displayName: "x".repeat(500) })).toHaveLength(60);
});
