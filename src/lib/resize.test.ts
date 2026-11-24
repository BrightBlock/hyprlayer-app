import { describe, expect, it } from "vitest";

import { keyedSize, maxBeside } from "./resize";

describe("maxBeside", () => {
  it("leaves the neighbour its share of the container", () => {
    expect(maxBeside(700, 160, 120, 900)).toBe(540);
  });

  it("never drops below the minimum in a short container", () => {
    expect(maxBeside(200, 160, 120, 900)).toBe(120);
  });

  it("stays under the cap in a tall container", () => {
    expect(maxBeside(2000, 160, 120, 900)).toBe(900);
  });

  it("keeps the cap before the container is measured", () => {
    expect(maxBeside(0, 160, 120, 900)).toBe(900);
  });
});

describe("keyedSize", () => {
  const bounds = { size: 300, min: 240, max: 560 };

  it("moves the handle with the arrows along its axis", () => {
    expect(keyedSize("ArrowDown", false, { ...bounds, direction: 1, axis: "y" })).toBe(316);
    expect(keyedSize("ArrowUp", false, { ...bounds, direction: 1, axis: "y" })).toBe(284);
    expect(keyedSize("ArrowLeft", false, { ...bounds, direction: 1, axis: "y" })).toBeNull();
  });

  it("grows a panel whose handle sits on its left edge when the handle moves left", () => {
    expect(keyedSize("ArrowLeft", false, { ...bounds, direction: -1, axis: "x" })).toBe(316);
    expect(keyedSize("ArrowRight", true, { ...bounds, direction: -1, axis: "x" })).toBe(240);
  });

  it("clamps steps and jumps to the ends on Home and End", () => {
    expect(keyedSize("ArrowRight", true, { ...bounds, size: 550, direction: 1, axis: "x" })).toBe(560);
    expect(keyedSize("Home", false, { ...bounds, direction: 1, axis: "x" })).toBe(240);
    expect(keyedSize("End", false, { ...bounds, direction: 1, axis: "x" })).toBe(560);
  });

  it("ignores other keys", () => {
    expect(keyedSize("Enter", false, { ...bounds, direction: 1, axis: "x" })).toBeNull();
  });
});
