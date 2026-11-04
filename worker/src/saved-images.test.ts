import { mkdtemp, mkdir, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { ExtensionAPI, ExtensionToolContext, ResizedImage } from "@earendil-works/pi-coding-agent";
import { ImageStore, IMAGE_METADATA_KEY, imageHash, resolveSavedImage, savedImages, type StoredImage } from "./saved-images.js";
import { createChatImagesExtension } from "./builtin/chat-images.js";
import type { ImageContent } from "./protocol.js";
import type { EntryLike } from "./tree.js";

const original: ImageContent = { type: "image", mimeType: "image/png", data: Buffer.from("original-pixels").toString("base64") };
const view: ImageContent = { ...original, data: Buffer.from("small-view").toString("base64") };
const resizeImage = async (): Promise<ResizedImage> => ({ ...view, originalWidth: 2400, originalHeight: 1200, width: 1600, height: 800, wasResized: true });
const cleanup: string[] = [];
afterEach(async () => { await Promise.all(cleanup.splice(0).map((path) => rm(path, { recursive: true, force: true }))); });
async function setup(isChat = false) {
  const cwd = await mkdtemp(join(tmpdir(), "wackcode-images-"));
  cleanup.push(cwd);
  const directory = isChat ? join(cwd, ".wackcode", "images") : join(cwd, "session", "images");
  const store = new ImageStore({ directory, cwd, isChat, resizeImage, convertToPng: async () => null });
  return { cwd, directory, store };
}
function message(id: string, image = view, record?: StoredImage, role = "user", toolName?: string): EntryLike {
  return { type: "message", id, parentId: null, message: { role, timestamp: 1, ...(toolName ? { toolName, toolCallId: `call-${id}` } : {}),
    content: [image], ...(record ? { [IMAGE_METADATA_KEY]: { v: 1, images: [record] } } : {}) } };
}

describe("saved image files", () => {
  it("shares concurrent views, isolates returned objects, and evicts old viewing copies", async () => {
    const h = await setup();
    let calls = 0;
    const store = new ImageStore({ ...h.store.options, resizeImage: async () => { calls++; return resizeImage(); } });
    const [a, b] = await Promise.all([store.view(original), store.view(original)]);
    a.data = "mutated";
    expect(b.data).toBe(view.data);
    expect((await store.view(original)).data).toBe(view.data);
    expect(calls).toBe(1);
    for (let i = 0; i < 16; i++) await store.view({ ...original, data: `other-${i}` });
    await store.view(original);
    expect(calls).toBe(18);
  });

  it("bounds cached bytes and retries failed decodes", async () => {
    const h = await setup();
    let calls = 0;
    const store = new ImageStore({ ...h.store.options, resizeImage: async () => {
      calls++;
      if (calls === 1) throw new Error("Decode failed");
      return { ...await resizeImage(), data: "x".repeat(9 * 1024 * 1024) };
    } });
    await expect(store.view(original)).rejects.toThrow("Decode failed");
    await store.view(original);
    await store.view({ ...original, data: "another" });
    await store.view(original);
    expect(calls).toBe(4);
  });

  it("verifies changed original files even when their viewing copy is cached", async () => {
    const { store, directory } = await setup(true);
    const prepared = await store.prepare(original);
    const saved = resolveSavedImage("wackcode-image://entry/cached/0", [message("cached", view, prepared.record)])!;
    await store.load(saved);
    await writeFile(join(directory, prepared.record.blob), "changed");
    await expect(store.original(saved)).rejects.toThrow("changed");
    expect((await store.load(saved)).quality).toBe("saved-copy");
  });

  it("does not reuse a decode across MIME types or stores", async () => {
    const h = await setup();
    let calls = 0;
    const options = { ...h.store.options, resizeImage: async () => { calls++; return resizeImage(); } };
    const store = new ImageStore(options);
    await store.view(original);
    await store.view({ ...original, mimeType: "image/jpeg" });
    await new ImageStore(options).view(original);
    expect(calls).toBe(3);
  });

  it("rechecks Chat confinement after warming the viewing cache", async () => {
    const h = await setup(true);
    const outside = await setup();
    await h.store.prepare(original);
    await rm(h.directory, { recursive: true });
    await symlink(outside.cwd, h.directory);
    await expect(h.store.prepare(original)).rejects.toThrow("outside");
    expect(await readdir(outside.cwd)).toEqual([]);
  });

  it("keeps byte-identical originals outside the viewing copy, deduplicates them, and writes private files", async () => {
    const { store, directory } = await setup();
    const prepared = await store.prepare(original);
    expect(prepared.image).toEqual(view);
    expect(prepared.record).toMatchObject({ width: 2400, height: 1200, viewWidth: 1600, viewHeight: 800, inlineHash: imageHash(view) });
    const file = join(directory, prepared.record.blob);
    expect(await readFile(file)).toEqual(Buffer.from(original.data, "base64"));
    expect((await stat(file)).mode & 0o777).toBe(0o600);
    expect(await store.read(prepared.record)).toEqual(original);
    await store.prepare(original);
    expect(await readdir(directory)).toEqual([prepared.record.blob]);
  });

  it("copies referenced originals for a fork that survives deleting the source", async () => {
    const source = await setup();
    const fork = await setup();
    const prepared = await source.store.prepare(original);
    const branch = [message("source", view, prepared.record)];
    expect(await fork.store.copyFrom(source.store, branch)).toBe(0);
    await rm(source.cwd, { recursive: true });
    const loaded = await fork.store.load(resolveSavedImage("wackcode-image://entry/source/0", branch)!);
    expect(loaded.quality).toBe("original");
    expect(loaded.path.startsWith(fork.directory)).toBe(true);
    expect(await readFile(loaded.path)).toEqual(Buffer.from(original.data, "base64"));
  });

  it("honors context replacements and omissions instead of returning unrelated originals", async () => {
    const { store } = await setup();
    const prepared = await store.prepare(original);
    const entry = message("one", view, prepared.record);
    const branch = [entry, { type: "compaction", id: "compact", parentId: "one" }] as EntryLike[];
    expect(resolveSavedImage("wackcode-image://entry/one/0", branch)?.record).toEqual(prepared.record);
    const replacement = { ...view, data: "different-pixels" };
    branch.push({ type: "context_edit", id: "edit", parentId: "compact", targetId: "one", replacement: { content: [replacement] } } as EntryLike);
    expect(resolveSavedImage("wackcode-image://entry/one/0", branch)).toMatchObject({ image: replacement, record: undefined });
    branch.push({ type: "context_edit", id: "omit", parentId: "edit", targetId: "one", replacement: null } as EntryLike);
    expect(savedImages(branch)).toEqual([]);
    expect(resolveSavedImage("wackcode-image://entry/other-chat/0", [entry])).toBeUndefined();
  });

  it("identifies legacy, missing and changed originals as saved copies, including later rereads", async () => {
    const { store, directory } = await setup();
    const legacy = resolveSavedImage("wackcode-image://entry/legacy/0", [message("legacy")])!;
    const loaded = await store.load(legacy);
    expect(loaded.quality).toBe("saved-copy");
    expect(loaded.warning).toContain("original resolution was not preserved");
    const reread = resolveSavedImage("wackcode-image://entry/replay/0", [message("replay", loaded.image, loaded.record)])!;
    expect((await store.load(reread)).quality).toBe("saved-copy");
    const prepared = await store.prepare(original);
    const saved = resolveSavedImage("wackcode-image://entry/current/0", [message("current", view, prepared.record)])!;
    await writeFile(join(directory, prepared.record.blob), "changed");
    await expect(store.original(saved)).rejects.toThrow("changed");
    expect(await store.load(saved)).toMatchObject({ quality: "saved-copy", warning: expect.stringContaining("missing or changed") });
    await rm(join(directory, prepared.record.blob));
    expect((await store.load(saved)).quality).toBe("saved-copy");
  });

  it("refuses a Chat image directory symlink outside the scratchpad", async () => {
    const { store, cwd, directory } = await setup(true);
    const outside = await setup();
    await mkdir(join(cwd, ".wackcode"));
    await symlink(outside.cwd, directory);
    await expect(store.prepare(original)).rejects.toThrow("outside");
    expect(await readdir(outside.cwd)).toEqual([]);
  });
});

type Handler = (event: any, context: any) => any;
async function extensionHarness(vision = true, isChat = false) {
  const setupResult = await setup(isChat);
  let branch: EntryLike[] = [];
  const tools = new Map<string, any>();
  const handlers = new Map<string, Handler>();
  const notices: string[] = [];
  const extension = createChatImagesExtension({ supportsVision: () => vision, notice: (text) => { notices.push(text); } });
  extension.controller.configure(setupResult.store);
  extension.factory({ registerTool: (tool: any) => { tools.set(tool.name, tool); }, on: (name: string, handler: Handler) => { handlers.set(name, handler); } } as unknown as ExtensionAPI);
  const ctx = { sessionManager: { getBranch: () => branch }, cwd: setupResult.cwd } as unknown as ExtensionToolContext;
  const execute = (name: string, input = {}, signal?: AbortSignal) => tools.get(name).execute("call", input, signal, undefined, ctx);
  return { ...setupResult, tools, notices, extension, handlers, ctx, execute, setBranch(next: EntryLike[]) { branch = next; } };
}

describe("saved image tools", () => {
  it("binds attachment originals before persistence and retains their identity on retries", async () => {
    const h = await extensionHarness();
    const images = await h.extension.controller.preparePrompt([original]);
    const finalized = h.handlers.get("message_end")!({ message: { role: "user", content: images } }, h.ctx).message;
    expect(finalized.wackcodeImages.images[0].width).toBe(2400);
    const record = finalized.wackcodeImages.images[0] as StoredImage;
    await h.extension.controller.preparePrompt(images, [record]);
    const retry = h.handlers.get("message_end")!({ message: { role: "user", content: images } }, h.ctx).message;
    expect(retry.wackcodeImages.images[0]).toEqual(record);
    await h.extension.controller.preparePrompt([view], [null]);
    const legacyRetry = h.handlers.get("message_end")!({ message: { role: "user", content: [view] } }, h.ctx).message;
    expect(legacyRetry.wackcodeImages.images[0].quality).toBe("saved-copy");
  });

  it.each(["read", "browser_screenshot", "computer_screenshot", "mcp__images__export", "package_image"])("archives %s images before normalization", async (toolName) => {
    const h = await extensionHarness();
    const result = await h.handlers.get("tool_result")!({ toolName, toolCallId: "shot", content: [original] }, h.ctx);
    expect(result.content[0]).toEqual(view);
    expect(result.content[1].text).toContain("original 2400 × 1200");
    const finalized = h.handlers.get("message_end")!({ message: { role: "toolResult", toolCallId: "shot", toolName, content: result.content } }, h.ctx).message;
    const record = finalized.wackcodeImages.images[0];
    expect(await h.store.read(record)).toEqual(original);
  });

  it("paginates newest first, reopens by reference, and excludes rereads from the catalogue", async () => {
    const h = await extensionHarness();
    const prepared = await h.store.prepare(original);
    const branch = Array.from({ length: 25 }, (_, index) => message(`entry-${index}`, view, prepared.record));
    h.setBranch(branch);
    const listed = JSON.parse((await h.execute("list_images")).content[0].text);
    expect(listed).toMatchObject({ total: 25, nextOffset: 20 });
    expect(listed.images).toHaveLength(20);
    expect(listed.images[0].reference).toBe("wackcode-image://entry/entry-24/0");
    expect(JSON.parse((await h.execute("list_images", { offset: 20 })).content[0].text).images).toHaveLength(5);
    await expect(h.execute("list_images", { limit: 101 })).rejects.toThrow("1 to 100");
    const reopened = await h.execute("read_image", { reference: listed.images[0].reference });
    expect(reopened.content[0].text).toContain("Original: 2400 × 1200");
    expect(reopened.content[1]).toEqual(view);
    const finalized = h.handlers.get("message_end")!({ message: { role: "toolResult", toolName: "read_image", toolCallId: "call", content: reopened.content } }, h.ctx).message;
    branch.push({ type: "message", id: "reopened", parentId: "entry-24", message: finalized });
    expect(JSON.parse((await h.execute("list_images")).content[0].text).total).toBe(25);
    expect(savedImages(branch).at(-1)?.record?.blob).toBe(prepared.record.blob);
    for (const reference of ["/tmp/image.png", "https://example.com/image.png", "wackcode-image://entry/missing/0"]) {
      await expect(h.execute("read_image", { reference })).rejects.toThrow();
    }
  });

  it("retains a valid original when Pi omits another image from the same tool result", async () => {
    const h = await extensionHarness();
    const prepared = await h.store.prepare(original);
    // Pi may omit a source during normalization; identical surviving views are ambiguous.
    const partial = await h.handlers.get("tool_result")!({ toolName: "package_image", toolCallId: "mixed", content: [original, original] }, h.ctx);
    expect(partial.content.filter((block: { type: string }) => block.type === "image")).toHaveLength(2);
    const result = { role: "toolResult", toolName: "package_image", toolCallId: "mixed", content: [view] };
    const ambiguous = h.handlers.get("message_end")!({ message: result }, h.ctx).message;
    expect(ambiguous.wackcodeImages.images).toEqual([null]);
    const distinct: ImageContent = { ...original, data: Buffer.from("second-original").toString("base64") };
    const imageStore = new ImageStore({ ...h.store.options, resizeImage: async (bytes) => bytes.toString() === "second-original"
      ? { ...await resizeImage(), data: "second-view" } : await resizeImage() });
    h.extension.controller.configure(imageStore);
    await h.handlers.get("tool_result")!({ toolName: "package_image", toolCallId: "mixed", content: [original, distinct] }, h.ctx);
    const finalized = h.handlers.get("message_end")!({ message: result }, h.ctx).message;
    expect(finalized.wackcodeImages.images).toEqual([prepared.record]);
  });

  it("provides a scratchpad-confined file without sending images to a non-vision model", async () => {
    const h = await extensionHarness(false, true);
    const prepared = await h.store.prepare(original);
    h.setBranch([message("attached", view, prepared.record)]);
    const result = await h.execute("read_image", { reference: "wackcode-image://entry/attached/0" });
    expect(result.content).toHaveLength(1);
    expect(result.content[0].text).toContain("cannot view images");
    expect(result.content[0].text).toContain(h.directory);
    expect(await readFile(join(h.directory, prepared.record.blob))).toEqual(Buffer.from(original.data, "base64"));
  });

  it("reports save failures instead of claiming the original was retained", async () => {
    const h = await extensionHarness();
    await writeFile(join(h.cwd, "session"), "not a directory");
    await expect(h.extension.controller.preparePrompt([original])).rejects.toThrow();
    const result = await h.handlers.get("tool_result")!({ toolName: "browser_screenshot", toolCallId: "shot", content: [original] }, h.ctx);
    expect(result.content.at(-1).text).toContain("could not be saved");
    const finalized = h.handlers.get("message_end")!({ message: { role: "toolResult", toolCallId: "shot", content: [original] } }, h.ctx).message;
    expect(finalized.wackcodeImages.images).toEqual([null]);
    expect(h.notices[0]).toContain("could not be saved");
  });
});
