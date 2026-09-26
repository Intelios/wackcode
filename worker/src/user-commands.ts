/**
 * The user's own slash commands (Settings › Commands): Pi prompt templates kept in the app-owned
 * `<app data>/commands` folder, layered ahead of package prompt templates like the user skills
 * sit ahead of package skills.
 *
 * Only this folder loads — nothing is discovered — and the files are ordinary Pi templates, so
 * `/name args` expands with the same `$1` / `$ARGUMENTS` rules as a package template. A name a
 * user command shares with a package template goes to the user's (it merges first); the package
 * entry keeps working under its renamed `prompt:<name>` invocation.
 */
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join } from "node:path";
import type { PromptTemplate } from "@earendil-works/pi-coding-agent";

type PiModule = typeof import("@earendil-works/pi-coding-agent");

/** Mirrors Pi's `loadTemplateFromFile`: the filename is the name, frontmatter describes, body expands. */
export function loadUserCommands(pi: Pick<PiModule, "parseFrontmatter">, dir: string | undefined): PromptTemplate[] {
  if (!dir || !existsSync(dir)) return [];
  const templates: PromptTemplate[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const filePath = join(dir, entry.name);
    let isFile = entry.isFile();
    if (entry.isSymbolicLink()) {
      try {
        isFile = statSync(filePath).isFile();
      } catch {
        continue;
      }
    }
    if (!isFile || !entry.name.endsWith(".md") || entry.name.startsWith(".")) continue;
    try {
      const { frontmatter, body } = pi.parseFrontmatter(readFileSync(filePath, "utf-8"));
      const name = basename(filePath, ".md");
      const described = frontmatter.description;
      let description = typeof described === "string" ? described : "";
      if (!description) {
        const firstLine = body.split("\n").find((line) => line.trim());
        if (firstLine) description = firstLine.slice(0, 60) + (firstLine.length > 60 ? "..." : "");
      }
      const hint = frontmatter["argument-hint"];
      templates.push({
        name,
        description,
        ...(typeof hint === "string" ? { argumentHint: hint } : {}),
        content: body,
        sourceInfo: { path: filePath, source: "Your commands", scope: "user", origin: "top-level" },
        filePath
      });
    } catch {
      // An unreadable file just never becomes a command.
    }
  }
  return templates.sort((a, b) => a.name.localeCompare(b.name));
}

/** The user's commands first, so `/name` expands theirs before a package template of the same name. */
export function mergePrompts(user: PromptTemplate[], packages: PromptTemplate[]): PromptTemplate[] {
  if (user.length === 0) return packages;
  return [...user, ...packages];
}

/**
 * A cheap fingerprint of what `/` offers and Pi's prompt expansion matches, so a `set_commands`
 * that changed nothing can skip the system-prompt rebuild.
 */
export function commandsSignature(templates: PromptTemplate[], disabled: string[]): string {
  return JSON.stringify([
    templates.map((template) => [template.name, template.description, template.argumentHint ?? "", template.filePath, hash(template.content)]),
    [...disabled].sort()
  ]);
}

function hash(text: string): number {
  let result = 5381;
  for (let index = 0; index < text.length; index++) result = ((result * 33) ^ text.charCodeAt(index)) >>> 0;
  return result;
}
