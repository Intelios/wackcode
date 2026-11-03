import { act, renderHook } from "@testing-library/react";
import { expect, it } from "vitest";
import { useStableCallbacks } from "./useStableCallbacks";

it("keeps event identities stable, uses the latest committed selection and retains an async call's origin", async () => {
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => { finish = resolve; });
  const { result, rerender } = renderHook(({ chat }) => useStableCallbacks({
    select: () => chat,
    send: async (text: string) => { await pending; return `${chat}:${text}`; }
  }), { initialProps: { chat: "a" } });
  const original = result.current;
  const sending = original.send("message");
  rerender({ chat: "b" });
  expect(result.current).toBe(original);
  expect(original.select()).toBe("b");
  await act(async () => { finish(); });
  expect(await sending).toBe("a:message");
  expect(await original.send("next")).toBe("b:next");
});
