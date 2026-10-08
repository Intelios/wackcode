/**
 * WackCode's built-in extensions: compiled into the worker, loaded through Pi's inline-factory
 * mechanism rather than the trusted-package path. They are ordinary code in this repo — no
 * trust gate applies because nothing external can reach this list. All are always on except
 * sub-agents and computer use, which the user switches on in Settings (their tools stay
 * inactive until then), browser preview and web fetch, which are on until the user switches them
 * off (see `SWITCHABLE_BUILTIN_TOOLS`), and MCP, whose tools come from the servers the user adds.
 *
 * A Chat mode chat loads `chatFactories` instead of `factories`: no shell, planning, todo,
 * sub-agents, goal loop, computer use or skill creator, plus the guard that enforces the rest
 * (`chat-mode/`). The controllers of the built-ins it leaves out still exist and report their
 * resting state, so the worker reads them without asking which kind of chat it is.
 */
import type { InlineExtension } from "@earendil-works/pi-coding-agent";
import { BROWSER_TOOL_NAMES, createBrowserExtension } from "./browser.js";
import { createAskUserQuestionExtension } from "./ask-user-question.js";
import { type BashJobsController, createBashJobsExtension } from "./bash-jobs.js";
import { createChatGuardExtension } from "./chat-mode/index.js";
import { type ComputerUseController, createComputerUseExtension } from "./computer-use/index.js";
import { createAutoTitleExtension, type AutoTitleController } from "./auto-title.js";
import { type GoalController, createGoalExtension } from "./goal/index.js";
import { type MemoryController, createMemoryExtension } from "./memory/index.js";
import type { BuiltinHost } from "./host.js";
import { type McpController, createMcpExtension } from "./mcp/index.js";
import type { ExecutionPolicyConfig } from "../protocol.js";
import { DEFAULT_EXECUTION_POLICY, normalizeExecutionPolicy } from "../execution-policy.js";
import { type PlanModeController, createPlanModeExtension } from "./plan-mode/index.js";
import { type SkillCreatorController, createSkillCreatorExtension } from "./skill-creator/index.js";
import { type SubagentsController, createSubagentsExtension } from "./subagents/index.js";
import { type TodoHandle, createTodoExtension } from "./todo/index.js";
import { WEB_FETCH_TOOL_NAME, createWebFetchExtension } from "./web-fetch/index.js";

/**
 * Built-in tools the user's Settings denylist applies to. Every other built-in tool is part of
 * how the app works (a disabled plan_mode_complete would break Plan mode) and ignores it.
 */
export const SWITCHABLE_BUILTIN_TOOLS: ReadonlySet<string> = new Set([WEB_FETCH_TOOL_NAME, ...BROWSER_TOOL_NAMES]);

export interface BuiltinExtensions {
  /** Factories handed to `DefaultResourceLoader.extensionFactories`. */
  factories: InlineExtension[];
  /** The same, for a Chat mode chat: the reduced set plus the chat guard. */
  chatFactories: InlineExtension[];
  /** Bounded shell waits and per-run command cleanup. */
  bashJobs: BashJobsController;
  /** Switches the plan-mode extension between Build, Plan and Ultra Plan. */
  planMode: PlanModeController;
  configureExecutionPolicy(policy?: ExecutionPolicyConfig): void;
  getExecutionPolicy(): ExecutionPolicyConfig;
  /** Read access to the todo list so snapshots can seed the panel. */
  todo: TodoHandle;
  /** Applies the user's sub-agent settings and says when its tool must stay off. */
  subagents: SubagentsController;
  autoTitle: AutoTitleController;
  /** The goal loop behind `/goal`: start/control plus the continuation flag the worker reads. */
  goal: GoalController;
  /** Project memory: the note tools and the index section the system prompt serves. */
  memory: MemoryController;
  /** The user's MCP servers: connects them and says which of their tools are usable. */
  mcp: McpController;
  /** Computer use: off until the user switches it on, and inactive while off. */
  computerUse: ComputerUseController;
  /** The /skill-creator workflow: branch state plus the tool's availability. */
  skillCreator: SkillCreatorController;
}

export function createBuiltinExtensions(host: BuiltinHost, options: {
  commandEnabled?: (appKey: string) => boolean;
  /** Read when the factories run, which is after `init` has said what kind of chat this is. */
  isChat?: () => boolean;
} = {}): BuiltinExtensions {
  let policy = DEFAULT_EXECUTION_POLICY;
  let subagents: ReturnType<typeof createSubagentsExtension> | undefined;
  const bashJobs = createBashJobsExtension();
  const mcp = createMcpExtension(host);
  const planMode = createPlanModeExtension({ ...host, publishPlanState(state) {
    subagents?.controller.refresh();
    host.publishPlanState(state);
  } }, {
    owns: (name) => mcp.controller.serverOf(name) !== undefined,
    isReadOnly: (name) => mcp.controller.isReadOnlyTool(name),
  }, () => policy);
  const todo = createTodoExtension(host);
  // One instance for the chat and its sub-agents, so they share its page cache.
  const webFetch: InlineExtension = { name: "wackcode-web-fetch", factory: createWebFetchExtension(), hidden: true };
  subagents = createSubagentsExtension(host, () => planMode.controller.getState().mode, webFetch, () => policy);
  const autoTitle = createAutoTitleExtension(host);
  const goal = createGoalExtension(host, () => planMode.controller.getState().mode !== "build");
  const memory = createMemoryExtension(() => options.isChat?.() ? "chat" : "project");
  const browser: InlineExtension = { name: "wackcode-browser", factory: createBrowserExtension(host), hidden: true };
  const computerUse = createComputerUseExtension(host);
  const skillCreator = createSkillCreatorExtension(host, {
    commandEnabled: () => options.commandEnabled?.("app:skill-creator") ?? true,
    isBuildMode: () => planMode.controller.getState().mode === "build",
  });
  const ask: InlineExtension = {
    name: "wackcode-ask",
    // Ultra Plan's questionnaires offer "Write the plan now".
    factory: createAskUserQuestionExtension(host, () => planMode.controller.getState().mode === "ultraplan"),
    hidden: true,
  };
  const autoTitleExtension: InlineExtension = { name: "wackcode-auto-title", factory: autoTitle.factory, hidden: true };
  const memoryExtension: InlineExtension = { name: "wackcode-memory", factory: memory.factory, hidden: true };
  const mcpExtension: InlineExtension = { name: "wackcode-mcp", factory: mcp.factory, hidden: true };
  return {
    factories: [
      // First: clean up foreground jobs before the goal loop can launch another round.
      { name: "wackcode-bash-jobs", factory: bashJobs.factory, hidden: true },
      ask,
      { name: "wackcode-plan-mode", factory: planMode.factory, hidden: true },
      { name: "wackcode-todo", factory: todo.factory, hidden: true },
      { name: "wackcode-subagents", factory: subagents.factory, hidden: true },
      autoTitleExtension,
      { name: "wackcode-goal", factory: goal.factory, hidden: true },
      memoryExtension,
      webFetch,
      browser,
      { name: "wackcode-computer-use", factory: computerUse.factory, hidden: true },
      { name: "wackcode-skill-creator", factory: skillCreator.factory, hidden: true },
      mcpExtension,
    ],
    chatFactories: [
      // First, so its tool_call block runs ahead of every other handler.
      createChatGuardExtension({
        workspace: () => host.workspace(),
        ownsMcpTool: (name) => mcp.controller.serverOf(name) !== undefined,
      }),
      ask,
      autoTitleExtension,
      memoryExtension,
      webFetch,
      browser,
      mcpExtension,
    ],
    bashJobs: bashJobs.controller,
    planMode: planMode.controller,
    getExecutionPolicy: () => policy,
    configureExecutionPolicy(next) {
      policy = normalizeExecutionPolicy(next);
      subagents?.controller.refresh();
    },
    todo: todo.handle,
    subagents: subagents.controller,
    autoTitle: autoTitle.controller,
    goal: goal.controller,
    memory: memory.controller,
    mcp: mcp.controller,
    computerUse: computerUse.controller,
    skillCreator: skillCreator.controller,
  };
}
