import { describe, expect, it } from "vitest";
import { DEFAULT_CHAT_PROMPT, DEFAULT_PLAN_PROMPT, DEFAULT_SYSTEM_PROMPT, DEFAULT_ULTRA_PLAN_PROMPT } from "./promptDefaults";
// The source of truth for the plan texts lives in the worker package; importing it here fails
// the moment either copy drifts, which is the whole point of this test.
import { buildPlanModePrompt } from "../worker/src/builtin/plan-mode/prompt.js";
import { DEFAULT_CHAT_PROMPT as WORKER_CHAT_PROMPT } from "../worker/src/builtin/chat-mode/prompt.js";

describe("prompt default display copies", () => {
  it("match the worker's Plan and Ultra Plan builders byte for byte", () => {
    expect(DEFAULT_PLAN_PROMPT).toBe(buildPlanModePrompt("plan"));
    expect(DEFAULT_ULTRA_PLAN_PROMPT).toBe(buildPlanModePrompt("ultraplan"));
  });

  it("matches the worker's Chat mode persona byte for byte", () => {
    expect(DEFAULT_CHAT_PROMPT).toBe(WORKER_CHAT_PROMPT);
  });

  it("keeps the persona copy honest in shape while the worker suite pins its exact text", () => {
    // The byte-exact pin against Pi's live preamble lives in worker/src/builtin.test.ts, which
    // can import the pi package. Here we only guard against an accidental placeholder.
    expect(DEFAULT_SYSTEM_PROMPT).toMatch(/^You are an expert coding assistant/);
    expect(DEFAULT_SYSTEM_PROMPT).toContain("coding agent harness");
  });
});
