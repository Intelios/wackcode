/**
 * Built-in todo extension — a live task list the agent keeps current while it works,
 * rendered above the composer by the desktop. Ported from `@juicesharp/rpiv-todo`
 * v2.11.0 (MIT) `todo.ts` and `index.ts`, adapted to WackCode: the TUI overlay and the
 * `/todos` slash command are replaced by the desktop's TodoPanel (state flows over
 * `todo_state`), i18n is dropped, `owner`/`metadata` are gone from the schema, and the
 * sid-keyed multi-session store collapses to one closure (one session per worker, so the
 * stale-ctx races upstream guards against cannot happen). Like upstream there are no disk
 * writes: every tool result embeds the full list, and session lifecycle hooks rebuild
 * state from the branch, so the list survives worker restarts and compaction.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { TodoState } from "../../protocol.js";
import type { BuiltinHost } from "../host.js";
import { buildToolResult } from "./envelope.js";
import { TODO_PROMPT_GUIDELINES, TODO_PROMPT_SNIPPET } from "./prompt.js";
import { applyTaskMutation } from "./reducer.js";
import { EMPTY_STATE, type TaskState, replayFromBranch } from "./state.js";
import {
  TODO_PARAMS,
  TODO_TOOL_LABEL,
  TODO_TOOL_NAME,
  normalizeTodoParams,
} from "./types.js";

export interface TodoHandle {
  /** The state the desktop panel renders. */
  getState(): TodoState;
}

interface SessionContext {
  sessionManager: { getBranch(): unknown[] };
}

export function createTodoExtension(host: BuiltinHost) {
  let state: TaskState = { tasks: [...EMPTY_STATE.tasks], nextId: EMPTY_STATE.nextId };

  const getState = (): TodoState => ({ tasks: state.tasks });

  const emit = () => host.publishTodoState(getState());

  const restore = (ctx: SessionContext) => {
    state = replayFromBranch(ctx.sessionManager.getBranch());
    emit();
  };

  const factory = (bound: ExtensionAPI) => {
    bound.registerTool({
      name: TODO_TOOL_NAME,
      label: TODO_TOOL_LABEL,
      description:
        "Manage a task list for tracking multi-step progress. Actions: create (new task), update (change status/fields/dependencies), list (all tasks, optionally filtered by status), get (single task details), delete (tombstone), clear (reset all). Status: pending → in_progress → completed, plus deleted tombstone. Use this to plan and track multi-step work like research, design, and implementation.",
      promptSnippet: TODO_PROMPT_SNIPPET,
      promptGuidelines: TODO_PROMPT_GUIDELINES,
      parameters: TODO_PARAMS,
      async execute(_toolCallId, params: unknown) {
        const parsed = normalizeTodoParams(params);
        if (!parsed.ok) throw new Error(parsed.error);
        const result = applyTaskMutation(state, parsed.action, parsed.params);
        state = result.state;
        emit();
        return buildToolResult(parsed.action, state, result.op);
      },
    });

    // The branch is the store: a replay rebuilds the list after a restart or compaction.
    bound.on("session_start", (_event, ctx: SessionContext) => restore(ctx));
    bound.on("session_compact", (_event, ctx: SessionContext) => restore(ctx));
    bound.on("session_tree", (_event, ctx: SessionContext) => restore(ctx));
  };

  return { factory, handle: { getState } };
}
