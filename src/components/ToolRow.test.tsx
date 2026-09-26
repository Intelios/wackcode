import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { NormalizedBlock } from "../types";
import { ToolRow } from "./ToolRow";

afterEach(cleanup);

const editCall: NormalizedBlock = {
  type: "tool-call",
  toolCallId: "call-1",
  toolName: "edit",
  arguments: { path: "src/app.ts" },
  text: ""
};

const editResult: NormalizedBlock = {
  type: "tool-result",
  toolCallId: "call-1",
  isError: false,
  text: "Edited",
  details: { diff: "@@ -1 +1 @@\n-old text\n+const x = 1;\n" }
};

describe("ToolRow", () => {
  it("tokenizes an edit diff with the edited file's language", () => {
    render(<ToolRow call={editCall} result={editResult} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    expect(screen.getByText("const")).toHaveClass("hljs-keyword");
    // Markers and header lines stay plain; the code after the marker is tokenized.
    expect(document.querySelector(".tool-diff .deletion")?.textContent).toBe("-old text\n");
    expect(document.querySelector(".tool-diff .hunk")?.textContent).toBe("@@ -1 +1 @@\n");
    expect(document.querySelector(".tool-diff .deletion .hljs")).toBeNull();
  });

  it("falls back to plain diff text without a language", () => {
    const noLangCall = { ...editCall, arguments: { path: "artifact.unknownext" } };
    render(<ToolRow call={noLangCall} result={editResult} />);
    fireEvent.click(screen.getByRole("button", { name: /Edited/ }));
    expect(screen.getByText("+const x = 1;")).toBeInTheDocument();
    expect(document.querySelector(".hljs")).toBeNull();
  });
});
