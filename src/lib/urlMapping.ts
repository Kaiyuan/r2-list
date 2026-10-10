export type ShareFormat = "url" | "markdown" | "html";

export interface UrlRules {
  baseUrl: string;
  prefix: string;
  template?: string;
  bucket?: string;
}

export interface ShareLink {
  name: string;
  url: string;
}

function pathSegments(value: string): string[] {
  const segments = value.replace(/\\/g, "/").split("/").filter(Boolean);
  if (segments.some((segment) => segment === "..")) {
    throw new Error("路径不能包含 ..");
  }
  return segments.filter((segment) => segment !== ".");
}

function encodePath(value: string): string {
  return pathSegments(value)
    .map((segment) =>
      encodeURIComponent(segment).replace(/[!'()*]/g, (character) =>
        `%${character.charCodeAt(0).toString(16).toUpperCase()}`,
      ),
    )
    .join("/");
}

function cleanBaseUrl(value: string): string {
  const baseUrl = value.trim().replace(/\/+$/, "");
  if (!baseUrl) throw new Error("请填写公开 URL 基址");

  let parsed: URL;
  try {
    parsed = new URL(baseUrl);
  } catch {
    throw new Error("URL 基址必须是完整的 http 或 https 地址");
  }
  if (!/^https?:$/.test(parsed.protocol) || parsed.search || parsed.hash) {
    throw new Error("URL 基址仅支持不含查询参数的 http 或 https 地址");
  }
  return baseUrl;
}

export function buildObjectUrl(relativePath: string, rules: UrlRules): string {
  const base = cleanBaseUrl(rules.baseUrl);
  const key = encodePath([...pathSegments(rules.prefix), ...pathSegments(relativePath)].join("/"));
  if (!key) throw new Error("对象路径不能为空");

  const template = rules.template?.trim();
  if (!template) return `${base}/${key}`;
  if (!/\{(?:key|path)\}/.test(template)) throw new Error("模板必须包含 {key} 或 {path}");
  if (/\{(?!base\}|bucket\}|key\}|path\})[^{}]+\}/.test(template)) {
    throw new Error("模板包含未知占位符");
  }

  return template.replace(/\{(base|bucket|key|path)\}/g, (_match, token: string) => {
    switch (token) {
      case "base":
        return base;
      case "bucket":
        return encodePath(rules.bucket ?? "");
      case "key":
      case "path":
        return key;
      default:
        return _match;
    }
  });
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function formatLinks(links: ShareLink[], format: ShareFormat): string {
  if (format === "markdown") {
    return links.map(({ name, url }) => `[${name.replace(/[\\[\]]/g, "\\$&")}](${url})`).join("\n");
  }
  if (format === "html") {
    return links
      .map(({ name, url }) => `<a href="${escapeHtml(url)}">${escapeHtml(name)}</a>`)
      .join("\n");
  }
  return links.map(({ url }) => url).join("\n");
}