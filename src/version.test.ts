import { describe, expect, it } from "vitest";
import { compareVersions, getVersionStatus } from "./version";

describe("compareVersions", () => {
  it("ignores the leading v prefix and compares numeric parts", () => {
    expect(compareVersions("v1.2.3", "v1.2.4")).toBeLessThan(0);
    expect(compareVersions("1.2.3", "1.2.3")).toBe(0);
    expect(compareVersions("1.2.5", "v1.2.4")).toBeGreaterThan(0);
  });

  it("reports when a newer release is available", () => {
    expect(getVersionStatus("1.2.3", "1.2.4")).toEqual({
      status: "update-available",
      current: "1.2.3",
      latest: "1.2.4",
    });
  });
});
