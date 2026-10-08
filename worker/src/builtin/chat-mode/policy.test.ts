import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ASK_USER_QUESTION_TOOL_NAME } from "../ask-user-question.js";
import { BROWSER_TOOL_NAMES } from "../browser.js";
import { MEMORY_TOOL_NAMES } from "../memory/index.js";
import { WEB_FETCH_TOOL_NAME } from "../web-fetch/index.js";
import { chatToolAllowed, chatToolDecision, confine } from "./policy.js";

describe("Chat mode's tool allowlist", () => {
  it("keeps Pi's file tools and drops the shell", () => {
    for (const name of ["read", "write", "edit", "ls", "grep", "find"]) expect(chatToolAllowed(name, "builtin")).toBe(true);
    expect(chatToolAllowed("bash", "builtin")).toBe(false);
  });

  it("keeps the questionnaire, web fetch, memory and the browser, and nothing else of WackCode's", () => {
    for (const name of [ASK_USER_QUESTION_TOOL_NAME, WEB_FETCH_TOOL_NAME, ...MEMORY_TOOL_NAMES, ...BROWSER_TOOL_NAMES]) {
      expect(chatToolAllowed(name, "wackcode")).toBe(true);
    }
    for (const name of ["bash_job", "todo", "plan_mode_complete", "subagent", "subagent_job", "skill_creator", "computer_act", "computer_open"]) {
      expect(chatToolAllowed(name, "wackcode")).toBe(false);
    }
  });

  it("keeps the user's MCP tools and never a package tool, whatever it is called", () => {
    expect(chatToolAllowed("mcp__notes__search", "mcp")).toBe(true);
    expect(chatToolAllowed("read", "package")).toBe(false);
    expect(chatToolAllowed("web_fetch", "package")).toBe(false);
  });
});

describe("scratchpad confinement", () => {
  let base: string;
  let scratch: string;
  let outside: string;

  beforeEach(() => {
    // macOS's temp folder is itself behind a symlink; the policy compares real paths.
    base = realpathSync.native(mkdtempSync(join(tmpdir(), "wackcode-chat-policy-")));
    scratch = join(base, "scratch");
    outside = join(base, "scratch-other");
    mkdirSync(join(scratch, "notes"), { recursive: true });
    mkdirSync(outside);
    writeFileSync(join(scratch, "notes", "a.md"), "inside");
    writeFileSync(join(outside, "secret.txt"), "outside");
  });
  afterEach(() => rmSync(base, { recursive: true, force: true }));

  const allowed = (tool: string, path: unknown) => confine(tool, path, scratch);

  it("allows files in the scratchpad and spells them relative to it", () => {
    expect(allowed("read", "notes/a.md")).toEqual({ ok: true, path: "notes/a.md" });
    expect(allowed("read", join(scratch, "notes", "a.md"))).toEqual({ ok: true, path: "notes/a.md" });
    expect(allowed("edit", "./notes/../notes/a.md")).toEqual({ ok: true, path: "notes/a.md" });
    expect(allowed("read", "@notes/a.md")).toEqual({ ok: true, path: "notes/a.md" });
    expect(allowed("read", pathToFileURL(join(scratch, "notes", "a.md")).href)).toEqual({ ok: true, path: "notes/a.md" });
    expect(allowed("ls", ".")).toEqual({ ok: true, path: "." });
    expect(allowed("ls", scratch)).toEqual({ ok: true, path: "." });
  });

  it("allows a new file, including one in a folder that does not exist yet", () => {
    expect(allowed("write", "draft.md")).toEqual({ ok: true, path: "draft.md" });
    expect(allowed("write", "new/deep/draft.md")).toEqual({ ok: true, path: "new/deep/draft.md" });
  });

  it("leaves a missing path alone for the tools that default to the scratchpad", () => {
    for (const tool of ["ls", "grep", "find"]) expect(allowed(tool, undefined)).toEqual({ ok: true });
    for (const tool of ["read", "write", "edit"]) expect(allowed(tool, undefined).ok).toBe(false);
    expect(allowed("read", 7).ok).toBe(false);
  });

  it("refuses every way of naming a file outside", () => {
    const escapes = [
      "../scratch-other/secret.txt",
      "notes/../../scratch-other/secret.txt",
      join(outside, "secret.txt"),
      "/etc/hosts",
      "~",
      "~/.ssh/id_rsa",
      `@${join(outside, "secret.txt")}`,
      pathToFileURL(join(outside, "secret.txt")).href,
      "file://example.com/etc/hosts",
      "..",
    ];
    for (const tool of ["read", "write", "edit", "ls", "grep", "find"]) {
      for (const path of escapes) expect(allowed(tool, path), `${tool} ${path}`).toMatchObject({ ok: false });
    }
    expect(homedir().startsWith(scratch)).toBe(false);
  });

  it("is not fooled by a sibling folder whose name starts with the scratchpad's", () => {
    // "scratch-other" begins with "scratch": a bare prefix test would let it through.
    expect(allowed("read", `${scratch}-other/secret.txt`).ok).toBe(false);
  });

  it("refuses a symlink that leaves the scratchpad, as a folder, a file, or dangling", () => {
    symlinkSync(outside, join(scratch, "link-dir"));
    symlinkSync(join(outside, "secret.txt"), join(scratch, "link-file"));
    symlinkSync(join(outside, "not-there.txt"), join(scratch, "dangling"));
    expect(allowed("read", "link-dir/secret.txt").ok).toBe(false);
    expect(allowed("write", "link-dir/new.txt").ok).toBe(false);
    expect(allowed("ls", "link-dir").ok).toBe(false);
    expect(allowed("read", "link-file").ok).toBe(false);
    expect(allowed("edit", "link-file").ok).toBe(false);
    // Writing through a dangling link would create the file it points at.
    expect(allowed("write", "dangling").ok).toBe(false);
  });

  it("follows a symlink that stays inside, and hands Pi the real location", () => {
    symlinkSync(join(scratch, "notes"), join(scratch, "alias"));
    expect(allowed("read", "alias/a.md")).toEqual({ ok: true, path: "notes/a.md" });
  });

  it("refuses a read whose filename fallback would land outside", () => {
    // Pi retries a missing "x AM.png" as "x AM.png" (macOS screenshot names).
    symlinkSync(join(outside, "secret.txt"), join(scratch, "shot AM.png"));
    expect(allowed("read", "shot AM.png").ok).toBe(false);
  });

  it("reads a file Pi only finds through a filename fallback", () => {
    writeFileSync(join(scratch, "shot PM.png"), "image");
    // The name holds a unicode space Pi would rewrite, so the model's own spelling is kept.
    expect(allowed("read", "shot PM.png")).toEqual({ ok: true });
  });

  it("says a missing file is missing rather than outside", () => {
    const result = allowed("read", "nope.md");
    expect(result).toMatchObject({ ok: false });
    expect(result.ok === false && result.reason).toContain("no file");
  });

  it("keeps a spelling Pi would misread from being rewritten into one", () => {
    mkdirSync(join(scratch, "~"));
    writeFileSync(join(scratch, "~", "x.md"), "tilde folder");
    writeFileSync(join(scratch, "@handle.md"), "at file");
    expect(allowed("read", "./~/x.md")).toEqual({ ok: true, path: "./~/x.md" });
    expect(allowed("read", "./@handle.md")).toEqual({ ok: true, path: "./@handle.md" });
  });

  it("refuses everything when the scratchpad itself is gone", () => {
    expect(confine("write", "a.md", join(base, "missing")).ok).toBe(false);
  });

  describe("the tool-call decision", () => {
    const owns = (name: string) => name.startsWith("mcp__notes__");

    it("rewrites an allowed file call's path in place", () => {
      const input = { path: join(scratch, "notes", "a.md"), offset: 1 };
      expect(chatToolDecision("read", input, scratch, owns)).toBeUndefined();
      expect(input).toEqual({ path: "notes/a.md", offset: 1 });
    });

    it("blocks an outside file call and leaves its arguments untouched", () => {
      const input = { path: "/etc/hosts" };
      expect(chatToolDecision("read", input, scratch, owns)).toMatchObject({ block: true });
      expect(input).toEqual({ path: "/etc/hosts" });
    });

    it("blocks a file call with no working directory or no arguments", () => {
      expect(chatToolDecision("read", { path: "notes/a.md" }, undefined, owns)).toMatchObject({ block: true });
      expect(chatToolDecision("read", undefined, scratch, owns)).toMatchObject({ block: true });
    });

    it("passes the chat's own tools and the user's MCP tools, and blocks the rest by name", () => {
      expect(chatToolDecision(WEB_FETCH_TOOL_NAME, { url: "https://example.com" }, scratch, owns)).toBeUndefined();
      expect(chatToolDecision("mcp__notes__search", {}, scratch, owns)).toBeUndefined();
      for (const name of ["bash", "bash_job", "subagent", "todo", "computer_act", "mcp__other__tool", "anything_else"]) {
        expect(chatToolDecision(name, {}, scratch, owns), name).toMatchObject({ block: true });
      }
    });
  });
});
