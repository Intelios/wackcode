import { describe, expect, it } from "vitest";
import type { CustomProviderRecord, SearchProviderRecord, WebConfig } from "./types";
import { draftComplete, draftFor, estimatedCost, formatCost, keyState, linkableConnections, monogram, needsKey, siteInitial, webPill } from "./web-presets";

const connection = (id: string, baseUrl: string, patch: Partial<CustomProviderRecord> = {}): CustomProviderRecord => ({
  id, name: id, kind: "custom", baseUrl, apiFormat: "openai-completions", models: [], createdAt: "", updatedAt: "",
  hasApiKey: true, connected: true, enabled: true, ...patch,
});

const provider = (patch: Partial<SearchProviderRecord> = {}): SearchProviderRecord => ({
  id: "s1", name: "DeepSeek", kind: "deepseek", keySource: { type: "own" }, hasKey: true,
  priceSearchPer1k: 0.75, priceFetchPer1k: 0.75, createdAt: "", updatedAt: "", ...patch,
});

describe("Settings › Web helpers", () => {
  it("offers only enabled API-key connections on the preset's own origin", () => {
    const providers = [
      connection("ds", "https://api.deepseek.com/v1"),
      connection("ds-off", "https://api.deepseek.com", { enabled: false }),
      connection("ds-nokey", "https://api.deepseek.com", { hasApiKey: false }),
      connection("openai", "https://api.openai.com/v1"),
    ];
    expect(linkableConnections("deepseek", providers).map((entry) => entry.id)).toEqual(["ds"]);
    expect(linkableConnections("brave", providers)).toEqual([]);
  });

  it("says where the key comes from", () => {
    const providers = [connection("ds", "https://api.deepseek.com"), connection("other", "https://api.openai.com")];
    expect(keyState(provider(), providers)).toEqual({ tone: "on", label: "Key saved" });
    expect(keyState(provider({ hasKey: false }), providers).tone).toBe("off");
    expect(keyState(provider({ keySource: { type: "connection", providerId: "ds" } }), providers).label).toBe("Uses ‘ds’ key");
    expect(keyState(provider({ keySource: { type: "connection", providerId: "other" } }), providers).tone).toBe("off");
    expect(keyState(provider({ keySource: { type: "connection", providerId: "gone" } }), providers).label).toBe("Linked connection removed");
    expect(keyState(provider({ kind: "searxng", keySource: { type: "none" } }), providers).label).toBe("No key needed");
  });

  it("estimates cost from billed requests and the user's prices", () => {
    const counters = { searches: 1200, fetches: 10, billedSearches: 1000, billedFetches: 4, failed: 3 };
    expect(estimatedCost(provider(), counters)).toBeCloseTo(0.753);
    expect(estimatedCost(provider({ priceSearchPer1k: null, priceFetchPer1k: null }), counters)).toBeUndefined();
    expect(formatCost(0)).toBe("$0.00");
    expect(formatCost(0.004)).toBe("<$0.01");
    expect(formatCost(0.753)).toBe("$0.75");
    expect(formatCost(123.4)).toBe("$123");
  });

  it("puts the state in the pill", () => {
    const config: WebConfig = { enabled: true, providers: [provider()], activeProviderId: "s1", routeFetch: false };
    expect(webPill(config, { month: "2026-10", providers: [{ id: "s1", searches: 142, fetches: 0, billedSearches: 140, billedFetches: 0, failed: 2 }] }, []))
      .toEqual({ text: "DeepSeek · 142 searches", live: true, attention: false });
    expect(webPill({ ...config, enabled: false }, undefined, []).text).toBe("Off");
    expect(webPill({ ...config, activeProviderId: null }, undefined, []).attention).toBe(true);
    expect(webPill({ ...config, providers: [provider({ hasKey: false })] }, undefined, [])).toMatchObject({ live: false, attention: true });
    expect(webPill({ ...config, providers: [] }, undefined, []).text).toBe("No providers");
  });

  it("starts a DeepSeek form on a connection's key when one can lend it, and names it uniquely", () => {
    const draft = draftFor("deepseek", [connection("ds", "https://api.deepseek.com")], ["DeepSeek"]);
    expect(draft).toMatchObject({ name: "DeepSeek 2", keySource: { type: "connection", providerId: "ds" }, priceSearchPer1k: 0.75 });
    expect(draftFor("deepseek", [], []).keySource).toEqual({ type: "own" });
    expect(draftFor("searxng", [], []).keySource).toEqual({ type: "none" });
  });

  it("knows when a form can be saved", () => {
    expect(draftComplete({ ...draftFor("brave", [], []), apiKey: "" })).toBe(false);
    expect(draftComplete({ ...draftFor("brave", [], []), apiKey: "key" })).toBe(true);
    expect(draftComplete({ ...draftFor("brave", [], []), id: "s1" }, provider({ kind: "brave" }))).toBe(true);
    expect(draftComplete(draftFor("searxng", [], []))).toBe(false);
    expect(draftComplete({ ...draftFor("searxng", [], []), baseUrl: "http://localhost:8888" })).toBe(true);
    const custom = { ...draftFor("custom", [], []), baseUrl: "https://x.example/search" };
    expect(draftComplete(custom)).toBe(true);
    expect(draftComplete({ ...custom, custom: { ...custom.custom!, method: "POST", bodyTemplate: "{}" } })).toBe(false);
    expect(needsKey("custom", { ...custom.custom!, authHeader: "X-Key" })).toBe(true);
  });

  it("makes monograms", () => {
    expect(monogram("DeepSeek")).toBe("D");
    expect(monogram("Brave Search")).toBe("BS");
    expect(monogram("docs.rs")).toBe("DR");
    expect(monogram("  ")).toBe("?");
    expect(siteInitial("blog.rust-lang.org")).toBe("R");
    expect(siteInitial("www.github.com")).toBe("G");
    expect(siteInitial("news.bbc.co.uk")).toBe("B");
    expect(siteInitial("localhost")).toBe("L");
  });
});
