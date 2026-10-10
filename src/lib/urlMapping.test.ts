import { describe, expect, it } from "vitest";
import { buildObjectUrl, formatLinks } from "./urlMapping";

describe("buildObjectUrl", () => {
  it("encodes path segments and joins the configured prefix", () => {
    expect(
      buildObjectUrl("活动照片/100% #1?.jpg", {
        baseUrl: "https://cdn.example.com/",
        prefix: "public files",
      }),
    ).toBe("https://cdn.example.com/public%20files/%E6%B4%BB%E5%8A%A8%E7%85%A7%E7%89%87/100%25%20%231%3F.jpg");
  });

  it("supports advanced templates", () => {
    expect(
      buildObjectUrl("image.png", {
        baseUrl: "https://cdn.example.com",
        prefix: "assets",
        bucket: "public media",
        template: "{base}/{bucket}/{key}",
      }),
    ).toBe("https://cdn.example.com/public%20media/assets/image.png");
  });

  it("rejects traversal, missing keys, and unknown placeholders", () => {
    const rules = { baseUrl: "https://cdn.example.com", prefix: "" };
    expect(() => buildObjectUrl("../secret.txt", rules)).toThrow("路径不能包含 ..");
    expect(() => buildObjectUrl("secret.txt", { ...rules, template: "https://example.com" })).toThrow(
      "模板必须包含 {key} 或 {path}",
    );
    expect(() => buildObjectUrl("secret.txt", { ...rules, template: "{base}/{unknown}/{key}" })).toThrow(
      "模板包含未知占位符",
    );
  });
});

describe("formatLinks", () => {
  const links = [{ name: "A < B.md", url: "https://cdn.example.com/a%20b.md" }];

  it("formats plain text, Markdown, and escaped HTML", () => {
    expect(formatLinks(links, "url")).toBe(links[0].url);
    expect(formatLinks(links, "markdown")).toBe("[A < B.md](https://cdn.example.com/a%20b.md)");
    expect(formatLinks(links, "html")).toBe(
      '<a href="https://cdn.example.com/a%20b.md">A &lt; B.md</a>',
    );
  });
});