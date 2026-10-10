import { describe, expect, it } from "vitest";
import { detectLocale, formatItemCount, locales, translate } from "./i18n";

describe("locale support", () => {
  it("offers all six requested languages", () => {
    expect(locales.map(({ id }) => id)).toEqual(["en", "fr", "zh-CN", "zh-TW", "ja", "ko"]);
  });

  it("detects regional Chinese and supported system languages", () => {
    expect(detectLocale("zh-Hans-CN")).toBe("zh-CN");
    expect(detectLocale("zh-Hant-TW")).toBe("zh-TW");
    expect(detectLocale("fr-CA")).toBe("fr");
    expect(detectLocale("ja-JP")).toBe("ja");
    expect(detectLocale("ko-KR")).toBe("ko");
    expect(detectLocale("de-DE")).toBe("en");
  });

  it("formats counts and interpolated messages", () => {
    expect(formatItemCount("en", 1)).toBe("1 item");
    expect(formatItemCount("fr", 2)).toBe("2 éléments");
    expect(translate("zh-TW", "copiedLinks", { count: 3, format: "HTML" })).toBe("已複製 3 個連結 · HTML");
  });
});