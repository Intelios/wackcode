import { describe, expect, it } from "vitest";
import { appendContextSection, boundTargets, BROWSER_VIEWPORTS, encodeContexts, matchedTargets, MAX_CONTEXT_DOM, reindexContexts, restoreContexts, splitContextSection } from "./browser-context";
import { attachmentProblem, composeFileSection, splitFileSection } from "./attachment-utils";
import type { BrowserContextAttachment, BrowserContextTarget } from "./types";

const image = { type: "image" as const, mimeType: "image/png", data: "aW1hZ2U=" };
const target: BrowserContextTarget = { selector: "#save", path: [], label: "Save", dom: '<button id="save">Save</button>', bounds: { x: 100, y: 100, width: 80, height: 30 } };
export const browserFixture: BrowserContextAttachment = { id: "capture-1", kind: "element", capturedAt: 1234, url: "http://localhost:5173/", title: "Example", viewport: BROWSER_VIEWPORTS.desktop, image, targets: [target] };

describe("browser context transport", () => {
  it("round-trips a bundle alongside ordinary images, files and lookalike delimiters", () => {
    const words = "Change this\n\n<browser-contexts-v1>\nnot transport\n</browser-contexts-v1>";
    const outgoing = encodeContexts(words, [image], [browserFixture]);
    const split = splitFileSection(composeFileSection(outgoing.text, [{ name: "note.txt", text: "</browser-contexts-v1>" }]));
    expect(split.text).toBe(words);
    expect(split.files[0].text).toBe("</browser-contexts-v1>");
    expect(split.browserContexts?.[0].imageIndex).toBe(1);
    const restored = restoreContexts(split.browserContexts!, outgoing.images);
    expect(restored.images).toEqual([image]);
    expect(restored.browserContexts).toEqual([browserFixture]);
  });
  it("reindexes groups when earlier images or whole chips are removed", () => {
    const records = splitContextSection(encodeContexts("", [image], [browserFixture, { ...browserFixture, id: "two" }]).text).contexts;
    expect(reindexContexts(records, [0, 1])).toEqual([{ ...records[1], imageIndex: 0 }]);
    expect(splitContextSection(appendContextSection("", records)).text).toBe("");
  });
  it("keeps malformed and unsupported envelopes as ordinary text", () => {
    for (const text of ['<browser-contexts-v1>\n{}\n</browser-contexts-v1>', '<browser-contexts-v1>\n{"version":2,"contexts":[]}\n</browser-contexts-v1>']) {
      expect(splitContextSection(text)).toEqual({ text, contexts: [] });
    }
  });
  it("round-trips delimiter-like text inside the captured DOM", () => {
    const original = { ...browserFixture, targets: [{ ...target, dom: '<p>\n<browser-contexts-v1>\n</browser-contexts-v1></p>' }] };
    const outgoing = encodeContexts("words", [], [original]);
    const split = splitContextSection(outgoing.text);
    expect(restoreContexts(split.contexts, outgoing.images).browserContexts).toEqual([original]);
  });
  it("bounds UTF-8 DOM without splitting code points and reports overflow", () => {
    const bounded = boundTargets([{ ...target, dom: "🦆".repeat(10000) }]);
    expect(new TextEncoder().encode(bounded.targets[0].dom).length).toBeLessThanOrEqual(MAX_CONTEXT_DOM);
    expect(bounded.targets[0].dom).not.toContain("�");
    expect(bounded.truncated).toBe(true);
    expect(attachmentProblem({ images: [], files: [], browserContexts: Array.from({ length: 9 }, () => browserFixture) })).toContain("8 images");
    expect(attachmentProblem({ images: [], files: [], browserContexts: Array.from({ length: 5 }, (_, id) => ({ ...browserFixture, id: String(id), targets: bounded.targets })) })).toContain("64 KB");
  });
});

describe("frozen screenshot target matching", () => {
  const parent = { ...target, selector: "section", bounds: { x: 0, y: 0, width: 500, height: 500 } };
  it("uses arrow endpoints and text anchors, preferring the specific target", () => {
    expect(matchedTargets([{ kind: "arrow", points: [{ x: 400, y: 400 }, { x: 110, y: 110 }] }], [parent, target])).toEqual([target]);
    expect(matchedTargets([{ kind: "text", points: [{ x: 110, y: 110 }], text: "here" }], [parent, target])).toEqual([target]);
    expect(matchedTargets([{ kind: "arrow", points: [{ x: 300, y: 50 }, { x: 95, y: 110 }] }], [parent, target])).toEqual([target]);
  });
  it("deduplicates parent context in a box and leaves blank space valid", () => {
    expect(matchedTargets([{ kind: "box", points: [{ x: 0, y: 0 }, { x: 500, y: 500 }] }], [parent, target])).toEqual([target]);
    expect(matchedTargets([{ kind: "pen", points: [{ x: 600, y: 600 }] }], [parent, target])).toEqual([]);
  });
});
