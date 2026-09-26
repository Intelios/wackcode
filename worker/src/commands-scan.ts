/**
 * Settings › Commands: a short-lived process that lists what a chat's `/` menu offers — the
 * user's own commands folder, plus every trusted package's extension commands and prompt
 * templates — without a chat. It reads one `{ request, agentDir }` line on stdin, loads the
 * named resources through Pi's own loader, resolves names exactly like the worker's catalog,
 * prints one `CommandScanResult` line and exits.
 *
 * There is no session, no provider key and no network (PI_OFFLINE). Unlike the skills scan this
 * one *executes* trusted extension code — the only way a command's name exists at all — inside
 * this throwaway process; untrusted packages never reach it because the host only passes
 * `trusted_at` resources.
 */
import { sep } from "node:path";
import { JsonLineDecoder } from "./framing.js";
import { commandKey, resolveCommandNames, resolveInvocationNames } from "./slash.js";
import { loadUserCommands } from "./user-commands.js";
import type {
  CommandScanRequest,
  CommandScanResult,
  ScannedCommand,
  ScannedCommandGroup,
  ScannedDiagnostic
} from "./protocol.js";

process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_OFFLINE = "1";

function finish(result: CommandScanResult): never {
  process.stdout.write(`${JSON.stringify(result)}\n`);
  process.exit(0);
}

interface PoolEntry {
  kind: "extension" | "prompt" | "custom";
  /** The catalog's clash prefix: "extension" | "prompt" | "custom". */
  source: string;
  invocation: string;
  key: string;
  description?: string;
  argumentHint?: string;
  filePath?: string;
  /** Owning package's `source`; custom entries and anything unmatched have none. */
  owner?: string;
}

async function scan(request: CommandScanRequest, agentDir: string): Promise<never> {
  try {
    const pi = await import("@earendil-works/pi-coding-agent");
    // Enabled resources only, exactly what `resource_paths` hands a worker.
    const extensionPaths = request.packages.flatMap((pkg) => pkg.extensions.filter((resource) => resource.enabled).map((resource) => resource.path));
    const promptPaths = request.packages.flatMap((pkg) => pkg.prompts.filter((resource) => resource.enabled).map((resource) => resource.path));
    const loader = new pi.DefaultResourceLoader({
      cwd: request.cwd,
      agentDir,
      noExtensions: true,
      noSkills: true,
      noPromptTemplates: true,
      noThemes: true,
      noContextFiles: true,
      additionalExtensionPaths: extensionPaths,
      additionalPromptTemplatePaths: promptPaths
    });
    await loader.reload();
    const loaded = loader.getExtensions();
    const prompts = loader.getPrompts();
    const custom = loadUserCommands(pi, request.commandsDir);

    const owns = (filePath: string, kind: "extensions" | "prompts") =>
      request.packages.find((pkg) => pkg[kind].some((resource) =>
        filePath === resource.path || filePath.startsWith(resource.path.endsWith(sep) ? resource.path : `${resource.path}${sep}`)
      ))?.source;

    // The runner's own rule: clashing registered names become `name:<occurrence>` across every
    // loaded extension, in load order — so the pool reproduces `invocationName` package-blind.
    const flat = loaded.extensions.flatMap((extension) =>
      [...extension.commands.values()].map((command) => ({
        command,
        path: command.sourceInfo?.path ?? extension.resolvedPath ?? extension.path
      }))
    );
    const resolved = resolveInvocationNames(flat.map((entry) => entry.command));

    const pool: PoolEntry[] = [
      ...resolved.map((command, index) => ({
        kind: "extension" as const,
        source: "extension",
        invocation: command.invocationName,
        key: commandKey("extension", flat[index].path, command.name),
        description: command.description,
        filePath: flat[index].path,
        owner: owns(flat[index].path, "extensions")
      })),
      // The user's commands merge ahead of package templates in a chat, so they resolve first.
      ...custom.map((template) => ({
        kind: "custom" as const,
        source: "custom",
        invocation: template.name,
        key: commandKey("custom", template.filePath),
        description: template.description,
        argumentHint: template.argumentHint,
        filePath: template.filePath
      })),
      ...prompts.prompts.map((template) => ({
        kind: "prompt" as const,
        source: "prompt",
        invocation: template.name,
        key: commandKey("prompt", template.filePath),
        description: template.description,
        argumentHint: template.argumentHint,
        filePath: template.filePath,
        owner: owns(template.filePath, "prompts")
      }))
    ];

    // Names resolve over the offered set only: a switched-off command frees its name, like a chat.
    const disabled = new Set(request.disabled);
    const offered = pool.filter((entry) => !disabled.has(entry.key));
    const names = resolveCommandNames(offered.map((entry) => ({ source: entry.source, invocation: entry.invocation })));
    const resolvedName = new Map(offered.map((entry, index) => [entry, names[index]]));

    const toScanned = (entry: PoolEntry): ScannedCommand => {
      const resolved = resolvedName.get(entry);
      return {
        key: entry.key,
        name: resolved ? resolved.name : entry.invocation,
        ...(resolved?.raw ? { rawName: resolved.raw } : {}),
        ...(entry.description ? { description: entry.description } : {}),
        ...(entry.argumentHint ? { argumentHint: entry.argumentHint } : {}),
        kind: entry.kind,
        ...(entry.filePath ? { filePath: entry.filePath } : {}),
        enabled: resolved !== undefined
      };
    };

    const diagnosticOwner = (path: string | undefined, kind: "extensions" | "prompts"): string | undefined => {
      if (!path) return undefined;
      return owns(path, kind) ?? request.packages.find((pkg) => pkg.installedPath && (path === pkg.installedPath || path.startsWith(`${pkg.installedPath}${sep}`)))?.source;
    };

    const diagnostics = new Map<string, ScannedDiagnostic[]>();
    const record = (owner: string | undefined, diagnostic: ScannedDiagnostic) => {
      if (!owner) return;
      diagnostics.set(owner, [...(diagnostics.get(owner) ?? []), diagnostic]);
    };
    for (const error of loaded.errors) {
      record(diagnosticOwner(error.path, "extensions"), { type: "error", message: error.error.slice(0, 300), path: error.path });
    }
    for (const diagnostic of prompts.diagnostics) {
      record(diagnosticOwner(diagnostic.path, "prompts"), {
        type: diagnostic.type,
        message: diagnostic.message.slice(0, 300),
        ...(diagnostic.path ? { path: diagnostic.path } : {})
      });
    }

    const packages: ScannedCommandGroup[] = request.packages
      .map((pkg) => ({
        id: pkg.source,
        commands: pool.filter((entry) => entry.owner === pkg.source).map(toScanned),
        diagnostics: diagnostics.get(pkg.source) ?? []
      }))
      .filter((group) => group.commands.length > 0 || group.diagnostics.length > 0);

    finish({ ok: true, custom: pool.filter((entry) => entry.kind === "custom").map(toScanned), packages });
  } catch (error) {
    finish({ ok: false, error: error instanceof Error ? error.message : String(error) });
  }
}

const decoder = new JsonLineDecoder();
let started = false;
process.stdin.on("data", (chunk: Buffer) => {
  const [line] = decoder.push(chunk);
  if (line === undefined || started) return;
  started = true;
  process.stdin.pause();
  let parsed: { request: CommandScanRequest; agentDir: string };
  try {
    parsed = JSON.parse(line) as { request: CommandScanRequest; agentDir: string };
  } catch {
    finish({ ok: false, error: "The desktop app sent an invalid request." });
  }
  void scan(parsed.request, parsed.agentDir);
});
process.stdin.on("end", () => {
  if (!started) finish({ ok: false, error: "The desktop app sent no resources to scan." });
});
