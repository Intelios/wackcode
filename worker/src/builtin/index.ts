/**
 * WackCode's built-in extensions: compiled into the worker, loaded through Pi's inline-factory
 * mechanism rather than the trusted-package path. They are ordinary code in this repo — no
 * trust gate applies because nothing external can reach this list. All are always on except
 * sub-agents, which the user switches on in Settings (its tool stays inactive until then).
 */
import type { InlineExtension } from "@earendil-works/pi-coding-agent";
import { createAskUserQuestionExtension } from "./ask-user-question.js";
import { createAutoTitleExtension, type AutoTitleController } from "./auto-title.js";
import type { BuiltinHost } from "./host.js";
import { type PlanModeController, createPlanModeExtension } from "./plan-mode/index.js";
import { type SubagentsController, createSubagentsExtension } from "./subagents/index.js";
import { type TodoHandle, createTodoExtension } from "./todo/index.js";

export interface BuiltinExtensions {
  /** Factories handed to `DefaultResourceLoader.extensionFactories`. */
  factories: InlineExtension[];
  /** Switches the plan-mode extension between Build, Plan and Ultra Plan. */
  planMode: PlanModeController;
  /** Read access to the todo list so snapshots can seed the panel. */
  todo: TodoHandle;
  /** Applies the user's sub-agent settings and says when its tool must stay off. */
  subagents: SubagentsController;
  autoTitle: AutoTitleController;
}

export function createBuiltinExtensions(host: BuiltinHost): BuiltinExtensions {
  const planMode = createPlanModeExtension(host);
  const todo = createTodoExtension(host);
  const subagents = createSubagentsExtension(host, () => planMode.controller.getState().mode);
  const autoTitle = createAutoTitleExtension(host);
  return {
    factories: [
      {
        name: "wackcode-ask",
        // Ultra Plan's questionnaires offer "Write the plan now".
        factory: createAskUserQuestionExtension(host, () => planMode.controller.getState().mode === "ultraplan"),
        hidden: true,
      },
      { name: "wackcode-plan-mode", factory: planMode.factory, hidden: true },
      { name: "wackcode-todo", factory: todo.factory, hidden: true },
      { name: "wackcode-subagents", factory: subagents.factory, hidden: true },
      { name: "wackcode-auto-title", factory: autoTitle.factory, hidden: true },
    ],
    planMode: planMode.controller,
    todo: todo.handle,
    subagents: subagents.controller,
    autoTitle: autoTitle.controller,
  };
}
