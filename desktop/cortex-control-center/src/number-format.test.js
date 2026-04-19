import { describe, expect, it } from "vitest";

import { formatCompactNumber, formatSignedCompactNumber } from "./number-format.js";

describe("number formatting", () => {
  it("formats common token magnitudes without scientific notation", () => {
    expect(formatCompactNumber(174)).toBe("174");
    expect(formatCompactNumber(21_600_000)).toBe("21.6M");
    expect(formatCompactNumber(119_000_000)).toBe("119M");
  });

  it("formats very large values with high-order suffixes", () => {
    const formatted = formatCompactNumber(46_628_816_004_992_054);
    expect(formatted).toBe("46.6Q");
    expect(formatted.toLowerCase()).not.toContain("e+");
  });

  it("never uses scientific notation for extreme values", () => {
    const formatted = formatCompactNumber(4.6628816004992054e76);
    expect(formatted.toLowerCase()).not.toContain("e+");
    expect(formatted).toMatch(/^[0-9]+(\.[0-9]{1,2})?[A-Za-z+0-9]+$/);
  });

  it("formats signed compact values with explicit sign", () => {
    expect(formatSignedCompactNumber(0)).toBe("0");
    expect(formatSignedCompactNumber(1_234_567)).toBe("+1.2M");
    expect(formatSignedCompactNumber(-1_234_567)).toBe("-1.2M");
  });
});
