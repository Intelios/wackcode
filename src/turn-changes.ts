/**
 * Per-turn file changes summary card model: collects completed edits and writes within a turn,
 * deduplicates repeated touches to the same file, disambiguates duplicate basenames, and suppresses
 * redundant 1-file summaries when that edit is already visible as a bare standalone row.
 */
import type { NormalizedBlock, NormalizedMessage } from "./types";
import type { TranscriptLayout } from "./explore-utils";
import { blockKey } from "./explore-utils";
import { displayPath } from "./chat-utils";
import { callArguments, editStats, splitPathSubject, summarizeTool } from "./tool-utils";

export interface TurnFileCallEntry {
  key: string;
  call: NormalizedBlock;
  result: NormalizedBlock;
}

export interface TurnFileChange {
  path: string;
  displayPath: string;
  dir: string;
  name: string;
  /** Shortest distinguishing trailing directory suffix when two changed files share `name`. */
  folderHint?: string;
  kind: "create" | "edit";
  additions: number;
  deletions: number;
  calls: TurnFileCallEntry[];
}

export interface TurnFileChangesSummary {
  heading: string;
  files: TurnFileChange[];
  additions: number;
  deletions: number;
}

const nonEmptyPath = (call: NormalizedBlock): string | undefined => {
  const path = callArguments(call).path;
  return typeof path === "string" && path.trim() ? path.trim() : undefined;
};

/**
 * Shortest distinguishing folder suffixes for colliding basenames (matching Claude Desktop `ret`).
 * Example: `packages/web/src/App.tsx` + `packages/cli/src/App.tsx` -> `"web/src"` vs `"cli/src"`.
 */
function disambiguateFolders(entries: Omit<TurnFileChange, "folderHint">[]): TurnFileChange[] {
  const byName = new Map<string, Omit<TurnFileChange, "folderHint">[]>();
  for (const entry of entries) {
    const group = byName.get(entry.name) ?? [];
    group.push(entry);
    byName.set(entry.name, group);
  }

  const hints = new Map<string, string>();
  for (const group of byName.values()) {
    if (group.length < 2) continue;
    const segmentsByPath = group.map((entry) => {
      const normalized = entry.path.replace(/^\.\//, "").replace(/\/+$/, "");
      const parts = normalized.split("/").filter(Boolean);
      return parts.slice(0, -1);
    });
    const maxDepth = Math.max(...segmentsByPath.map((parts) => parts.length), 0);
    for (let depth = 1; depth <= maxDepth; depth += 1) {
      const candidate = segmentsByPath.map((parts) => parts.slice(-depth).join("/"));
      if (new Set(candidate).size === group.length) {
        group.forEach((entry, index) => {
          if (candidate[index]) hints.set(entry.path, candidate[index]);
        });
        break;
      }
    }
    group.forEach((entry, index) => {
      const fullDir = segmentsByPath[index].join("/");
      if (!hints.has(entry.path) && fullDir) hints.set(entry.path, fullDir);
    });
  }

  return entries.map((entry) => {
    const hint = hints.get(entry.path);
    return hint ? { ...entry, folderHint: hint } : entry;
  });
}

function isSoleVisibleEdit(
  messages: NormalizedMessage[],
  startIndex: number,
  endIndex: number,
  layout: TranscriptLayout,
  onlyFile: TurnFileChange
): boolean {
  const callIds = new Set(onlyFile.calls.map((c) => c.call.toolCallId).filter((id): id is string => Boolean(id)));
  const blocks = new Set(onlyFile.calls.map((c) => c.call));
  const hasCall = (block: NormalizedBlock) => blocks.has(block) || (block.toolCallId !== undefined && callIds.has(block.toolCallId));
  let visibleStandaloneSlots = 0;

  for (let index = startIndex; index < endIndex; index += 1) {
    const message = messages[index];
    if (message.role !== "assistant") continue;
    const slots = layout.messages.get(message.id) ?? [];
    for (const slot of slots) {
      if (slot.type === "explore") {
        // Any call belonging to the edited file inside an ExploreGroup means it isn't bare-visible.
        if (slot.group.items.some((item) => hasCall(item.block))) return false;
      } else if (slot.type === "coalesced") {
        if (slot.step.items.some((item) => hasCall(item.block))) visibleStandaloneSlots += 1;
      } else if (slot.type === "block") {
        const block = message.blocks[slot.index];
        if (block?.type === "tool-call" && hasCall(block)) {
          visibleStandaloneSlots += 1;
        }
      }
    }
  }
  return visibleStandaloneSlots === 1;
}

export function summarizeTurnFileChanges(
  messages: NormalizedMessage[],
  startIndex: number,
  endIndex: number,
  results: Map<string, NormalizedBlock>,
  options: { layout: TranscriptLayout; foldedByWorkTurn: boolean }
): TurnFileChangesSummary | undefined {
  const byPath = new Map<string, {
    path: string;
    kind: "create" | "edit";
    createdInTurn: boolean;
    additions: number;
    deletions: number;
    calls: TurnFileCallEntry[];
  }>();

  for (let index = startIndex; index < endIndex; index += 1) {
    const message = messages[index];
    if (message.role !== "assistant") continue;
    message.blocks.forEach((block, blockIndex) => {
      if (block.type !== "tool-call") return;
      if (block.toolName !== "edit" && block.toolName !== "write") return;
      const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
      if (!result || result.isError) return;
      const path = nonEmptyPath(block);
      if (!path) return;

      const details = (result.details ?? {}) as { created?: unknown; existed?: unknown };
      const isNew = block.toolName === "write" && (details.created === true || details.existed === false);
      let acc = byPath.get(path);
      if (!acc) {
        acc = {
          path,
          kind: isNew ? "create" : "edit",
          createdInTurn: isNew,
          additions: 0,
          deletions: 0,
          calls: []
        };
        byPath.set(path, acc);
      }

      if (block.toolName === "write") {
        const summary = summarizeTool(block, result);
        acc.additions = summary.additions ?? 0;
        if (acc.createdInTurn) acc.deletions = 0;
      } else {
        const stats = editStats(block, result);
        acc.additions += stats?.additions ?? 0;
        acc.deletions += stats?.deletions ?? 0;
      }
      acc.calls.push({ key: blockKey(message, blockIndex), call: block, result });
    });
  }

  if (byPath.size === 0) return undefined;

  const rawFiles: Omit<TurnFileChange, "folderHint">[] = [...byPath.values()].map((acc) => {
    const shown = displayPath(acc.path);
    const { dir, name } = splitPathSubject(shown);
    return {
      path: acc.path,
      displayPath: shown,
      dir,
      name,
      kind: acc.kind,
      additions: acc.additions,
      deletions: acc.deletions,
      calls: acc.calls
    };
  });

  if (rawFiles.length === 1 && !options.foldedByWorkTurn) {
    if (isSoleVisibleEdit(messages, startIndex, endIndex, options.layout, rawFiles[0])) {
      return undefined;
    }
  }

  const files = disambiguateFolders(rawFiles);
  const allCreate = files.every((f) => f.kind === "create");
  const allEdit = files.every((f) => f.kind === "edit");
  const count = files.length;
  const noun = count === 1 ? "file" : "files";
  const heading = allCreate ? `Created ${count} ${noun}` : allEdit ? `Edited ${count} ${noun}` : `Changed ${count} ${noun}`;

  let additions = 0;
  let deletions = 0;
  for (const file of files) {
    additions += file.additions;
    deletions += file.deletions;
  }

  return { heading, files, additions, deletions };
}
