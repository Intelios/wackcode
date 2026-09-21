import { describe, expect, it } from "vitest";
import { JsonLineDecoder } from "./framing.js";

describe("JsonLineDecoder", () => {
  it("uses LF framing without splitting Unicode separators", () => {
    const decoder = new JsonLineDecoder();
    expect(decoder.push(Buffer.from('{"text":"one\u2028two"}'))).toEqual([]);
    expect(decoder.push(Buffer.from("\n{\"ok\":true}\r\n"))).toEqual([
      '{"text":"one\u2028two"}',
      '{"ok":true}'
    ]);
  });
});
