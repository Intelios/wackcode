export function titleFromPrompt(message: string, max = 48): string {
  const line = message
    .split("\n")
    .map((part) => part.trim())
    .find((part) => part.length > 0);
  if (!line) return "New chat";
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

export function displayPath(path: string): string {
  const normalized = path.replace(/^\.\//, "").replace(/\/+$/, "");
  const parts = normalized.split("/").filter(Boolean);
  if (parts.length === 0) return path;
  if (normalized.startsWith("/") && parts.length > 2) return `…/${parts.slice(-2).join("/")}`;
  if (parts.length > 3) return `…/${parts.slice(-2).join("/")}`;
  return normalized;
}

export function formatTokens(value?: number): string {
  if (value === undefined) return "—";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}
