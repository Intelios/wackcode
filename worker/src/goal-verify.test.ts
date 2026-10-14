import { describe, expect, it, vi } from "vitest";
import { parseGoalVerdict, runGoalVerification } from "./builtin/goal/verify.js";

/**
 * The verifier's JSON contract, pinned so the loop's asymmetry stays deliberate:
 * unusable output fails OPEN (completes the goal rather than wedging it), while a clean
 * `passed:false` with no `nextAction` stops the loop instead of spinning.
 */
describe("goal verifier contract", () => {
  it("carries the conversation ID to the auxiliary model request", async () => {
    const completeSimple = vi.fn().mockResolvedValue({ stopReason: "stop", content: [{ type: "text", text: '{"passed":true}' }] });
    const verdict = await runGoalVerification({ completeSimple } as never,
      { reasoning: false } as never, "Verify", { objective: "Test", iteration: 1, transcript: "Done" }, undefined, "conversation-id");
    expect(verdict.kind).toBe("pass");
    expect(completeSimple.mock.calls[0][2].sessionId).toBe("conversation-id");
  });

  it("accepts a clean pass", () => {
    expect(parseGoalVerdict('{"passed": true, "reason": "all checks ran"}')).toEqual({ kind: "pass", reason: "all checks ran" });
    expect(parseGoalVerdict('{"passed":true}')).toEqual({ kind: "pass", reason: undefined });
  });

  it("continues on passed:false with a nextAction", () => {
    expect(parseGoalVerdict('{"passed": false, "reason": "no test", "nextAction": "run pnpm test"}'))
      .toEqual({ kind: "continue", reason: "no test", nextAction: "run pnpm test" });
  });

  it("stops the loop on passed:false without a nextAction", () => {
    expect(parseGoalVerdict('{"passed": false, "reason": "nothing left to try"}'))
      .toEqual({ kind: "stop", reason: "nothing left to try" });
    expect(parseGoalVerdict('{"passed": false, "nextAction": "  "}').kind).toBe("stop");
  });

  it("fails open on malformed or missing JSON", () => {
    expect(parseGoalVerdict("the goal looks done").kind).toBe("inconclusive");
    expect(parseGoalVerdict("").kind).toBe("inconclusive");
    expect(parseGoalVerdict("{not json}").kind).toBe("inconclusive");
    expect(parseGoalVerdict("[1,2]").kind).toBe("inconclusive");
    expect(parseGoalVerdict('{"passed": "yes"}').kind).toBe("inconclusive");
    expect(parseGoalVerdict('{"status": "done"}').kind).toBe("inconclusive");
  });

  it("unwraps fenced JSON and prose around the object", () => {
    expect(parseGoalVerdict('```json\n{"passed": true, "reason": "ok"}\n```')).toEqual({ kind: "pass", reason: "ok" });
    expect(parseGoalVerdict('Verdict: {"passed": false, "nextAction": "keep going"} end'))
      .toEqual({ kind: "continue", reason: "The goal is not met yet.", nextAction: "keep going" });
  });
});
