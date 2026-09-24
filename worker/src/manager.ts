import { readFile } from "node:fs/promises";
import { isAbsolute, relative, resolve } from "node:path";
import { JsonLineDecoder } from "./framing.js";
import {
  FRAME,
  RESOURCE_KINDS,
  type ManagerCommand,
  type ManagerInit,
  type ManagerOutput,
  type ManagerPackage,
  type ManagerResource,
  type ResourceKind
} from "./manager-protocol.js";

process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.AI_AGENT = "pi";
process.env.PI_CODING_AGENT = "true";
// Deliberately NOT PI_OFFLINE: installing a package is the one thing this process exists for,
// and Pi gates installs on that flag.

type PiModule = typeof import("@earendil-works/pi-coding-agent");
type SettingsManager = ReturnType<PiModule["SettingsManager"]["inMemory"]>;
type PackageManager = InstanceType<PiModule["DefaultPackageManager"]>;

let piDir: string | undefined;
let settingsManager: SettingsManager | undefined;
let packageManager: PackageManager | undefined;
let commandQueue = Promise.resolve();

// Every protocol line is framed so npm's inherited stdout cannot corrupt the stream.
// This must stay the only writer to stdout in this process.
function send(output: ManagerOutput): void {
  process.stdout.write(`${FRAME}${JSON.stringify(output)}\n`);
}

function safeError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.replace(/(sk-[A-Za-z0-9_-]{8,}|Bearer\s+[A-Za-z0-9._-]+)/gi, "[credential redacted]");
}

function sourceKind(source: string): ManagerPackage["kind"] {
  if (source.startsWith("npm:")) return "npm";
  if (source.startsWith("git:") || /^(https?|ssh|git):\/\//.test(source)) return "git";
  return "local";
}

// "npm:@scope/name@1.2.3" -> "@scope/name"; git and local sources keep their tail segment.
function displayName(source: string): string {
  if (source.startsWith("npm:")) {
    const spec = source.slice(4);
    const at = spec.lastIndexOf("@");
    return at > 0 ? spec.slice(0, at) : spec;
  }
  const withoutRef = source.replace(/@[^/@]+$/, "");
  return withoutRef.split("/").filter(Boolean).pop() ?? source;
}

/**
 * Pi rewrites local sources to a path relative to the settings file so settings stay portable,
 * so the string in settings may not be the one the caller holds. npm and git sources are
 * compared verbatim; local ones are compared as resolved absolute paths.
 */
function sameSource(stored: string, wanted: string): boolean {
  if (stored === wanted) return true;
  if (!piDir || sourceKind(stored) !== "local" || sourceKind(wanted) !== "local") return false;
  const absolute = (value: string) => (isAbsolute(value) ? value : resolve(piDir as string, value));
  return absolute(stored) === absolute(wanted);
}

async function packageVersion(installedPath?: string): Promise<string | undefined> {
  if (!installedPath) return undefined;
  try {
    const manifest = JSON.parse(await readFile(`${installedPath}/package.json`, "utf8")) as { version?: string };
    return typeof manifest.version === "string" ? manifest.version : undefined;
  } catch {
    return undefined;
  }
}

async function initialize(command: ManagerInit): Promise<void> {
  if (packageManager) throw new Error("The package manager is already initialized");
  piDir = command.piDir;
  const pi = await import("@earendil-works/pi-coding-agent");
  settingsManager = pi.SettingsManager.create(command.piDir, command.piDir, { projectTrusted: false });
  if (command.npmCommand?.length) settingsManager.setNpmCommand(command.npmCommand);
  packageManager = new pi.DefaultPackageManager({
    cwd: command.piDir,
    agentDir: command.piDir,
    settingsManager
  });
  packageManager.setProgressCallback((event) => {
    send({ type: "progress", action: event.action, phase: event.type, source: event.source, message: event.message });
  });
  await settingsManager.flush();
}

/**
 * Resolve every configured package into its individual resource files, each flagged with whether
 * the user's filters currently enable it. This is the render model for the per-resource toggles
 * and the source of the absolute paths a task worker is handed.
 */
async function buildCatalog(): Promise<ManagerPackage[]> {
  if (!packageManager) throw new Error("The package manager is not initialized");
  const configured = packageManager.listConfiguredPackages();
  const resolved = await packageManager.resolve();

  const packages = new Map<string, ManagerPackage>();
  for (const entry of configured) {
    packages.set(entry.source, {
      source: entry.source,
      displayName: displayName(entry.source),
      kind: sourceKind(entry.source),
      installedPath: entry.installedPath,
      extensions: [],
      skills: [],
      prompts: [],
      themes: [],
      errors: []
    });
  }

  for (const kind of RESOURCE_KINDS) {
    for (const resource of resolved[kind]) {
      // Only package-origin resources belong to a package row; top-level paths are not shown.
      if (resource.metadata.origin !== "package") continue;
      const entry = packages.get(resource.metadata.source);
      if (!entry) continue;
      entry[kind].push(toResource(resource.path, resource.enabled, entry.installedPath));
    }
  }

  for (const entry of packages.values()) {
    entry.version = await packageVersion(entry.installedPath);
    for (const kind of RESOURCE_KINDS) {
      entry[kind].sort((left, right) => left.name.localeCompare(right.name));
    }
  }
  return [...packages.values()].sort((left, right) => left.displayName.localeCompare(right.displayName));
}

function toResource(path: string, enabled: boolean, installedPath?: string): ManagerResource {
  const name = installedPath ? relative(installedPath, path) : path;
  return { path, name: name && !name.startsWith("..") ? name : path, enabled };
}

/**
 * Persist per-resource toggles as Pi's own PackageSource object form. An omitted kind means
 * "load all of it"; `[]` means none. Exact relative paths are used rather than globs so a
 * toggle means exactly what it says.
 */
function applyResourceFilters(
  source: string,
  filters: Partial<Record<ResourceKind, string[] | undefined>>
): void {
  if (!settingsManager) throw new Error("The package manager is not initialized");
  const packages = settingsManager.getPackages();
  const index = packages.findIndex((entry) => sameSource(typeof entry === "string" ? entry : entry.source, source));
  if (index === -1) throw new Error(`That package is not installed: ${source}`);
  const next: Record<string, unknown> = { source };
  for (const kind of RESOURCE_KINDS) {
    const selection = filters[kind];
    if (selection !== undefined) next[kind] = selection;
  }
  packages[index] = next as (typeof packages)[number];
  settingsManager.setPackages(packages);
}

async function handle(command: ManagerCommand): Promise<void> {
  try {
    if (command.type === "init") {
      await initialize(command);
    } else if (!packageManager || !settingsManager) {
      throw new Error("The package manager is not initialized");
    } else if (command.type === "list") {
      send({ type: "catalog", packages: await buildCatalog() });
    } else if (command.type === "install") {
      await packageManager.installAndPersist(command.source);
      // Filtered before the catalogue is built, so no worker ever starts with the package's
      // extensions switched on.
      if (command.onlySkills) applyResourceFilters(command.source, { extensions: [], prompts: [], themes: [] });
      await settingsManager.flush();
      send({ type: "catalog", packages: await buildCatalog() });
    } else if (command.type === "remove") {
      await packageManager.removeAndPersist(command.source);
      await settingsManager.flush();
      send({ type: "catalog", packages: await buildCatalog() });
    } else if (command.type === "update") {
      await packageManager.update(command.source);
      await settingsManager.flush();
      send({ type: "catalog", packages: await buildCatalog() });
    } else if (command.type === "set_resources") {
      applyResourceFilters(command.source, {
        extensions: command.extensions,
        skills: command.skills,
        prompts: command.prompts,
        themes: command.themes
      });
      await settingsManager.flush();
      send({ type: "catalog", packages: await buildCatalog() });
    } else if (command.type === "shutdown") {
      send({ type: "response", id: command.id, success: true });
      process.exit(0);
    }
    send({ type: "response", id: command.id, success: true });
  } catch (error) {
    send({ type: "response", id: command.id, success: false, error: safeError(error) });
  }
}

const decoder = new JsonLineDecoder();
process.stdin.on("data", (chunk: Buffer) => {
  for (const line of decoder.push(chunk)) {
    let command: ManagerCommand;
    try {
      command = JSON.parse(line) as ManagerCommand;
    } catch {
      send({ type: "manager_error", message: "The desktop bridge sent invalid JSON" });
      continue;
    }
    commandQueue = commandQueue.then(() => handle(command)).catch((error) => {
      send({ type: "manager_error", message: safeError(error) });
    });
  }
});

process.stdin.resume();
process.on("uncaughtException", (error) => {
  send({ type: "manager_error", message: safeError(error) });
  process.exitCode = 1;
});
process.on("unhandledRejection", (error) => {
  send({ type: "manager_error", message: safeError(error) });
  process.exitCode = 1;
});
