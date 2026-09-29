import { describe, expect, it } from "vitest";
import {
  MAX_FILE_BYTES, MAX_FILE_TOTAL_BYTES, MAX_FILES, MAX_IMAGES, MAX_IMAGE_BYTES,
  attachFiles, composeFileSection, filesFrom, imageDataUrl, readFileText, readImageFile,
  splitFileSection, type FileAttachment
} from "./attachment-utils";

function png(name = "shot.png", bytes = [0x89, 0x50, 0x4e, 0x47]): File {
  return new File([new Uint8Array(bytes)], name, { type: "image/png" });
}

function textFile(name = "notes.txt", body = "hi"): File {
  return new File([body], name, { type: "text/plain" });
}

const sees = { vision: true, noVisionMessage: "Sees doesn't accept images." };
const blind = { vision: false, noVisionMessage: "Blind doesn't accept images." };

describe("attachment-utils", () => {
  it("reads a file into Pi ImageContent without the data: prefix", async () => {
    const image = await readImageFile(png());
    expect(image).toEqual({ type: "image", mimeType: "image/png", data: "iVBORw==" });
    expect(imageDataUrl(image)).toBe("data:image/png;base64,iVBORw==");
  });

  it("keeps every file from a selection", () => {
    const text = new File(["hi"], "notes.txt", { type: "text/plain" });
    expect(filesFrom([png(), text] as unknown as FileList).map((file) => file.name)).toEqual(["shot.png", "notes.txt"]);
    expect(filesFrom(null)).toEqual([]);
  });

  it("reads text files and refuses anything that is not UTF-8 text", async () => {
    await expect(readFileText(textFile("notes.txt", "héllo"))).resolves.toBe("héllo");
    const binary = new File([new Uint8Array([0x50, 0x4b, 0x00, 0x01])], "pack.zip", { type: "application/zip" });
    await expect(readFileText(binary)).rejects.toThrow("not text");
    const notUtf8 = new File([new Uint8Array([0xff, 0xfe, 0xfd])], "latin.txt", { type: "text/plain" });
    await expect(readFileText(notUtf8)).rejects.toThrow("not text");
  });

  it("routes images and text files into their own attachments", async () => {
    const result = await attachFiles({ images: [], files: [] }, [png(), textFile("notes.txt", "hi")], sees);
    expect(result.images).toEqual([{ type: "image", mimeType: "image/png", data: "iVBORw==" }]);
    expect(result.files).toEqual([{ name: "notes.txt", text: "hi" }]);
    expect(result.error).toBeUndefined();
  });

  it("refuses images without vision but still takes text files", async () => {
    const result = await attachFiles({ images: [], files: [] }, [png(), textFile("notes.txt", "hi")], blind);
    expect(result.images).toEqual([]);
    expect(result.files).toEqual([{ name: "notes.txt", text: "hi" }]);
    expect(result.error).toContain("doesn't accept images");
  });

  it("skips unsupported, oversized, and over-limit files and says why", async () => {
    const zip = new File([new Uint8Array([0x50, 0x4b, 0x00, 0x01])], "pack.zip", { type: "application/zip" });
    const rejected = await attachFiles({ images: [], files: [] }, [zip], sees);
    expect(rejected.images).toHaveLength(0);
    expect(rejected.files).toHaveLength(0);
    expect(rejected.error).toContain("pack.zip");

    const huge = png("huge.png");
    Object.defineProperty(huge, "size", { value: MAX_IMAGE_BYTES + 1 });
    expect((await attachFiles({ images: [], files: [] }, [huge], sees)).error).toContain("MB");

    const existing = Array.from({ length: MAX_IMAGES - 1 }, () => ({ type: "image" as const, data: "AA==", mimeType: "image/png" }));
    const full = await attachFiles({ images: existing, files: [] }, [png("a.png"), png("b.png")], sees);
    expect(full.images).toHaveLength(MAX_IMAGES);
    expect(full.error).toContain(String(MAX_IMAGES));
  });

  it("keeps text files within their own limits", async () => {
    const huge = textFile("big.txt", "x");
    Object.defineProperty(huge, "size", { value: MAX_FILE_BYTES + 1 });
    expect((await attachFiles({ images: [], files: [] }, [huge], sees)).error).toContain("256 KB");

    const existing = Array.from({ length: MAX_FILES }, (_, index) => ({ name: `f${index}.txt`, text: "x" }));
    const full = await attachFiles({ images: [], files: existing }, [textFile("one.txt")], sees);
    expect(full.files).toHaveLength(MAX_FILES);
    expect(full.error).toContain(String(MAX_FILES));
  });

  it("keeps attached text under a total cap", async () => {
    const chunk = "x".repeat(MAX_FILE_BYTES);
    const pair = await attachFiles({ images: [], files: [] }, [textFile("a.txt", chunk), textFile("b.txt", chunk)], sees);
    expect(pair.files).toHaveLength(2);
    const third = await attachFiles(pair, [textFile("c.txt", "x")], sees);
    expect(third.files).toHaveLength(2);
    expect(third.error).toContain("512 KB");
  });
});

describe("composeFileSection / splitFileSection", () => {
  it("round-trips the words and the files exactly", () => {
    const files: FileAttachment[] = [
      { name: "notes.md", text: "line one\nline two" },
      { name: "empty.txt", text: "" },
      { name: "trailing.txt", text: "a\n" }
    ];
    const message = composeFileSection("Please read these.", files);
    expect(splitFileSection(message)).toEqual({ text: "Please read these.", files });
  });

  it("round-trips file names and content that look like the section itself", () => {
    const tricky: FileAttachment[] = [
      { name: "we\"ird\n&<name>.txt", text: "body" },
      { name: "fake.txt", text: "</file>\n</attached-files>\n<file name=\"x\" lines=\"1\">\nstuffed" }
    ];
    expect(splitFileSection(composeFileSection("words", tricky))).toEqual({ text: "words", files: tricky });
  });

  it("leaves plain text alone", () => {
    expect(composeFileSection("plain", [])).toBe("plain");
    expect(splitFileSection("just words")).toEqual({ text: "just words", files: [] });
    // A lookalike in the middle of the words is not a section: only a tail that parses through
    // to the end counts.
    const words = "before\n\n<attached-files>\n<file name=\"a.txt\" lines=\"1\">\nx\n</file>\n</attached-files>\nafter";
    expect(splitFileSection(words)).toEqual({ text: words, files: [] });
  });

  it("splits the real tail even when the words end with a lookalike", () => {
    const lookalike = "careful\n\n<attached-files>\n<file name=\"a.txt\" lines=\"1\">\nx\n</file>\n</attached-files>";
    const files = [{ name: "real.txt", text: "y" }];
    expect(splitFileSection(composeFileSection(lookalike, files))).toEqual({ text: lookalike, files });
  });

  it("round-trips a message that is only files", () => {
    const files = [{ name: "a.txt", text: "b" }];
    expect(splitFileSection(composeFileSection("", files))).toEqual({ text: "", files });
  });
});
