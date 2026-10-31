import { describe, expect, it } from "vitest";
import { apportion, contextDetail, textTokens, type BreakdownGroup, type ContextDetailInput } from "./context-breakdown.js";

const input = (overrides: Partial<ContextDetailInput> = {}): ContextDetailInput => ({
  messages: { user: 2_000, assistant: 3_000, tool: 5_000 },
  contract: 0,
  appContext: 0,
  summary: 0,
  prompt: {
    preamble: 4_000,
    addendum: 1_000,
    addendumParts: [{ label: "App tool guide", tokens: 900 }],
    others: []
  },
  project: { total: 600, rows: [{ label: "AGENTS.md", tokens: 500 }] },
  skills: { total: 400, rows: [{ label: "logo-design", tokens: 300 }, { label: "playtest", tokens: 90 }] },
  memory: { total: 200, rows: [{ label: "A note", tokens: 150 }] },
  tools: [
    { id: "tools:builtin", label: "Built-in tools", tools: [{ label: "read", tokens: 800 }, { label: "bash", tokens: 700 }] },
    { id: "mcp:github", label: "MCP · GitHub", tools: [{ label: "create_issue", tokens: 900 }] }
  ],
  used: 20_000,
  reserveTokens: 16_384,
  ...overrides
});

const tokens = (groups: BreakdownGroup[]) => groups.reduce((sum, group) => sum + group.tokens, 0);
const childTokens = (group: BreakdownGroup) => (group.children ?? []).reduce((sum, child) => sum + child.tokens, 0);
const ids = (groups: BreakdownGroup[]) => groups.map((group) => group.id);

describe("textTokens", () => {
  it("rounds a text's characters up to whole tokens, as Pi does", () => {
    expect(textTokens(undefined)).toBe(0);
    expect(textTokens("")).toBe(0);
    expect(textTokens("a")).toBe(1);
    expect(textTokens("hello")).toBe(2);
    expect(textTokens("12345678")).toBe(2);
    expect(textTokens("123456789")).toBe(3);
  });
});

describe("apportion", () => {
  it("keeps proportions and always lands exactly on the total", () => {
    expect(apportion([1, 1, 1], 10)).toEqual([4, 3, 3]);
    expect(apportion([2, 1], 9)).toEqual([6, 3]);
    expect(apportion([10, 0], 3)).toEqual([3, 0]);
    expect(apportion([1, 1, 1], 0)).toEqual([0, 0, 0]);
    expect(apportion([], 5)).toEqual([]);
  });

  it("ignores values that cannot be tokens", () => {
    expect(apportion([Number.NaN, -4, 1], 3)).toEqual([0, 0, 3]);
  });

  it("sums to the total for totals smaller than the row count", () => {
    expect(apportion([1, 1, 1, 1], 2).reduce((sum, value) => sum + value, 0)).toBe(2);
  });
});

describe("contextDetail", () => {
  it("makes every row add up to the provider's own count", () => {
    const detail = contextDetail(input());
    expect(tokens(detail.groups)).toBe(20_000);
    for (const group of detail.groups) {
      if (group.children) expect(childTokens(group)).toBe(group.tokens);
    }
  });

  it("scales down when the estimates overshoot, and up when they undershoot", () => {
    expect(tokens(contextDetail(input({ used: 9_300 })).groups)).toBe(9_300);
    expect(tokens(contextDetail(input({ used: 40_000 })).groups)).toBe(40_000);
  });

  it("orders the groups the way the panel reads them", () => {
    const detail = contextDetail(input({ contract: 700, appContext: 250, summary: 4_000 }));
    expect(ids(detail.groups)).toEqual([
      "messages", "summary", "contract", "app-context",
      "tools:builtin", "mcp:github",
      "system", "project", "skills", "memory"
    ]);
  });

  it("leaves out groups the request did not carry", () => {
    const detail = contextDetail(input({ memory: { total: 0, rows: [] } }));
    expect(ids(detail.groups)).not.toContain("memory");
    expect(ids(detail.groups)).not.toContain("summary");
    expect(ids(detail.groups)).not.toContain("contract");
    expect(ids(detail.groups)).not.toContain("app-context");
  });

  it("splits messages into user, assistant and tool results", () => {
    const messages = contextDetail(input()).groups.find((group) => group.id === "messages");
    expect(messages?.children?.map((child) => child.label)).toEqual(["User messages", "Assistant replies", "Tool results"]);
    expect(messages?.tokens).toBe(messages?.children?.reduce((sum, child) => sum + child.tokens, 0));
  });

  it("names the addendum remainder the app cannot label", () => {
    const detail = contextDetail(input({
      prompt: { preamble: 1_000, addendum: 1_000, addendumParts: [{ label: "App tool guide", tokens: 400 }], others: [] }
    }));
    const system = detail.groups.find((group) => group.id === "system");
    const labels = system?.children?.map((child) => child.label);
    expect(labels).toContain("Core prompt");
    expect(labels).toContain("App tool guide");
    expect(labels).toContain("Other instructions");
  });

  it("keeps a section a package added as its own row", () => {
    const detail = contextDetail(input({
      prompt: { preamble: 1_000, addendum: 0, addendumParts: [], others: [{ label: "Rules", tokens: 300 }] }
    }));
    const system = detail.groups.find((group) => group.id === "system");
    expect(system?.children?.map((child) => child.label)).toContain("Rules");
    expect(system?.tokens).toBeGreaterThanOrEqual(1_300);
  });

  it("shows a group's items only when there is more than one worth expanding", () => {
    const detail = contextDetail(input({ memory: { total: 200, rows: [{ label: "Only note", tokens: 150 }] } }));
    const memory = detail.groups.find((group) => group.id === "memory");
    expect(memory?.tokens).toBeGreaterThan(0);
    expect(memory?.children).toBeUndefined();
    const skills = detail.groups.find((group) => group.id === "skills");
    expect(skills?.children?.map((child) => child.label)).toEqual(["logo-design", "playtest"]);
  });

  it("carries the compaction reserve through untouched", () => {
    expect(contextDetail(input()).reserveTokens).toBe(16_384);
    expect(contextDetail(input({ reserveTokens: 8_000 })).reserveTokens).toBe(8_000);
  });

  it("returns nothing usable when there is no context to explain", () => {
    const detail = contextDetail(input({ used: 0 }));
    expect(detail.groups).toEqual([]);
  });
});
