/**
 * Coding chats' compaction: Pi's summary, steered by WackCode's instructions, plus the sections
 * `summary.ts` builds in code. Pi still chooses the cut point and the kept tail, and its own
 * summarizer runs through the host (`runDefaultCompaction`), so model routing, credentials,
 * retries and the usage ledger are exactly Pi's. Chat mode chats don't load this.
 *
 * Anything that goes wrong falls back to Pi's default compaction rather than failing it, and
 * an installed package that compacts on its own is left to do so (the host returns undefined).
 */
import type { ExtensionAPI, SessionBeforeCompactEvent, SessionEntry } from "@earendil-works/pi-coding-agent";
import { getLatestCompactionEntry } from "@earendil-works/pi-coding-agent";
import type { TodoState } from "../../protocol.js";
import type { BuiltinHost } from "../host.js";
import {
  COMPACTION_DETAILS_VERSION,
  type WackCodeCompactionDetails,
  buildSummary,
  isWackCodeDetails,
  keepUserMessages,
  mergeSkills,
  skillRefs,
  summaryInstructions,
  userMessageText,
} from "./summary.js";

export function createCompactionExtension(host: Pick<BuiltinHost, "runDefaultCompaction" | "notice">, todo: () => TodoState) {
  return function compaction(pi: ExtensionAPI) {
    pi.on("session_before_compact", async (event: SessionBeforeCompactEvent) => {
      const { preparation, signal } = event;
      const previous = latestDetails(event.branchEntries);
      if (previous) {
        // Pi updates the previous summary; give it Pi's part alone, and the file lists Pi
        // only carries forward from its own compactions.
        preparation.previousSummary = previous.body;
        for (const path of previous.readFiles) preparation.fileOps.read.add(path);
        for (const path of previous.modifiedFiles) preparation.fileOps.edited.add(path);
      }
      let result;
      try {
        result = await host.runDefaultCompaction(preparation, summaryInstructions(event.customInstructions), signal, event.reason);
      } catch (error) {
        if (!signal.aborted) host.notice(`Compaction used the standard summary instead: ${error instanceof Error ? error.message : String(error)}`, "warning");
        return undefined;
      }
      if (!result) return undefined;
      const compacted = [...preparation.messagesToSummarize, ...preparation.turnPrefixMessages];
      const added = compacted.map(userMessageText).filter((text): text is string => text !== undefined);
      const fileLists = result.details as { readFiles?: string[]; modifiedFiles?: string[] } | undefined;
      const details: WackCodeCompactionDetails = {
        wackcode: COMPACTION_DETAILS_VERSION,
        body: result.summary,
        ...keepUserMessages([...(previous?.userMessages ?? []), ...added], previous?.omittedUserMessages ?? 0),
        skills: mergeSkills(previous?.skills ?? [], skillRefs(compacted)),
        readFiles: fileLists?.readFiles ?? [],
        modifiedFiles: fileLists?.modifiedFiles ?? [],
      };
      return {
        compaction: {
          summary: buildSummary(details, todo()),
          firstKeptEntryId: result.firstKeptEntryId,
          tokensBefore: result.tokensBefore,
          usage: result.usage,
          details,
        },
      };
    });
  };
}

function latestDetails(entries: SessionEntry[]): WackCodeCompactionDetails | undefined {
  const details = getLatestCompactionEntry(entries)?.details;
  return isWackCodeDetails(details) ? details : undefined;
}
