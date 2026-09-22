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

  it("reassembles a multi-megabyte line split across many chunks, including a CRLF straddling two", () => {
    const decoder = new JsonLineDecoder();
    const payload = JSON.stringify({ type: "prompt", images: [{ type: "image", data: "A".repeat(6 * 1024 * 1024), mimeType: "image/png" }] });
    const bytes = Buffer.from(`${payload}\r\n{"next":"é"}\n`);
    // 64 KiB chunks, plus one boundary forced between the \r and the \n.
    const cr = bytes.indexOf(0x0d);
    const cuts = new Set([cr + 1, bytes.length]);
    for (let offset = 65_536; offset < bytes.length; offset += 65_536) cuts.add(offset);
    const lines: string[] = [];
    let start = 0;
    for (const end of [...cuts].sort((a, b) => a - b)) {
      lines.push(...decoder.push(bytes.subarray(start, end)));
      start = end;
    }
    expect(lines).toHaveLength(2);
    expect(lines[0]).toBe(payload);
    expect(lines[1]).toBe('{"next":"é"}');
  });

  it("joins a UTF-8 character split between chunks", () => {
    const decoder = new JsonLineDecoder();
    const bytes = Buffer.from('{"t":"é"}\n');
    const split = bytes.indexOf(0xc3) + 1;
    expect(decoder.push(bytes.subarray(0, split))).toEqual([]);
    expect(decoder.push(bytes.subarray(split))).toEqual(['{"t":"é"}']);
  });
});
