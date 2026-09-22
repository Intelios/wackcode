import { describe, expect, it } from "vitest";
import { MAX_IMAGES, MAX_IMAGE_BYTES, attachImages, imageDataUrl, imageFilesFrom, readImageFile } from "./attachment-utils";

function png(name = "shot.png", bytes = [0x89, 0x50, 0x4e, 0x47]): File {
  return new File([new Uint8Array(bytes)], name, { type: "image/png" });
}

describe("attachment-utils", () => {
  it("reads a file into Pi ImageContent without the data: prefix", async () => {
    const image = await readImageFile(png());
    expect(image).toEqual({ type: "image", mimeType: "image/png", data: "iVBORw==" });
    expect(imageDataUrl(image)).toBe("data:image/png;base64,iVBORw==");
  });

  it("keeps only image files from a selection", () => {
    const text = new File(["hi"], "notes.txt", { type: "text/plain" });
    expect(imageFilesFrom([png(), text] as unknown as FileList).map((file) => file.name)).toEqual(["shot.png"]);
    expect(imageFilesFrom(null)).toEqual([]);
  });

  it("skips unsupported, oversized, and over-limit files and says why", async () => {
    const svg = new File(["<svg/>"], "logo.svg", { type: "image/svg+xml" });
    const rejected = await attachImages([], [svg]);
    expect(rejected.images).toHaveLength(0);
    expect(rejected.error).toContain("logo.svg");

    const huge = png("huge.png");
    Object.defineProperty(huge, "size", { value: MAX_IMAGE_BYTES + 1 });
    expect((await attachImages([], [huge])).error).toContain("MB");

    const existing = Array.from({ length: MAX_IMAGES - 1 }, () => ({ type: "image" as const, data: "AA==", mimeType: "image/png" }));
    const full = await attachImages(existing, [png("a.png"), png("b.png")]);
    expect(full.images).toHaveLength(MAX_IMAGES);
    expect(full.error).toContain(String(MAX_IMAGES));
  });
});
