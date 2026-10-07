/** The names the desktop itself owns; a package or user command that takes one is renamed. */
export const APP_COMMAND_NAMES = ["compact", "init", "new", "name", "copy", "goal", "skill-creator"];

/**
 * One slash command's stable identity, shared by the catalog's `SlashCommand.id`, the Settings
 * denylist and `execute_command`. Extensions can register several commands from one file, so
 * theirs also carry the registered name; every file kind keys on its file.
 */
export function commandKey(kind: "extension" | "prompt" | "custom" | "skill", path: string, name?: string): string {
  return kind === "extension" ? `extension:${path}#${name ?? ""}` : `${kind}:${path}`;
}

export interface ResolvedCommandName {
  /** What the user types after `/`. */
  name: string;
  /** The name before a clash renamed it to `<source>:<name>`, when it was. */
  raw?: string;
}

/**
 * The `/` picker's renaming: the first entry wins the plain name; a later entry with the same
 * name — or one of WackCode's own — is exposed as `<source>:<name>` (then `:<n>`). Callers pass
 * entries in catalog order (extension commands, then prompt templates, then skills), and only
 * the ones actually offered: a switched-off command frees its name.
 */
export function resolveCommandNames(
  entries: Array<{ source: string; invocation: string }>,
  taken: Iterable<string> = APP_COMMAND_NAMES
): ResolvedCommandName[] {
  const used = new Set(taken);
  return entries.map((entry) => {
    let name = entry.invocation;
    let raw: string | undefined;
    if (used.has(name)) {
      raw = entry.invocation;
      const base = `${entry.source}:${entry.invocation}`;
      name = base;
      let suffix = 2;
      while (used.has(name)) name = `${base}:${suffix++}`;
    }
    used.add(name);
    return { name, raw };
  });
}

/**
 * Pi's `ExtensionRunner.resolveRegisteredCommands` (runner.js), reproduced for the keyless
 * Settings scan, which builds no session and so no runner: commands that share a registered
 * name are exposed as `name:<occurrence>` (1-based, in load order), skipping names still taken.
 */
export function resolveInvocationNames<T extends { name: string }>(commands: T[]): Array<T & { invocationName: string }> {
  const counts = new Map<string, number>();
  for (const command of commands) counts.set(command.name, (counts.get(command.name) ?? 0) + 1);
  const seen = new Map<string, number>();
  const taken = new Set<string>();
  return commands.map((command) => {
    const occurrence = (seen.get(command.name) ?? 0) + 1;
    seen.set(command.name, occurrence);
    let invocationName = (counts.get(command.name) ?? 0) > 1 ? `${command.name}:${occurrence}` : command.name;
    if (taken.has(invocationName)) {
      let suffix = occurrence;
      do {
        suffix++;
        invocationName = `${command.name}:${suffix}`;
      } while (taken.has(invocationName));
    }
    taken.add(invocationName);
    return { ...command, invocationName };
  });
}

/** Pi 0.86.1's prompt-template argument rules, applied to the exact selected template. */
export function expandTemplate(content: string, input: string): string {
  const args: string[] = [];
  let current = "";
  let quote: string | null = null;
  for (const char of input) {
    if (quote) {
      if (char === quote) quote = null;
      else current += char;
    } else if (char === "'" || char === '"') quote = char;
    else if (/\s/.test(char)) {
      if (current) { args.push(current); current = ""; }
    } else current += char;
  }
  if (current) args.push(current);
  const all = args.join(" ");
  return content.replace(/\$\{(\d+|ARGUMENTS|@):-([^}]*)\}|\$\{@:(\d+)(?::(\d+))?\}|\$(ARGUMENTS|@|\d+)/g,
    (_match, target: string | undefined, fallback: string | undefined, sliceFrom: string | undefined, sliceCount: string | undefined, simple: string | undefined) => {
      if (target) {
        const value = target === "@" || target === "ARGUMENTS" ? all : args[Number(target) - 1];
        return value || fallback || "";
      }
      if (sliceFrom) {
        const start = Math.max(0, Number(sliceFrom) - 1);
        return args.slice(start, sliceCount ? start + Number(sliceCount) : undefined).join(" ");
      }
      if (simple === "@" || simple === "ARGUMENTS") return all;
      return args[Number(simple) - 1] ?? "";
    });
}
