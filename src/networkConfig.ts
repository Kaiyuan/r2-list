export type RemoteConfig = { endpoint: string; username: string; password: string };

function trimTrailingSlashes(value: string): string {
  return value.trim().replace(/\/+$/, "");
}

export function buildRemoteConfig(config: Partial<RemoteConfig>): RemoteConfig {
  const endpoint = trimTrailingSlashes(config.endpoint ?? "");

  return {
    endpoint,
    username: (config.username ?? "").trim(),
    password: config.password ?? "",
  };
}

export function buildRemoteLocationLabel(endpoint: string): string {
  return trimTrailingSlashes(endpoint) || "WebDAV";
}
