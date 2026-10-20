import { describe, expect, it } from "vitest";
import { DEFAULT_CHAT_PROMPT, DEFAULT_PLAN_PROMPT, DEFAULT_SYSTEM_PROMPT, DEFAULT_ULTRA_PLAN_PROMPT } from "./promptDefaults";
// The source of truth for the plan texts lives in the worker package; importing it here fails
// the moment either copy drifts, which is the whole point of this test.
import { buildPlanModePrompt } from "../worker/src/builtin/plan-mode/prompt.js";
import { DEFAULT_CHAT_PROMPT as WORKER_CHAT_PROMPT } from "../worker/src/builtin/chat-mode/prompt.js";
import { DEFAULT_CODE_PROMPT } from "../worker/src/builtin/code-prompt/prompt.js";

describe("prompt default display copies", () => {
  it("match the worker's Plan and Ultra Plan builders byte for byte", () => {
    expect(DEFAULT_PLAN_PROMPT).toBe(buildPlanModePrompt("plan"));
    expect(DEFAULT_ULTRA_PLAN_PROMPT).toBe(buildPlanModePrompt("ultraplan"));
  });

  it("matches the worker's Chat mode persona byte for byte", () => {
    expect(DEFAULT_CHAT_PROMPT).toBe(WORKER_CHAT_PROMPT);
  });

  it("matches the worker's coding core byte for byte", () => {
    expect(DEFAULT_SYSTEM_PROMPT).toBe(DEFAULT_CODE_PROMPT);
    expect(DEFAULT_SYSTEM_PROMPT).toMatch(/^You are a coding agent working inside WackCode/);
    expect(DEFAULT_SYSTEM_PROMPT).not.toMatch(/\bpi\b/i);
  });
});
