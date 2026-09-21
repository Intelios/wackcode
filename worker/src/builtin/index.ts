/**
 * WackCode's built-in extensions: always on, compiled into the worker, loaded through Pi's
 * inline-factory mechanism rather than the trusted-package path. They are ordinary code in
 * this repo — no trust gate applies because nothing external can reach this list.
 */
import type { InlineExtension } from "@earendil-works/pi-coding-agent";
import { createAskUserQuestionExtension } from "./ask-user-question.js";
import type { BuiltinHost } from "./host.js";
import { type PlanModeController, createPlanModeExtension } from "./plan-mode/index.js";

export interface BuiltinExtensions {
  /** Factories handed to `DefaultResourceLoader.extensionFactories`. */
  factories: InlineExtension[];
  /** Switches the plan-mode extension between Build and Plan. */
  planMode: PlanModeController;
}

export function createBuiltinExtensions(host: BuiltinHost): BuiltinExtensions {
  const planMode = createPlanModeExtension(host);
  return {
    factories: [
      { name: "wackcode-ask", factory: createAskUserQuestionExtension(host), hidden: true },
      { name: "wackcode-plan-mode", factory: planMode.factory, hidden: true },
    ],
    planMode: planMode.controller,
  };
}
