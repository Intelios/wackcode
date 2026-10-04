/**
 * Built-in sub-agents — the `subagent` tool lets the chat's agent hand a self-contained task to
 * a child agent with its own context window, or run several at once. Modelled on Pi's
 * `examples/extensions/subagent` (MIT; single and parallel modes, output format, caps) and the
 * foreground model of `pi-subagents` (MIT): children run in-process as their own Pi sessions
 * (see `subagent-runner.ts`) rather than as spawned `pi` processes, which WackCode's worker
 * cannot start. Agents come from Settings, not from `.md` files on disk.
 *
 * Unlike the other built-ins this one can be switched off, and is off by default because every
 * child is extra model usage. The factory always loads; while switched off its tool is simply
 * kept out of the active set (`inactiveTools`), so turning it on or off never restarts a chat.
 */
import type { ExtensionAPI, InlineExtension } from "@earendil-works/pi-coding-agent";
import type { ExecutionPolicyConfig, SubagentRuntimeConfig, SubagentSpec, TaskMode } from "../../protocol.js";
import type { BuiltinHost, SubagentOutcome } from "../host.js";
import {
  MAX_CARD_OUTPUT,
  addUsage,
  cardUsage,
  createDetails,
  createThrottle,
  emptyUsage,
  modelContent,
  progressText,
  recordActivity,
  snapshotDetails,
  summarizeActivity,
  truncate,
} from "./details.js";
import { DEFAULT_EXECUTION_POLICY, isReadOnlyPlanning } from "../../execution-policy.js";
import { resolveSubagentSpec } from "./access.js";
import { readOnlyGuard } from "./guard.js";
import { SUBAGENT_PROMPT_SNIPPET, subagentDescription, subagentGuidelines } from "./prompt.js";
import { runScheduled } from "./scheduler.js";
import { normalizeSubagentParams, subagentParams } from "./schema.js";
import { WEB_FETCH_TOOL_NAME } from "../web-fetch/index.js";
import { SUBAGENT_TOOL_LABEL, SUBAGENT_TOOL_NAME, resolveChildTools } from "./types.js";

export interface SubagentsController {
  /** Apply the host's settings; null switches the tool off. */
  configure(config: SubagentRuntimeConfig | null): void;
  /** Tools that must stay out of the active set right now. */
  inactiveTools(): string[];
  /** Refresh model guidance after an access or mode change. */
  refresh(): void;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * `webFetch` is the parent's own Web Fetch extension. A child whose tools include `web_fetch`
 * loads it too, so both share one page cache and one address policy.
 */
export function createSubagentsExtension(host: BuiltinHost, currentMode: () => TaskMode, webFetch: InlineExtension, executionPolicy: () => ExecutionPolicyConfig = () => DEFAULT_EXECUTION_POLICY) {
  let config: SubagentRuntimeConfig | null = null;
  let pi: ExtensionAPI | undefined;
  /** The rendered definition last registered. Re-registering changes the system prompt, which
   *  costs every live chat its prompt cache, so it only happens when this actually changes. */
  let registeredKey: string | undefined;
  /** Calls where no child finished, flagged as errors once Pi finalizes the result. */
  const failedCalls = new Set<string>();

  const specsFor = (current: SubagentRuntimeConfig, agents: string[]): SubagentSpec[] =>
    agents.map((name) => {
      const spec = current.agents.find((agent) => agent.name === name);
      if (spec) return spec;
      const available = current.agents.map((agent) => agent.name).join(", ") || "none";
      throw new Error(`There is no sub-agent called '${name}'. Available: ${available}.`);
    });

  async function execute(
    toolCallId: string,
    params: unknown,
    signal: AbortSignal | undefined,
    onUpdate: ((result: { content: { type: "text"; text: string }[]; details: unknown }) => void) | undefined,
  ) {
    const current = config;
    if (!current || current.agents.length === 0) throw new Error("Sub-agents are switched off in Settings.");
    const parsed = normalizeSubagentParams(params);
    if (!parsed.ok) throw new Error(parsed.error);
    const specs = specsFor(current, parsed.tasks.map((task) => task.agent));
    const mode = currentMode();
    const policy = executionPolicy();
    // A read-only planning parent remains the ceiling even with the child override on.
    if (isReadOnlyPlanning(mode, policy)) {
      const editors = [...new Set(specs.filter((spec) => !spec.readOnly).map((spec) => spec.name))];
      if (editors.length > 0) {
        throw new Error(
          `Plan mode only runs read-only sub-agents, and ${editors.join(", ")} can edit files. Use a read-only sub-agent while planning.`,
        );
      }
    }

    // From here on nothing throws: a thrown error would replace the result, losing both the
    // card and the usage the children already spent.
    const items = parsed.tasks.map((input, index) => ({ input, spec: resolveSubagentSpec(specs[index], mode, policy) }));
    const details = createDetails(parsed.mode, items.map((item) => ({ input: item.input, readOnly: item.spec.readOnly })));
    const outputs: (string | undefined)[] = [];
    const updates = createThrottle(() =>
      onUpdate?.({ content: [{ type: "text", text: progressText(details) }], details: snapshotDetails(details) }),
    );
    const available = host.childToolNames();

    const usages = await runScheduled(items, current.maxConcurrency, async ({ input, spec }, index) => {
      const result = details.results[index];
      if (signal?.aborted) {
        result.status = "aborted";
        return emptyUsage();
      }
      if (spec.unavailable) {
        result.status = "failed";
        result.error = spec.unavailable;
        updates.schedule();
        return emptyUsage();
      }
      result.status = "running";
      result.startedAt = Date.now();
      updates.schedule();

      const tools = resolveChildTools(spec.tools, spec.readOnly, available);
      let outcome: SubagentOutcome;
      try {
        outcome = await host.runSubagent({
          toolCallId,
          index,
          spec,
          task: input.task,
          tools,
          extensions: [
            ...(spec.readOnly ? [readOnlyGuard()] : []),
            ...(tools.includes(WEB_FETCH_TOOL_NAME) ? [webFetch] : []),
          ],
          signal,
          observer: {
            started: (model) => {
              result.model = model;
              updates.schedule();
            },
            tool: (name, args) => {
              recordActivity(result, summarizeActivity(name, args));
              updates.schedule();
            },
            usage: (total, turns) => {
              result.usage = cardUsage(total, turns);
              updates.schedule();
            },
          },
        });
      } catch (error) {
        outcome = {
          status: signal?.aborted ? "aborted" : "failed",
          output: "",
          error: errorText(error),
          usage: emptyUsage(),
          turns: 0,
        };
      }

      const output = host.redact(outcome.output);
      outputs[index] = output;
      const card = truncate(output, MAX_CARD_OUTPUT);
      result.status = outcome.status;
      if (card.text) result.output = card.text;
      if (card.truncated) result.outputTruncated = true;
      if (outcome.error) result.error = host.redact(outcome.error);
      // Saved with the final result for the side panel; `snapshotDetails` keeps it out of the
      // live card updates that follow while siblings still run.
      if (outcome.transcript) result.transcript = outcome.transcript;
      result.usage = cardUsage(outcome.usage, outcome.turns);
      result.endedAt = Date.now();
      updates.schedule();
      return outcome.usage;
    });
    updates.cancel();

    if (!details.results.some((result) => result.status === "done")) failedCalls.add(toolCallId);
    return {
      content: [{ type: "text" as const, text: modelContent(details, outputs) }],
      details,
      // Recorded on the tool result, so the chat's token and cost totals include the children.
      usage: usages.reduce((total, usage) => addUsage(total, usage), emptyUsage()),
    };
  }

  const register = () => {
    if (!pi) return;
    const agents = (config?.agents ?? []).map((spec) => resolveSubagentSpec(spec, currentMode(), executionPolicy()));
    const definition = {
      name: SUBAGENT_TOOL_NAME,
      label: SUBAGENT_TOOL_LABEL,
      description: subagentDescription(agents),
      promptSnippet: SUBAGENT_PROMPT_SNIPPET,
      promptGuidelines: subagentGuidelines(config?.trigger ?? "on_request", isReadOnlyPlanning(currentMode(), executionPolicy())),
      parameters: subagentParams(agents.map((agent) => agent.name)),
    };
    const key = JSON.stringify(definition);
    if (key === registeredKey) return;
    registeredKey = key;
    pi.registerTool({
      ...definition,
      parameters: definition.parameters as never,
      // One call at a time, and never alongside the parent's own tools: a Worker child must
      // not edit while the parent does. Parallel work happens inside one call's `tasks`.
      executionMode: "sequential",
      execute: (toolCallId, params: unknown, signal, onUpdate) =>
        execute(toolCallId, params, signal, onUpdate as never) as never,
    });
  };

  const factory = (bound: ExtensionAPI) => {
    pi = bound;
    registeredKey = undefined;
    register();
    // A returned isError is ignored by Pi, and a thrown error would drop the details and usage,
    // so a call where every child failed is flagged here, after the result is final.
    bound.on("tool_result", (event) => {
      if (event.toolName !== SUBAGENT_TOOL_NAME || !failedCalls.delete(event.toolCallId)) return undefined;
      return { isError: true };
    });
  };

  const controller: SubagentsController = {
    refresh: register,
    configure(next) {
      config = next;
      register();
    },
    inactiveTools() {
      return config && config.agents.length > 0 ? [] : [SUBAGENT_TOOL_NAME];
    },
  };

  return { factory, controller };
}
