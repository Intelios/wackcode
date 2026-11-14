import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TurnFileChangesSummary } from "../turn-changes";
import { TurnChangesCard } from "./TurnChangesCard";

afterEach(cleanup);

function makeSummary(count: number): TurnFileChangesSummary {
  const files = Array.from({ length: count }, (_, i) => ({
    path: `src/f${i + 1}.ts`,
    displayPath: `src/f${i + 1}.ts`,
    dir: "src/",
    name: `f${i + 1}.ts`,
    kind: "edit" as const,
    additions: i + 1,
    deletions: 1,
    calls: [{
      key: `c-${i}`,
      call: { type: "tool-call" as const, toolName: "edit", toolCallId: `c-${i}`, arguments: { path: `src/f${i + 1}.ts` } },
      result: { type: "tool-result" as const, toolCallId: `c-${i}`, text: "ok", details: { diff: `-old${i}\n+new${i}` } }
    }]
  }));
  return {
    heading: `Edited ${count} files`,
    files,
    additions: files.reduce((s, f) => s + f.additions, 0),
    deletions: files.length
  };
}

describe("TurnChangesCard", () => {
  it("renders up to 3 files with Show more toggle, inline diff expansion, and Changes action", () => {
    const onOpenChanges = vi.fn();
    const { container } = render(<TurnChangesCard summary={makeSummary(5)} onOpenChanges={onOpenChanges} />);
    expect(screen.getByRole("region", { name: "Edited 5 files" })).toBeInTheDocument();
    expect(container.querySelectorAll(".turn-changes-file")).toHaveLength(3);

    const more = screen.getByRole("button", { name: "Show 2 more" });
    fireEvent.click(more);
    expect(container.querySelectorAll(".turn-changes-file")).toHaveLength(5);
    expect(screen.getByRole("button", { name: "Show less" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Inspect changes to src/f1.ts" }));
    expect(screen.getByText(/\+new0/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Changes" }));
    expect(onOpenChanges).toHaveBeenCalledTimes(1);
  });
});
