export function normalizeVersion(value: string): string {
  return value.trim().replace(/^v/i, "").split(/[\-+]/, 1)[0].trim();
}

export function parseVersionParts(value: string): number[] {
  const normalized = normalizeVersion(value);
  if (!normalized) return [0];

  return normalized.split(".").map((part) => {
    const digits = part.replace(/\D/g, "");
    return digits ? Number.parseInt(digits, 10) : 0;
  });
}

export function compareVersions(current: string, latest: string): number {
  const left = parseVersionParts(current);
  const right = parseVersionParts(latest);
  const limit = Math.max(left.length, right.length);

  for (let index = 0; index < limit; index += 1) {
    const leftValue = left[index] ?? 0;
    const rightValue = right[index] ?? 0;
    if (leftValue !== rightValue) return leftValue - rightValue;
  }

  return 0;
}

export function getVersionStatus(current: string, latest: string): {
  status: "up-to-date" | "update-available";
  current: string;
  latest: string;
} {
  const normalizedCurrent = normalizeVersion(current);
  const normalizedLatest = normalizeVersion(latest);

  if (!normalizedCurrent || !normalizedLatest) {
    return { status: "up-to-date", current: normalizedCurrent || current, latest: normalizedLatest || latest };
  }

  return compareVersions(normalizedCurrent, normalizedLatest) >= 0
    ? { status: "up-to-date", current: normalizedCurrent, latest: normalizedLatest }
    : { status: "update-available", current: normalizedCurrent, latest: normalizedLatest };
}
