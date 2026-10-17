/**
 * Settings › Web's pure helpers: the presets as the page describes them, key state, cost
 * estimates and the hero's pill. What a preset actually sends lives in Rust (`web.rs`); keep
 * `linkOrigin`, `supportsFetch` and `needsKey` in step with its `Preset` table.
 */
import type {
  CustomProviderRecord, CustomSearchSpec, ProviderRecord, SaveSearchProviderInput, SearchProviderKind, SearchProviderRecord,
  WebConfig, WebUsageCounters, WebUsageSummary
} from "./types";

export interface SearchPreset {
  kind: SearchProviderKind;
  label: string;
  tagline: string;
  /** SearXNG's instance or a Custom API's URL. */
  needsUrl: boolean;
  urlPlaceholder?: string;
  supportsFetch: boolean;
  /** A Providers connection on this origin may lend its key. */
  linkOrigin?: string;
  /** Where to get a key, as plain text: the renderer never opens a provider's site itself. */
  keyHint?: string;
  priceSearchPer1k?: number;
  priceFetchPer1k?: number;
}

export const SEARCH_PRESETS: readonly SearchPreset[] = [
  {
    kind: "deepseek", label: "DeepSeek",
    tagline: "Web search and page reading on DeepSeek's API. Can reuse your DeepSeek connection's key.",
    needsUrl: false, supportsFetch: true, linkOrigin: "https://api.deepseek.com",
    keyHint: "A DeepSeek API key, from platform.deepseek.com.", priceSearchPer1k: 0.75, priceFetchPer1k: 0.75,
  },
  {
    kind: "brave", label: "Brave Search",
    tagline: "Brave's own independent web index, through the Brave Search API.",
    needsUrl: false, supportsFetch: false, keyHint: "A Brave Search API subscription token.",
  },
  {
    kind: "tavily", label: "Tavily",
    tagline: "A search API built for agents, with page extraction.",
    needsUrl: false, supportsFetch: true, keyHint: "A Tavily API key (starts with tvly-).",
  },
  {
    kind: "searxng", label: "SearXNG",
    tagline: "Your own metasearch instance: no key, nothing billed, and it can run on this Mac.",
    needsUrl: true, urlPlaceholder: "http://localhost:8888", supportsFetch: false,
  },
  {
    kind: "custom", label: "Custom",
    tagline: "Any JSON search API. Say where the query goes and where the results are.",
    needsUrl: true, urlPlaceholder: "https://api.example.com/search", supportsFetch: false,
  },
];

export function presetFor(kind: SearchProviderKind): SearchPreset {
  return SEARCH_PRESETS.find((preset) => preset.kind === kind) ?? SEARCH_PRESETS[SEARCH_PRESETS.length - 1];
}

/** One or two letters for a provider's mark: the initials of its first two words. */
export function monogram(name: string): string {
  const words = name.trim().split(/[\s._-]+/).filter(Boolean);
  if (words.length === 0) return "?";
  const letters = words.length > 1 ? words[0][0] + words[1][0] : words[0][0];
  return letters.toUpperCase();
}

/** A result's mark: the first letter of the site's own name (rust-lang for blog.rust-lang.org). */
export function siteInitial(host: string): string {
  const labels = host.toLowerCase().replace(/^www\./, "").split(".").filter(Boolean);
  let name = labels.length >= 2 ? labels[labels.length - 2] : labels[0] ?? "";
  // co.uk, com.au and friends: the name sits one label further left.
  if (labels.length >= 3 && name.length <= 3 && labels[labels.length - 1].length === 2) name = labels[labels.length - 3];
  return (name.match(/[a-z0-9]/)?.[0] ?? "?").toUpperCase();
}

export function needsKey(kind: SearchProviderKind, custom?: CustomSearchSpec | null): boolean {
  if (kind === "searxng") return false;
  if (kind === "custom") return Boolean(custom?.authHeader?.trim());
  return true;
}

export function sameOrigin(left: string, right: string): boolean {
  try {
    return new URL(left).origin === new URL(right).origin;
  } catch {
    return false;
  }
}

/** API-key connections a preset may borrow a key from: switched on, with a key, on its origin. */
export function linkableConnections(kind: SearchProviderKind, providers: readonly ProviderRecord[]): CustomProviderRecord[] {
  const origin = presetFor(kind).linkOrigin;
  if (!origin) return [];
  return providers.filter((provider): provider is CustomProviderRecord =>
    provider.kind === "custom" && provider.enabled !== false && provider.hasApiKey && sameOrigin(provider.baseUrl, origin));
}

/** What a provider's key line says, in the `package-state` tones. */
export function keyState(record: SearchProviderRecord, providers: readonly ProviderRecord[]): { tone: "on" | "off"; label: string } {
  const source = record.keySource;
  if (source.type === "none") return { tone: "on", label: "No key needed" };
  if (source.type === "connection") {
    const connection = providers.find((provider) => provider.id === source.providerId);
    if (!connection) return { tone: "off", label: "Linked connection removed" };
    const usable = linkableConnections(record.kind, providers).some((provider) => provider.id === connection.id);
    return usable ? { tone: "on", label: `Uses ‘${connection.name}’ key` } : { tone: "off", label: `‘${connection.name}’ can't lend its key` };
  }
  return record.hasKey ? { tone: "on", label: "Key saved" } : { tone: "off", label: "No API key" };
}

export function keyReady(record: SearchProviderRecord, providers: readonly ProviderRecord[]): boolean {
  return keyState(record, providers).tone === "on";
}

export const EMPTY_COUNTERS: WebUsageCounters = { searches: 0, fetches: 0, billedSearches: 0, billedFetches: 0, failed: 0 };

export function usageFor(usage: WebUsageSummary | undefined, id: string): WebUsageCounters {
  return usage?.providers.find((entry) => entry.id === id) ?? EMPTY_COUNTERS;
}

/** The estimate from the user's own prices; undefined when no price is set. */
export function estimatedCost(record: SearchProviderRecord, counters: WebUsageCounters): number | undefined {
  const search = record.priceSearchPer1k;
  const fetch = record.priceFetchPer1k;
  if (search == null && fetch == null) return undefined;
  return (counters.billedSearches * (search ?? 0) + counters.billedFetches * (fetch ?? 0)) / 1000;
}

export function formatCost(value: number): string {
  if (value === 0) return "$0.00";
  if (value < 0.01) return "<$0.01";
  return `$${value.toFixed(value < 10 ? 2 : 0)}`;
}

export const plural = (count: number, word: string) =>
  `${count.toLocaleString("en-US")} ${word}${count === 1 ? "" : /(s|sh|ch|x)$/.test(word) ? "es" : "s"}`;

/** The hero pill: one or two words of status, and whether the hero is live or needs the user. */
export function webPill(
  config: WebConfig, usage: WebUsageSummary | undefined, providers: readonly ProviderRecord[]
): { text: string; live: boolean; attention: boolean } {
  const active = config.providers.find((record) => record.id === config.activeProviderId);
  if (config.providers.length === 0) return { text: "No providers", live: false, attention: false };
  if (!config.enabled) return { text: "Off", live: false, attention: false };
  if (!active) return { text: "Pick a provider", live: false, attention: true };
  if (!keyReady(active, providers)) return { text: `${active.name} · needs a key`, live: false, attention: true };
  const searches = usageFor(usage, active.id).searches;
  return { text: searches > 0 ? `${active.name} · ${plural(searches, "search")}` : active.name, live: true, attention: false };
}

/** The page's "this month" heading for `YYYY-MM`. */
export function monthLabel(month: string): string {
  const [year, number] = month.split("-").map(Number);
  if (!year || !number) return "This month";
  return new Date(year, number - 1, 1).toLocaleDateString("en-GB", { month: "long", year: "numeric" });
}

export const BLANK_CUSTOM: CustomSearchSpec = {
  method: "GET", authHeader: "", authPrefix: "", queryParam: "q", countParam: "", bodyTemplate: "",
  resultsPath: "results", titlePath: "title", urlPath: "url", snippetPath: "", datePath: "",
};

/** A new provider's form: DeepSeek borrows the first linkable connection's key when there is one. */
export function draftFor(kind: SearchProviderKind, providers: readonly ProviderRecord[], existingNames: readonly string[]): SaveSearchProviderInput {
  const preset = presetFor(kind);
  const link = linkableConnections(kind, providers)[0];
  let name = preset.label;
  for (let index = 2; existingNames.includes(name); index += 1) name = `${preset.label} ${index}`;
  return {
    name,
    kind,
    keySource: !needsKey(kind, kind === "custom" ? BLANK_CUSTOM : undefined) ? { type: "none" } : link ? { type: "connection", providerId: link.id } : { type: "own" },
    apiKey: "",
    baseUrl: preset.needsUrl ? "" : null,
    custom: kind === "custom" ? { ...BLANK_CUSTOM } : null,
    priceSearchPer1k: preset.priceSearchPer1k ?? null,
    priceFetchPer1k: preset.priceFetchPer1k ?? null,
  };
}

export function draftFromRecord(record: SearchProviderRecord): SaveSearchProviderInput {
  return {
    id: record.id,
    name: record.name,
    kind: record.kind,
    keySource: record.keySource,
    apiKey: "",
    baseUrl: record.baseUrl ?? (presetFor(record.kind).needsUrl ? "" : null),
    custom: record.custom ? { ...BLANK_CUSTOM, ...record.custom } : record.kind === "custom" ? { ...BLANK_CUSTOM } : null,
    priceSearchPer1k: record.priceSearchPer1k ?? null,
    priceFetchPer1k: record.priceFetchPer1k ?? null,
  };
}

/** Whether the form has what Rust needs before Save or Test search can run. */
export function draftComplete(draft: SaveSearchProviderInput, saved?: SearchProviderRecord): boolean {
  if (!draft.name.trim()) return false;
  if (presetFor(draft.kind).needsUrl && !draft.baseUrl?.trim()) return false;
  if (draft.kind === "custom") {
    const spec = draft.custom;
    if (!spec || !spec.titlePath.trim() || !spec.urlPath.trim()) return false;
    if (spec.method === "GET" && !spec.queryParam?.trim()) return false;
    if (spec.method === "POST" && !spec.bodyTemplate?.includes("{{query}}")) return false;
  }
  if (draft.keySource.type === "own" && needsKey(draft.kind, draft.custom)) {
    const keptKey = saved?.keySource.type === "own" && saved.hasKey;
    if (!draft.apiKey?.trim() && !keptKey) return false;
  }
  return true;
}
