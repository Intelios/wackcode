import { describe, expect, it } from "vitest";
import { conversationReadabilityConstraints } from "./image-readability-constraints";
import type { NormalizedBlock, NormalizedMessage } from "./types";

const message = (id: string, role: NormalizedMessage["role"], blocks: NormalizedBlock[], stopReason?: string): NormalizedMessage => ({ id, role, blocks, ...(stopReason ? { stopReason } : {}) });
const call = (toolCallId: string, toolName = "show_visual"): NormalizedBlock => ({ type: "tool-call", toolCallId, toolName, arguments: { title: "Chart", html: "<p>Chart</p>" } });
const result = (toolCallId: string, extra: Partial<NormalizedBlock> = {}): NormalizedBlock => ({ type: "tool-result", toolCallId, details: { v: 1, title: "Chart" }, ...extra });

describe("conversation image-readability constraints", () => {
  it("leaves ordinary conversations free to adapt", () => {
    expect(conversationReadabilityConstraints([])).toEqual({ visual: false, warning: false });
    expect(conversationReadabilityConstraints([
      message("u", "user", [{ type: "text", text: "show_visual was mentioned" }]),
      message("a", "assistant", [{ type: "text", text: "Ready" }], "stop")
    ])).toEqual({ visual: false, warning: false });
  });

  it("finds a successful visual outside the transcript's newest turns", () => {
    const history = [
      message("a", "assistant", [call("chart")]),
      message("r", "tool", [result("chart", { details: { v: 1, title: "Saved chart", inlined: true } })]),
      ...Array.from({ length: 100 }, (_, index) => message(`later-${index}`, "assistant", [{ type: "text", text: "Later reply" }]))
    ];
    expect(conversationReadabilityConstraints(history)).toEqual({ visual: true, warning: false });
  });

  it("recognizes named results without needing a retained call", () => {
    expect(conversationReadabilityConstraints([
      message("r", "tool", [result("chart", { toolName: "show_visual" })])
    ])).toEqual({ visual: true, warning: false });
  });

  it("matches call ids across history regardless of message ordering", () => {
    expect(conversationReadabilityConstraints([
      message("r", "tool", [result("chart")]),
      message("a", "assistant", [call("chart")])
    ])).toEqual({ visual: true, warning: false });
  });

  it("protects legacy visuals rendered from call arguments without saved metadata", () => {
    for (const details of [undefined, { v: 2, title: "Old chart" }]) {
      expect(conversationReadabilityConstraints([
        message("a", "assistant", [call("chart")]),
        message("r", "tool", [result("chart", { details })])
      ])).toEqual({ visual: true, warning: false });
    }
  });

  it("ignores running, failed, malformed and unrelated visual-like results", () => {
    for (const blocks of [
      [call("chart")],
      [call("chart"), result("chart", { isError: true })],
      [{ ...call("chart"), arguments: {} }, result("chart", { details: undefined })],
      [{ ...call("chart"), arguments: { title: "Chart", html: " " } }, result("chart", { details: { v: 2, title: "Chart" } })],
      [call("chart"), result("different")],
      [call("chart", "preview_visual"), result("chart", { toolName: "preview_visual" })],
      [result("unknown")]
    ]) {
      expect(conversationReadabilityConstraints([message("m", "assistant", blocks)])).toEqual({ visual: false, warning: false });
    }
  });

  it("protects Stopped labels from any historical or partial aborted message", () => {
    const stopped = message("stopped", "assistant", [], "aborted");
    expect(conversationReadabilityConstraints([stopped, message("later", "assistant", [{ type: "text", text: "Continuing" }])])).toEqual({ visual: false, warning: true });
    expect(conversationReadabilityConstraints([], stopped)).toEqual({ visual: false, warning: true });
    expect(conversationReadabilityConstraints([message("error", "assistant", [], "error")])).toEqual({ visual: false, warning: false });
  });

  it("includes a partial's calls and successful results in the same conversation", () => {
    const history = [message("r", "tool", [result("chart")])];
    const partial = message("live", "assistant", [call("chart")], "aborted");
    expect(conversationReadabilityConstraints(history, partial)).toEqual({ visual: true, warning: true });
    expect(history).toHaveLength(1);
    expect(partial.blocks).toEqual([call("chart")]);
  });
});
