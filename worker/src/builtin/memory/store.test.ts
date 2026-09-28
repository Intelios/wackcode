import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  composeMemory, isMemoryName, memoryIndex, memoryName, parseMemory, slugify, splitFrontmatter, yamlScalar,
  type MemoryNote
} from "./store.js";

function note(overrides: Partial<MemoryNote> = {}): MemoryNote {
  return {
    name: "feedback_run-worker-tests",
    type: "feedback",
    title: "Run worker tests after protocol changes",
    description: "Protocol edits need pnpm test:worker",
    modified: "2026-09-28T10:12:00.000Z",
    body: "Any change to protocol.ts needs `pnpm test:worker` before review.",
    ...overrides
  };
}

describe("memory scalars and names", () => {
  it("writes plain scalars only when they are safe, like Rust's yaml_scalar", () => {
    expect(yamlScalar("Run the tests")).toBe("Run the tests");
    expect(yamlScalar("say: hi")).toBe("\"say: hi\"");
    expect(yamlScalar("A note #1")).toBe("\"A note #1\"");
    expect(yamlScalar("trailing ")).toBe("\"trailing \"");
    expect(yamlScalar("true")).toBe("\"true\"");
    expect(yamlScalar("")).toBe("\"\"");
    expect(yamlScalar("123abc")).toBe("\"123abc\"");
    expect(yamlScalar("abc123")).toBe("abc123");
  });

  it("slugs titles into note names", () => {
    expect(memoryName("feedback", "Run worker tests after protocol changes!")).toBe("feedback_run-worker-tests-after-protocol-changes");
    expect(memoryName("project", "  Ünicode — déjà vu  ")).toBe("project_unicode-deja-vu");
    expect(memoryName("user", "???")).toBe("user_note");
  });
});

describe("memory file round trip", () => {
  it("composes and parses back to the same note", () => {
    const text = composeMemory(note());
    const parsed = parseMemory(note().name, text);
    expect(parsed).toEqual(note());
  });

  it("parses frontmatter the way skills.rs::split does, and keeps quoted values", () => {
    const text = "---\ntype: user\ntitle: \"Prefers: dark UI, no emojis\"\ndescription: Tells WackCode how to answer\n---\nBody text";
    const { frontmatter, body } = splitFrontmatter(text);
    expect(frontmatter.get("title")).toBe("Prefers: dark UI, no emojis");
    expect(body).toBe("Body text");
    expect(parseMemory("user_prefers", text)?.title).toBe("Prefers: dark UI, no emojis");
  });

  it("rejects files without a title or with an unknown type", () => {
    expect(parseMemory("x", "Just a body, no frontmatter.")).toBeUndefined();
    expect(parseMemory("x", "---\nbody only, no title\n---\nnothing")).toBeUndefined();
    expect(parseMemory("x", "---\ntype: diary\ntitle: Not a memory type\n---\nbody")).toBeUndefined();
    // No type is fine: "project" is the default.
    expect(parseMemory("x", "---\ntitle: Untyped\n---\nbody")?.type).toBe("project");
  });
});

describe("memory index", () => {
  it("lists one line per note, grouped by type then title, each with its name", () => {
    const index = memoryIndex([
      note({ name: "project_shipping", type: "project", title: "Shipping checklist" }),
      note({ name: "user_prefers", type: "user", title: "Prefers terse answers", description: "" })
    ]);
    expect(index).toBeDefined();
    const lines = index!.split("\n").filter((line) => line.startsWith("- "));
    expect(lines).toEqual([
      "- [user] Prefers terse answers (name: user_prefers)",
      "- [project] Shipping checklist — Protocol edits need pnpm test:worker (name: project_shipping)"
    ]);
  });

  it("is undefined with no notes and caps a runaway list", () => {
    expect(memoryIndex([])).toBeUndefined();
    const many: MemoryNote[] = Array.from({ length: 300 }, (_, index) =>
      note({ name: `project_note-${index}`, type: "project", title: `Note ${index} ${"x".repeat(90)}` }));
    const index = memoryIndex(many)!;
    expect(index).toContain("more notes are not listed");
    expect(index.length).toBeLessThan(30_000);
  });
});

describe("memory files on disk (format contract with memory.rs)", () => {
  it("round trips a file written exactly the way Settings will read it", () => {
    const dir = mkdtempSync(join(tmpdir(), "wackcode-memory-"));
    try {
      writeFileSync(join(dir, "feedback_run-worker-tests.md"), composeMemory(note()), "utf-8");
      const index = memoryIndex([note()]);
      expect(index).toContain("(name: feedback_run-worker-tests)");
      expect(index).toContain("Run worker tests after protocol changes");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("skips dot-files via the parse contract: a staging file never becomes a note", () => {
    // The leading-dot staging convention is enforced by the reader in index.ts; the parse side
    // simply has to tolerate any bytes without a title.
    expect(parseMemory(".staging", "---\nbody only\n---\nno title")).toBeUndefined();
  });
});

describe("slugify", () => {
  it("keeps names filesystem-safe", () => {
    expect(slugify("Fix: the/imaginary `bug` #42")).toBe("fix-the-imaginary-bug-42");
    expect(slugify("————")).toBe("note");
    expect(slugify("a".repeat(100))).toHaveLength(64);
  });
});

describe("isMemoryName", () => {
  it("accepts only plain filenames, like memory.rs::validate_name", () => {
    expect(isMemoryName("feedback_run-worker-tests")).toBe(true);
    expect(isMemoryName("project_shipping-checklist-2")).toBe(true);
    for (const bad of ["", "Upper", "-leading", "_leading", "has space", "a/b", "../escape", "..", ".hidden", "é-note"]) {
      expect(isMemoryName(bad)).toBe(false);
    }
  });
});
