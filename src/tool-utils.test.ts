import { describe, expect, it } from "vitest";
import { bashExploreKind, diffStats, displayAgentName, editStats, exploreKind, groupTools, parseSubagentDetails, pendingSubagentDetails, pruneDisabledTools, sameToolCatalog, subagentDetailsFor, summarizeTool } from "./tool-utils";
import type { NormalizedBlock, NormalizedMessage, ToolCatalogEntry } from "./types";

function call(toolName: string, args: unknown): NormalizedBlock {
  return { type: "tool-call", toolName, toolCallId: "c1", arguments: args };
}

describe("summarizeTool", () => {
  it("names an MCP tool and its server", () => {
    const summary = summarizeTool(call("mcp__github__search_issues", { q: "bug" }));
    expect(summary).toMatchObject({ activeVerb: "Calling", doneVerb: "Called", subject: "search_issues (github)" });
    expect(summarizeTool(call("mcp_not_really", {})).doneVerb).toBe("mcp_not_really");
  });

  it("summarizes read calls", () => {
    const summary = summarizeTool(call("read", { path: "/Users/jack/app/src/main.ts" }));
    expect(summary.doneVerb).toBe("Read");
    expect(summary.subject).toBe("…/src/main.ts");
  });

  it("summarizes memory saves, recalls and forgets", () => {
    const saved = summarizeTool(call("memory_save", { type: "feedback", title: "Run worker tests", description: "", body: "…" }));
    expect(saved.doneVerb).toBe("Saved memory");
    expect(saved.subject).toBe("Run worker tests");
    const updated = summarizeTool(call("memory_save", { type: "feedback", name: "feedback_run-worker-tests", title: "Run worker tests", description: "", body: "…" }));
    expect(updated.doneVerb).toBe("Updated memory");
    const recalled = summarizeTool(call("memory_recall", { names: ["user_prefers", "feedback_run-worker-tests"] }));
    expect(recalled.doneVerb).toBe("Recalled memory");
    expect(recalled.subject).toBe("user_prefers, feedback_run-worker-tests");
    expect(summarizeTool(call("memory_forget", { name: "user_prefers" })).subject).toBe("user_prefers");
  });

  it("summarizes web_fetch calls with a short URL", () => {
    const summary = summarizeTool(call("web_fetch", { url: "https://www.example.com/docs/intro?ref=x" }));
    expect(summary.activeVerb).toBe("Fetching");
    expect(summary.doneVerb).toBe("Fetched");
    expect(summary.subject).toBe("example.com/docs/intro");
    expect(summarizeTool(call("web_fetch", { url: "https://example.com/" })).subject).toBe("example.com");
    expect(summarizeTool(call("web_fetch", { url: "http://localhost:43127/v1/models" })).subject).toBe("localhost:43127/v1/models");
    expect(summarizeTool(call("web_fetch", { url: "not a url" })).subject).toBe("not a url");
  });

  it("summarizes bash calls with the command", () => {
    const summary = summarizeTool(call("bash", { command: "pnpm test" }));
    expect(summary.doneVerb).toBe("Ran");
    expect(summary.subject).toBe("pnpm test");
  });

  it("computes edit stats from the result diff", () => {
    const result: NormalizedBlock = {
      type: "tool-result",
      toolCallId: "c1",
      details: { diff: "--- a/f.ts\n+++ b/f.ts\n@@ -1,2 +1,2 @@\n-old\n+new\n+extra\n" }
    };
    const summary = summarizeTool(call("edit", { path: "f.ts", edits: [] }), result);
    expect(summary.doneVerb).toBe("Edited");
    expect(summary.additions).toBe(2);
    expect(summary.deletions).toBe(1);
  });

  it("falls back to edit arguments when no diff is present", () => {
    const summary = summarizeTool(call("edit", { path: "f.ts", edits: [{ oldText: "a\nb", newText: "a\nb\nc" }] }));
    expect(summary.additions).toBe(3);
    expect(summary.deletions).toBe(2);
  });

  it("summarizes write calls with a line count", () => {
    const summary = summarizeTool(call("write", { path: "src/new.ts", content: "one\ntwo\n" }));
    expect(summary.subject).toBe("src/new.ts");
    expect(summary.additions).toBe(2);
  });

  it("handles streamed arguments still arriving as a string", () => {
    const summary = summarizeTool(call("read", "{\"path\": \"src/x.ts\"}"));
    expect(summary.subject).toBe("src/x.ts");
  });

  it("falls back gracefully for unknown tools", () => {
    const summary = summarizeTool(call("mcp_deploy", {}));
    expect(summary.doneVerb).toBe("mcp_deploy");
  });
});

describe("diffStats", () => {
  it("ignores file header lines", () => {
    expect(diffStats("+++ b/a\n--- a/a\n+x\n-y")).toEqual({ additions: 1, deletions: 1 });
  });
  it("returns undefined without changes", () => {
    expect(diffStats("no diff")).toBeUndefined();
    expect(diffStats(undefined)).toBeUndefined();
  });
});

describe("tool catalogue", () => {
  const builtin = (name: string, available = true): ToolCatalogEntry =>
    ({ name, description: `${name} tool`, source: { kind: "builtin" }, available });
  const fromPackage = (name: string, packageId: string): ToolCatalogEntry =>
    ({ name, description: "", source: { kind: "package", packageId }, available: true });

  it("puts built-ins first and sorts packages, keeping unattributed tools in Other", () => {
    const groups = groupTools([
      fromPackage("web_search", "npm:pi-web-access"),
      builtin("read"),
      { name: "mystery", description: "", source: { kind: "package" }, available: true },
      fromPackage("delegate", "npm:pi-subagents"),
      builtin("bash")
    ]);
    expect(groups.map((group) => group.id)).toEqual(["builtin", "npm:pi-subagents", "npm:pi-web-access", "other"]);
    expect(groups[0].tools.map((tool) => tool.name)).toEqual(["bash", "read"]);
    expect(groups[3].label).toBe("Other");
  });

  it("leaves MCP and WackCode tools out: their switches live elsewhere", () => {
    const groups = groupTools([
      builtin("read"),
      { name: "mcp__github__search", description: "", source: { kind: "mcp", serverId: "mcp-1" }, available: true },
      { name: "todo", description: "", source: { kind: "wackcode" }, available: true }
    ]);
    expect(groups.map((group) => group.id)).toEqual(["builtin"]);
  });

  it("drops disabled names whose tool no longer exists, but keeps them when the catalogue is empty", () => {
    const catalog = [builtin("read"), builtin("bash")];
    expect(pruneDisabledTools(["bash", "gone"], catalog)).toEqual(["bash"]);
    // An empty catalogue means "not loaded yet", not "nothing installed" — never prune then.
    expect(pruneDisabledTools(["bash", "gone"], [])).toEqual(["bash", "gone"]);
  });

  it("treats an availability change as a different catalogue so Settings re-renders", () => {
    expect(sameToolCatalog([builtin("grep")], [builtin("grep")])).toBe(true);
    expect(sameToolCatalog([builtin("grep")], [builtin("grep", false)])).toBe(false);
    expect(sameToolCatalog([builtin("grep")], [])).toBe(false);
  });
});

describe("sub-agent details", () => {
  const result = { agent: "scout", task: "t", readOnly: true, status: "done", activity: [{ tool: "ls", subject: "." }, { nope: true }], usage: { input: 3 } };

  it("reads the versioned shape the worker stores, filling gaps and dropping junk", () => {
    const parsed = parseSubagentDetails({ v: 1, mode: "parallel", results: [result] });
    expect(parsed?.mode).toBe("parallel");
    expect(parsed?.results[0].activity).toEqual([{ tool: "ls", subject: "." }]);
    expect(parsed?.results[0].usage).toEqual({ input: 3, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 });
  });

  it("treats other versions and malformed results as absent", () => {
    expect(parseSubagentDetails(undefined)).toBeUndefined();
    expect(parseSubagentDetails({})).toBeUndefined();
    expect(parseSubagentDetails({ v: 2, mode: "single", results: [result] })).toBeUndefined();
    expect(parseSubagentDetails({ v: 1, mode: "single", results: [] })).toBeUndefined();
    expect(parseSubagentDetails({ v: 1, mode: "single", results: [{ ...result, status: "exploded" }] })).toBeUndefined();
  });

  it("builds a queued card from a call's arguments, even while they stream", () => {
    expect(pendingSubagentDetails(call("subagent", { agent: "scout", task: "look" }))?.results[0]).toMatchObject({ agent: "scout", status: "queued" });
    expect(pendingSubagentDetails(call("subagent", { tasks: [{ agent: "a", task: "1" }, { agent: "b", task: "2" }] }))?.mode).toBe("parallel");
    expect(pendingSubagentDetails(call("subagent", '{"agent":"sco'))).toBeUndefined();
    expect(summarizeTool(call("subagent", { tasks: [{}, {}] })).activeVerb).toBe("Running 2 sub-agents");
  });

  it("titles an agent's name for its chip", () => {
    expect(displayAgentName("scout")).toBe("Scout");
    expect(displayAgentName("code-reviewer")).toBe("Code Reviewer");
    expect(displayAgentName("db_migrator2")).toBe("Db Migrator2");
    expect(displayAgentName("-")).toBe("-");
  });

  it("finds a call's details by the chips' precedence: result, then live update, then arguments", () => {
    const toolCall: NormalizedBlock = { type: "tool-call", toolName: "subagent", toolCallId: "call-1", arguments: { agent: "scout", task: "look" } };
    const asked: NormalizedMessage = { id: "m1", role: "assistant", blocks: [toolCall] };
    const answered: NormalizedMessage = { id: "m2", role: "tool", blocks: [{ type: "tool-result", toolName: "subagent", toolCallId: "call-1", details: { v: 1, mode: "single", results: [{ ...result, status: "done" }] } }] };
    const live = { "call-1": { v: 1, mode: "single", results: [{ ...result, status: "running" }] } };

    expect(subagentDetailsFor([asked, answered], undefined, live, "call-1", true)).toMatchObject({ finished: true, details: { results: [{ status: "done" }] } });
    expect(subagentDetailsFor([asked], undefined, live, "call-1", true)).toMatchObject({ finished: false, details: { results: [{ status: "running" }] } });
    expect(subagentDetailsFor([asked], undefined, {}, "call-1", true)).toMatchObject({ finished: false, details: { results: [{ status: "queued" }] } });
    // Still streaming in.
    expect(subagentDetailsFor([], asked, {}, "call-1", true)?.details.results[0].agent).toBe("scout");
    // Cut off without a result, or not in the conversation at all.
    expect(subagentDetailsFor([asked], undefined, live, "call-1", false)).toBeUndefined();
    expect(subagentDetailsFor([asked, answered], undefined, live, "call-2", true)).toBeUndefined();
  });
});

describe("exploreKind", () => {
  it("sorts Pi's read-only tools and leaves every other tool out", () => {
    expect(exploreKind(call("read", { path: "a.ts" }))).toBe("file");
    expect(exploreKind(call("grep", { pattern: "x" }))).toBe("search");
    expect(exploreKind(call("find", { pattern: "*.ts" }))).toBe("search");
    expect(exploreKind(call("ls", { path: "src" }))).toBe("list");
    for (const name of ["edit", "write", "subagent", "todo", "web_fetch", "plan_mode_complete", "mcp__github__search"]) {
      expect(exploreKind(call(name, {}))).toBeUndefined();
    }
    expect(exploreKind(call("bash", { command: "ls src" }))).toBe("list");
    expect(exploreKind(call("bash", { command: "pnpm test" }))).toBeUndefined();
    expect(exploreKind(call("bash", "{\"command\": \"ls"))).toBeUndefined();
  });
});

describe("bashExploreKind", () => {
  it("classifies read-only commands, pipelines and chains by their first explorer", () => {
    const cases: [string, ReturnType<typeof bashExploreKind>][] = [
      ["ls /Users/jack/wackcode/src/components/", "list"],
      ["grep -rn \"PackagesSection\" src --include=*.tsx --include=*.ts | head -30", "search"],
      ["rg -n 'a | b' src", "search"],
      ["cat package.json | head -40", "file"],
      ["sed -n 1,120p src/App.tsx", "file"],
      ["sed -n '200,235p' src/styles.css; sed -nE '1p' x", "file"],
      ["cd /repo && git status --short", "command"],
      ["git log --oneline -3", "command"],
      ["git ls-files | wc -l", "list"],
      ["find src -name '*.test.ts' 2>/dev/null | sort | uniq", "search"],
      ["ls src 2>&1 | head", "list"],
      ["ls missing || echo none", "list"],
      ["wc -l src/*.ts\n", "file"],
      ["pwd", "list"],
      ["ls \\\n  src", "list"],
      ["# look around\nls", "list"],
      ["grep -n \"end$\" file", "search"]
    ];
    for (const [command, kind] of cases) expect(bashExploreKind(command), command).toBe(kind);
  });

  it("leaves anything that might write or run something else visible", () => {
    const cases = [
      "",
      "cd src",
      "echo hi",
      "pnpm test",
      "rm -rf dist",
      "ls > files.txt",
      "cat a >> b",
      "cat <<EOF\nhi\nEOF",
      "grep x < file",
      "ls $(pwd)",
      "ls `pwd`",
      "echo \"$(rm -rf /)\"",
      "ls & rm x",
      "(cd src && ls)",
      "sed -i 's/a/b/' file",
      "sed -n 's/a/b/w out' file",
      "sed -n -f script.sed file",
      "sed '1,10p' file",
      "find . -name '*.tmp' -delete",
      "find . -exec rm {} \\;",
      "fd -x rm",
      "rg --pre ./script x",
      "sort -o out.txt in.txt",
      "sort -ro out.txt in.txt",
      "uniq in.txt out.txt",
      "git diff --output=patch.diff",
      "git checkout main",
      "git -C repo status",
      "ls | xargs rm",
      "ls | tee out.txt",
      "ls &&",
      "ls |",
      "cat 'unterminated",
      "FOO=1 ls",
      "ls |& cat"
    ];
    for (const command of cases) expect(bashExploreKind(command), command).toBeUndefined();
  });
});
