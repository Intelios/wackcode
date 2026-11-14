import { describe, expect, it } from "vitest";
import { activitiesOf, groupActivities, summarizeActivityFold } from "./chat-activity";
import type { NormalizedBlock } from "../../types";

const call = (id: string, toolName: string, args: Record<string, unknown> = {}): NormalizedBlock =>
  ({ type: "tool-call", toolName, toolCallId: id, arguments: args });
const ok = (id: string): [string, NormalizedBlock] => [id, { type: "tool-result", toolCallId: id, text: "ok" }];

describe("summarizeActivityFold", () => {
  it("deduplicates scratchpad file paths and prioritizes writes/web over reads", () => {
    const calls = [
      call("1", "read", { path: "notes/a.md" }),
      call("2", "read", { path: "notes/a.md" }),
      call("3", "write", { path: "notes/summary.md" }),
      call("4", "web_fetch", { url: "https://example.com" }),
      call("5", "web_fetch", { url: "https://example.org" })
    ];
    const results = new Map<string, NormalizedBlock>(["1", "2", "3", "4", "5"].map((id) => ok(id)));
    const activities = activitiesOf(calls, results, false);
    expect(summarizeActivityFold(activities)).toBe("Jotted down summary.md, looked up 2 pages, +1 more");
  });

  it("summarizes grouped activities specifically instead of 'Did N things'", () => {
    const calls = [
      call("1", "web_search", { query: "q1" }),
      call("2", "web_search", { query: "q2" }),
      call("3", "web_fetch", { url: "https://example.com" })
    ];
    const results = new Map<string, NormalizedBlock>(["1", "2", "3"].map((id) => ok(id)));
    const groups = groupActivities(activitiesOf(calls, results, false));
    expect(groups).toHaveLength(1);
    expect(groups[0].type === "fold" && groups[0].label).toBe("Searched the web 2 times, looked up 1 page");
  });
});
