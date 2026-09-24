/**
 * Protocol for the package-manager process.
 *
 * Separate from `protocol.ts` on purpose: the manager is a different process with different
 * privileges. It never receives an API key, and unlike a task worker it does not set
 * PI_OFFLINE, because installing a package needs the network.
 *
 * Framing note: Pi's DefaultPackageManager spawns npm with `stdio: "inherit"`, and the override
 * for that is not part of the package's public exports. npm's own chatter therefore lands on
 * this process's stdout. Every protocol line is prefixed with FRAME so the host can tell
 * protocol from noise; anything unprefixed is npm talking and is kept only for error messages.
 */
export const FRAME = "\x1e";

export type ResourceKind = "extensions" | "skills" | "prompts" | "themes";

export const RESOURCE_KINDS: ResourceKind[] = ["extensions", "skills", "prompts", "themes"];

export interface ManagerResource {
  /** Absolute path to the resource file. */
  path: string;
  /** Path relative to the package root, which is what the user sees and what filters match. */
  name: string;
  enabled: boolean;
}

export interface ManagerPackage {
  /** The source string as installed, e.g. "npm:pi-web-access". */
  source: string;
  displayName: string;
  kind: "npm" | "git" | "local";
  version?: string;
  installedPath?: string;
  extensions: ManagerResource[];
  skills: ManagerResource[];
  prompts: ManagerResource[];
  themes: ManagerResource[];
  /** Extension load failures for this package. Never fatal. */
  errors: string[];
}

export interface ManagerInit {
  id: string;
  type: "init";
  /** Shared Pi directory: holds settings.json, npm/ and git/. */
  piDir: string;
  /** Pins npm to the bundled copy, e.g. [<node>, <resources>/npm/bin/npm-cli.js]. */
  npmCommand?: string[];
}

export type ManagerCommand =
  | ManagerInit
  | { id: string; type: "list" }
  | {
      id: string;
      type: "install";
      source: string;
      /** Settings › Skills: switch on only the package's skills; its code and the rest start off. */
      onlySkills?: boolean;
    }
  | { id: string; type: "remove"; source: string }
  | { id: string; type: "update"; source?: string }
  | {
      id: string;
      type: "set_resources";
      source: string;
      /** Omit a kind to load all of it; [] loads none. Mirrors Pi's PackageSource filters. */
      extensions?: string[];
      skills?: string[];
      prompts?: string[];
      themes?: string[];
    }
  | { id: string; type: "shutdown" };

export type ManagerOutput =
  | { type: "response"; id: string; success: true }
  | { type: "response"; id: string; success: false; error: string }
  | {
      type: "progress";
      action: "install" | "remove" | "update" | "clone" | "pull";
      phase: "start" | "progress" | "complete" | "error";
      source: string;
      message?: string;
    }
  | { type: "catalog"; packages: ManagerPackage[] }
  | { type: "manager_error"; message: string };
