import { describe, expect, it } from "vitest";
import {
  MAX_FILE_BYTES, MAX_FILE_TOTAL_BYTES, MAX_FILES, MAX_IMAGES, MAX_IMAGE_BYTES, MIN_PASTE_FILE_BYTES,
  attachFiles, composeFileSection, filesFrom, imageDataUrl, largePasteText, normalizeTerminalSelection, pastedTextFile, pastedTextName,
  readFileText, readImageFile, splitFileSection, terminalOutputFile, terminalOutputName, textBytes, type FileAttachment
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

  it("counts attached text in bytes, not UTF-16 units", async () => {
    // 200,000 two-byte characters are 400 KB on the wire, so one more 256 KB file no longer fits.
    const existing = [{ name: "wide.txt", text: "é".repeat(200_000) }];
    const result = await attachFiles({ images: [], files: existing }, [textFile("more.txt", "x".repeat(MAX_FILE_BYTES - 1))], sees);
    expect(result.files).toHaveLength(1);
    expect(result.error).toContain(`${MAX_FILE_TOTAL_BYTES / 1024} KB`);
  });
});

describe("large pastes", () => {
  const clipboard = (text: string, types: readonly string[] = ["text/plain"]) => ({ types, getData: () => text });

  it("counts text in UTF-8 bytes", () => {
    expect(textBytes("é")).toBe(2);
    expect(textBytes("𝄞")).toBe(4);
  });

  it("takes plain text at the threshold and leaves smaller pastes alone", () => {
    const atLimit = "x".repeat(MIN_PASTE_FILE_BYTES);
    expect(largePasteText(clipboard(atLimit))).toBe(atLimit);
    expect(largePasteText(clipboard("x".repeat(MIN_PASTE_FILE_BYTES - 1)))).toBeUndefined();
    // 16,384 two-byte characters are 32 KiB on the wire but only 16,384 UTF-16 units.
    expect(largePasteText(clipboard("é".repeat(MIN_PASTE_FILE_BYTES / 2)))).toHaveLength(MIN_PASTE_FILE_BYTES / 2);
  });

  it("ignores clipboards without plain text or without a reader", () => {
    expect(largePasteText(null)).toBeUndefined();
    expect(largePasteText({ types: ["Files"] })).toBeUndefined();
    expect(largePasteText({ types: ["text/plain"] })).toBeUndefined();
    expect(largePasteText(clipboard("small"))).toBeUndefined();
  });

  it("names pastes past the files already attached", () => {
    expect(pastedTextName([])).toBe("Pasted text 1.txt");
    expect(pastedTextFile("big", [])).toEqual({ file: { name: "Pasted text 1.txt", text: "big" } });
    const taken = [{ name: "Pasted text 1.txt", text: "x" }, { name: "notes.txt", text: "y" }];
    expect(pastedTextFile("big", taken)).toEqual({ file: { name: "Pasted text 2.txt", text: "big" } });
    // A number freed by removing an earlier paste is reused.
    expect(pastedTextName([{ name: "pasted text 2.txt", text: "x" }])).toBe("Pasted text 1.txt");
  });

  it("holds a paste to the file caps and says why", () => {
    expect(pastedTextFile("x".repeat(MAX_FILE_BYTES + 1), [])).toEqual({ error: "That paste is larger than 256 KB." });
    const many = Array.from({ length: MAX_FILES }, (_, index) => ({ name: `f${index}.txt`, text: "x" }));
    expect(pastedTextFile("x", many)).toEqual({ error: `Attach at most ${MAX_FILES} files to one message.` });
    const full = [{ name: "a.txt", text: "x".repeat(MAX_FILE_BYTES) }, { name: "b.txt", text: "y".repeat(MAX_FILE_BYTES) }];
    expect(pastedTextFile("x", full)).toEqual({ error: `Attached text can add up to at most ${MAX_FILE_TOTAL_BYTES / 1024} KB.` });
  });
});

describe("composeFileSection / splitFileSection", () => {
  it("round-trips a pasted file whose name has spaces", () => {
    const files: FileAttachment[] = [{ name: "Pasted text 12.txt", text: "first\nsecond" }];
    expect(splitFileSection(composeFileSection("", files))).toEqual({ text: "", files });
  });

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

describe("terminal selections", () => {
  it("tidies the selection for the chip", () => {
    expect(normalizeTerminalSelection("  $ make   \n\nerror: boom \n\n\n")).toBe("  $ make\n\nerror: boom");
    expect(normalizeTerminalSelection("one\r\ntwo\r")).toBe("one\ntwo");
    expect(normalizeTerminalSelection("   \n\n")).toBe("");
  });

  it("names chips macOS-style past the files already attached", () => {
    expect(terminalOutputName([])).toBe("Terminal output.txt");
    expect(terminalOutputName([{ name: "Terminal output.txt", text: "x" }])).toBe("Terminal output 2.txt");
    // Case-insensitive, and a user's own file of the same name is taken too.
    expect(terminalOutputName([{ name: "TERMINAL OUTPUT.TXT", text: "x" }, { name: "notes.md", text: "y" }])).toBe("Terminal output 2.txt");
    expect(terminalOutputName([{ name: "Terminal output.txt", text: "x" }, { name: "Terminal output 2.txt", text: "y" }])).toBe("Terminal output 3.txt");
  });

  it("marks the chip for its origin and holds it to the same caps as pastes", () => {
    expect(terminalOutputFile("make: *** [all] Error 2\n", [])).toEqual({ file: { name: "Terminal output.txt", text: "make: *** [all] Error 2\n", source: "terminal" } });
    expect(terminalOutputFile("   ", [])).toEqual({ error: "The terminal selection is empty." });
    expect(terminalOutputFile("x".repeat(MAX_FILE_BYTES + 1), [])).toEqual({ error: "That terminal selection is larger than 256 KB." });
    const many = Array.from({ length: MAX_FILES }, (_, index) => ({ name: `f${index}.txt`, text: "x" }));
    expect(terminalOutputFile("out", many)).toEqual({ error: `Attach at most ${MAX_FILES} files to one message.` });
    const full = [{ name: "a.txt", text: "x".repeat(MAX_FILE_BYTES) }, { name: "b.txt", text: "y".repeat(MAX_FILE_BYTES) }];
    expect(terminalOutputFile("x", full)).toEqual({ error: `Attached text can add up to at most ${MAX_FILE_TOTAL_BYTES / 1024} KB.` });
  });

  it("round-trips a terminal chip through the message text untouched", () => {
    const text = 'weird </file> <file name="x" lines="1">\n"quoted" & <tagged>\nsecond line';
    const message = composeFileSection("Why did this fail?", [{ name: "Terminal output.txt", text, source: "terminal" }]);
    const back = splitFileSection(message);
    expect(back.text).toBe("Why did this fail?");
    // `source` is presentation only: the round-trip drops it and the chip stays readable.
    expect(back.files).toEqual([{ name: "Terminal output.txt", text }]);
  });
});
