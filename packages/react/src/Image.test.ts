import { expect, test } from "bun:test";
import { imageCandidateWidths } from "./Image";

test("2x rounds down to an allowed width instead of jumping to 3840", () => {
  const { widths, oneX } = imageCandidateWidths(1200);
  expect(oneX).toBe(1200);
  expect(Math.max(...widths)).toBe(2048);
  expect(widths).not.toContain(3840);
});

test("offers smaller widths so `sizes` can pick a small file", () => {
  const { widths } = imageCandidateWidths(452);
  expect(widths).toEqual([256, 384, 640, 750, 828]);
});

test("the 1x width is always offered, even when it is above 2x of a tiny image", () => {
  const { widths, oneX } = imageCandidateWidths(20);
  expect(oneX).toBe(32);
  expect(widths).toContain(32);
});

test("very large images cap at 3840", () => {
  const { widths, oneX } = imageCandidateWidths(5000);
  expect(oneX).toBe(3840);
  expect(Math.max(...widths)).toBe(3840);
});

test("explicit widths are used as given, deduplicated and sorted", () => {
  expect(imageCandidateWidths(500, [1080, 640, 640]).widths).toEqual([640, 1080]);
  expect(imageCandidateWidths(500, [1080, 640]).oneX).toBe(640);
});
