import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { inlineVisualImages, MAX_VISUAL_IMAGE_BYTES, MAX_VISUAL_PAGE_BYTES, visualImageMime } from "./builtin/visual-images.js";

const PNG = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10, 0]);
let base: string;
beforeEach(async () => { base = await mkdtemp(join(tmpdir(), "wackcode-visual-images-")); });
afterEach(async () => { await rm(base, { recursive: true, force: true }); });

describe("visual image bytes", () => {
  it.each([
    [PNG, "image/png"], [Buffer.from("<svg/>"), "image/svg+xml"], [Buffer.from([255, 216, 255, 0]), "image/jpeg"],
    [Buffer.from("GIF89a"), "image/gif"], [Buffer.from("RIFFxxxxWEBP"), "image/webp"],
    [Buffer.from([0, 0, 0, 20, 102, 116, 121, 112, 97, 118, 105, 102, 0, 0, 0, 0, 97, 118, 105, 102]), "image/avif"],
    [Buffer.concat([Buffer.from("BM"), Buffer.alloc(12)]), "image/bmp"],
    [Buffer.from([0, 0, 1, 0, 1, 0]), "image/x-icon"],
    [Buffer.from('\uFEFF<?xml version="1.0"?>\n<!--icon--><svg xmlns="http://www.w3.org/2000/svg"></svg>'), "image/svg+xml"],
  ])("recognizes supported image signatures", (bytes, mime) => expect(visualImageMime(bytes)).toBe(mime));

  it("refuses text, JSON and a non-root SVG", () => {
    for (const text of ['{"key":"secret"}', "not an image", "<html><svg/></html>", "prefix<svg></svg>"]) expect(visualImageMime(Buffer.from(text))).toBeUndefined();
  });
});

describe("visual local images", () => {
  it("inlines quoted attributes and CSS URLs by bytes, retaining other URLs", async () => {
    const png = join(base, "an image.svg");
    await writeFile(png, PNG);
    const html = `<img src="${png}"><style>.a{background:url('${png}')}</style><div style="background:url(${png})"></div><img src="https://a.test/a.png">`;
    const result = await inlineVisualImages(html);
    expect(result.inlined).toBe(true);
    expect(result.html.match(/data:image\/png;base64,/g)).toHaveLength(3);
    expect(result.html).not.toContain(png);
    expect(result.html).toContain('src="https://a.test/a.png"');
  });

  it("leaves self-contained markup unchanged", async () => {
    expect(await inlineVisualImages('<svg></svg><img src="data:image/png;base64,abc">')).toEqual({ html: '<svg></svg><img src="data:image/png;base64,abc">', inlined: false });
  });

  it("keeps apostrophes and parentheses inside the active quote delimiter", async () => {
    const apostrophe = join(base, "Jack's image.png");
    const parentheses = join(base, "Screenshot (1).png");
    const doubleQuote = join(base, 'A "quoted" image.png');
    for (const path of [apostrophe, parentheses, doubleQuote]) await writeFile(path, PNG);
    const html = `<img src="${apostrophe}"><img src='${doubleQuote}'><style>.a{background:url('${parentheses}')}</style>`;
    const result = await inlineVisualImages(html);
    expect(result.html.match(/data:image\/png;base64,/g)).toHaveLength(3);
    expect(result.html).not.toContain(base);
  });

  it("names missing paths and refuses renamed text and symlinks to it", async () => {
    const renamed = join(base, "secrets.png");
    const link = join(base, "link.gif");
    const missing = join(base, "missing.jpg");
    await writeFile(renamed, '{"key":"hidden"}');
    await symlink(renamed, link);
    await expect(inlineVisualImages(`<img src="${renamed}"><img src="${link}"><img src="${missing}">`)).rejects.toThrow(/secrets\.png[\s\S]*link\.gif[\s\S]*missing\.jpg/);
  });

  it("refuses a named pipe without waiting for a writer", async () => {
    const pipe = join(base, "pipe.png");
    execFileSync("mkfifo", [pipe]);
    await expect(inlineVisualImages(`<img src="${pipe}">`)).rejects.toThrow("not a regular image file");
  });

  it("enforces 4 MiB per image and 8 MiB for the finished page", async () => {
    const oversized = join(base, "large.png");
    await writeFile(oversized, Buffer.concat([PNG, Buffer.alloc(MAX_VISUAL_IMAGE_BYTES)]));
    await expect(inlineVisualImages(`<img src="${oversized}">`)).rejects.toThrow("4 MiB");
    const medium = join(base, "medium.png");
    await writeFile(medium, Buffer.concat([PNG, Buffer.alloc(3 * 1024 * 1024)]));
    await expect(inlineVisualImages(`<img src="${medium}"><img src="${medium}">`)).rejects.toThrow("8 MiB");
    await expect(inlineVisualImages("x".repeat(MAX_VISUAL_PAGE_BYTES + 1))).rejects.toThrow("8 MiB");
  });

  it("allows only scratchpad images in Chat mode, including symlink targets", async () => {
    const scratch = join(base, "scratch");
    await mkdir(scratch);
    const inside = join(scratch, "inside.png");
    const outside = join(base, "outside.png");
    const escape = join(scratch, "escape.png");
    await writeFile(inside, PNG);
    await writeFile(outside, PNG);
    await symlink(outside, escape);
    expect((await inlineVisualImages(`<img src="${inside}">`, { isChat: true, cwd: scratch })).inlined).toBe(true);
    for (const path of [outside, escape]) await expect(inlineVisualImages(`<img src="${path}">`, { isChat: true, cwd: scratch })).rejects.toThrow("outside");
  });
});
