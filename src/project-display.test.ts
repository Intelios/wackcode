import { describe, expect, it } from "vitest";
import { matchesProject, monogram, orderProjects, shortPath } from "./project-display";
import type { ProjectRecord } from "./types";

const project = (name: string, path: string): ProjectRecord => ({ id: name, name, path, gitRoot: null, gitHasHead: false, runCommand: null, branch: null, createdAt: "now" });

describe("shortPath", () => {
  it("replaces the home folder with a tilde", () => {
    expect(shortPath("/Users/jack/Documents/GitHub/wackcode")).toBe("~/Documents/GitHub/wackcode");
    expect(shortPath("/Users/jack")).toBe("~");
  });

  it("leaves paths outside /Users alone", () => {
    expect(shortPath("/tmp/scratch")).toBe("/tmp/scratch");
    expect(shortPath("/Volumes/Work/code")).toBe("/Volumes/Work/code");
  });

  it("ignores a trailing slash", () => {
    expect(shortPath("/Users/jack/code/app/")).toBe("~/code/app");
  });

  it("collapses long paths to the root and the last two folders", () => {
    expect(shortPath("/Users/jack/Documents/GitHub/clients/acme/wackcode")).toBe("~/…/acme/wackcode");
    expect(shortPath("/Volumes/Work/a/b/c/d")).toBe("/…/c/d");
  });
});

describe("monogram", () => {
  it("uses the first letter or digit, uppercased", () => {
    expect(monogram("wackcode")).toBe("W");
    expect(monogram("7zip")).toBe("7");
    expect(monogram("  _media-logger")).toBe("M");
    expect(monogram("Éclair")).toBe("É");
  });

  it("is null when the name has no letters or digits", () => {
    expect(monogram("🦆")).toBeNull();
    expect(monogram("---")).toBeNull();
  });
});

describe("matchesProject", () => {
  const item = project("TokenTrail", "/Users/jack/code/tokentrail");

  it("matches the name or the path, ignoring case", () => {
    expect(matchesProject(item, "token")).toBe(true);
    expect(matchesProject(item, "JACK/CODE")).toBe(true);
    expect(matchesProject(item, "stick")).toBe(false);
  });

  it("matches everything for an empty query", () => {
    expect(matchesProject(item, "")).toBe(true);
    expect(matchesProject(item, "   ")).toBe(true);
  });
});

describe("orderProjects", () => {
  it("floats pinned projects to the top while preserving each group’s order", () => {
    const projects = ["a", "b", "c", "d"].map((name) => project(name, `/code/${name}`));
    expect(orderProjects(projects, new Set(["d", "b"])).map((item) => item.id)).toEqual(["b", "d", "a", "c"]);
  });
});
