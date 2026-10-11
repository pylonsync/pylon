import { describe, expect, test } from "bun:test";
import { defineConnection } from "./index";

describe("defineConnection", () => {
  test("leaves defaults out of the manifest", () => {
    expect(defineConnection({ name: "google", provider: "google", owner: "user", kind: "oauth" })).toEqual({
      name: "google",
      provider: "google",
    });
  });

  test("carries an app connection and its key fields", () => {
    expect(
      defineConnection({
        name: "toast",
        provider: "toast",
        owner: "app",
        kind: "api_key",
        fields: ["clientId", "clientSecret", "restaurantGuid"],
      }),
    ).toEqual({
      name: "toast",
      provider: "toast",
      owner: "app",
      kind: "api_key",
      fields: ["clientId", "clientSecret", "restaurantGuid"],
    });
  });

  test("refuses fields on an OAuth connection", () => {
    expect(() => defineConnection({ name: "sq", provider: "square", fields: ["x"] })).toThrow(
      /fields apply to kind "api_key" only/,
    );
  });
});
