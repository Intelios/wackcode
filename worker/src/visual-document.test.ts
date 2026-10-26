import { describe, expect, it } from "vitest";
import { visualDocument } from "./visual-document.js";

const visual = { role: "toolResult", toolName: "show_visual", toolCallId: "visual", details: { v: 1, inlined: true, html: '<img src="data:image/png;base64,original">' } };

describe("saved visual documents", () => {
  it("fetches only the requested successful visual result", () => {
    expect(visualDocument([{ ...visual, toolCallId: "other" }, visual], "visual")).toBe(visual.details.html);
    for (const message of [
      { ...visual, isError: true },
      { ...visual, role: "assistant" },
      { ...visual, toolName: "read" },
      { ...visual, details: { ...visual.details, inlined: false } },
      { ...visual, details: { ...visual.details, html: undefined } },
    ]) expect(() => visualDocument([message], "visual")).toThrow("no longer in this chat");
    expect(() => visualDocument([visual], "missing")).toThrow("no longer in this chat");
    expect(() => visualDocument([], "visual")).toThrow("no longer in this chat");
  });

  it("uses the latest result and refuses a failed replacement", () => {
    expect(visualDocument([visual, { ...visual, details: { ...visual.details, html: "latest" } }], "visual")).toBe("latest");
    expect(() => visualDocument([visual, { ...visual, isError: true }], "visual")).toThrow();
  });
});
