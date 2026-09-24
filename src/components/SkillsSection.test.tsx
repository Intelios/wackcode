import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PackageRecord, SkillEntry, SkillFolderView, SkillsOverview } from "../types";
import { SkillsSection, skillDraftIssue } from "./SkillsSection";

afterEach(cleanup);

function skill(name: string, patch: Partial<SkillEntry> = {}): SkillEntry {
  return {
    name, description: `${name} description`, filePath: `/Users/me/.agents/skills/${name}/SKILL.md`, baseDir: `/Users/me/.agents/skills/${name}`,
    manual: false, enabled: true, editable: true, ...patch
  };
}

function folder(id: string, label: string, patch: Partial<SkillFolderView> = {}): SkillFolderView {
  return {
    id, label, path: `/Users/me/.${id}/skills`, displayPath: `~/.${id}/skills`, kind: "tool", exists: true, enabled: false,
    skills: [], diagnostics: [], ...patch
  };
}

const theirs = skill("theirs", {
  filePath: "/Users/me/.claude/skills/theirs/SKILL.md", baseDir: "/Users/me/.claude/skills/theirs", editable: false
});

const overview: SkillsOverview = {
  libraryPath: "/Users/me/.agents/skills",
  folders: [
    folder("library", "Your skills", {
      kind: "library", enabled: true, path: "/Users/me/.agents/skills", displayPath: "~/.agents/skills",
      skills: [skill("pdf-tools"), skill("notes", { manual: true })]
    }),
    folder("claude", "Claude Code", { skills: [theirs, skill("pdf-tools", { filePath: "/Users/me/.claude/skills/pdf-tools/SKILL.md", editable: false, shadowedBy: "Your skills" })] }),
    folder("codex", "Codex", { exists: false })
  ],
  packages: []
};

const pkg: PackageRecord = {
  source: "npm:skillful", displayName: "skillful", kind: "npm", extensions: [], prompts: [], themes: [], errors: [],
  skills: [{ path: "/pkg/skills/a", name: "skills/a", enabled: true }, { path: "/pkg/skills/b", name: "skills/b", enabled: true }],
  trustedAt: "t", installedAt: "t"
};

function renderSection(start: SkillsOverview = overview, packages: PackageRecord[] = []) {
  const change = { overview: start };
  const actions = {
    onList: vi.fn().mockResolvedValue(start),
    onRead: vi.fn().mockResolvedValue({ body: "# Steps\n1. Do it.", files: ["scripts/run.sh"], filesTruncated: false }),
    onSave: vi.fn().mockResolvedValue(change),
    onDelete: vi.fn().mockResolvedValue(change),
    onSetEnabled: vi.fn().mockResolvedValue(change),
    onSetFolderEnabled: vi.fn().mockResolvedValue(change),
    onAddFolder: vi.fn().mockResolvedValue(null),
    onRemoveFolder: vi.fn().mockResolvedValue(change),
    onImport: vi.fn().mockResolvedValue({ overview: start, note: "Imported 1 of 2 skills. A skill named pdf-tools already exists in Your skills." }),
    onCopyToLibrary: vi.fn().mockResolvedValue(change),
    onReveal: vi.fn().mockResolvedValue(undefined),
    onSearch: vi.fn().mockResolvedValue({
      results: [{ name: "skill-pack", version: "1.0.0", description: "Handy skills", publisher: "me", npmUrl: "", publishedAt: "", declares: [], types: ["extension", "skill"], downloads: 1234 }],
      source: "pidev",
      hasMore: false
    }),
    onDetails: vi.fn().mockResolvedValue({ name: "skill-pack", version: "1.0.0", description: "", publisher: "", npmUrl: "", publishedAt: "", declares: ["extensions", "skills"] }),
    onInstallSkills: vi.fn().mockResolvedValue(undefined),
    onSetPackageSkills: vi.fn().mockResolvedValue(undefined),
    onOpenPackages: vi.fn()
  };
  render(<SkillsSection packages={packages} {...actions} />);
  return actions;
}

describe("skillDraftIssue", () => {
  it("follows the Agent Skills rules and refuses a name already in Your skills", () => {
    const taken = new Set(["pdf-tools"]);
    const draft = { name: "review", description: "Reviews code.", manual: false, body: "" };
    expect(skillDraftIssue(draft, taken)).toBeUndefined();
    expect(skillDraftIssue({ ...draft, name: "Review" }, taken)).toMatch(/lowercase/);
    expect(skillDraftIssue({ ...draft, name: "re--view" }, taken)).toMatch(/lowercase/);
    expect(skillDraftIssue({ ...draft, name: "pdf-tools" }, taken)).toMatch(/already exists/);
    // Renaming a skill to its own name is fine.
    expect(skillDraftIssue({ ...draft, name: "pdf-tools" }, taken, "pdf-tools")).toBeUndefined();
    expect(skillDraftIssue({ ...draft, description: " " }, taken)).toMatch(/Describe/);
    expect(skillDraftIssue({ ...draft, description: "d".repeat(1_025) }, taken)).toMatch(/1024/);
  });
});

describe("SkillsSection", () => {
  it("lists your skills, and other tools' folders stay off until switched on", async () => {
    const actions = renderSection();
    const yours = await screen.findByRole("region", { name: "Your skills" });
    expect(within(yours).getByText("pdf-tools")).toBeInTheDocument();
    expect(within(yours).getByText("/skill only")).toBeInTheDocument();

    const folders = screen.getByRole("region", { name: "Other tools' folders" });
    const claude = within(folders).getByRole("switch", { name: "Load skills from Claude Code" });
    expect(claude).toHaveAttribute("aria-checked", "false");
    expect(within(folders).getByRole("switch", { name: "Load skills from Codex" })).toBeDisabled();
    fireEvent.click(claude);
    await waitFor(() => expect(actions.onSetFolderEnabled).toHaveBeenCalledWith("claude", true));

    fireEvent.click(within(yours).getByRole("switch", { name: "Use pdf-tools" }));
    await waitFor(() => expect(actions.onSetEnabled).toHaveBeenCalledWith("/Users/me/.agents/skills/pdf-tools/SKILL.md", false));
  });

  it("shows which skill loses a name clash, and offers a copy instead of an edit outside Your skills", async () => {
    const actions = renderSection();
    const folders = await screen.findByRole("region", { name: "Other tools' folders" });
    fireEvent.click(within(folders).getByRole("button", { name: /Claude Code/ }));
    expect(within(folders).getByText("Not loaded")).toBeInTheDocument();
    expect(within(folders).getByRole("switch", { name: "Use theirs" })).toBeDisabled();

    fireEvent.click(within(folders).getByRole("button", { name: /theirs/ }));
    expect(await screen.findByText(/1\. Do it\./)).toBeInTheDocument();
    expect(screen.getByText("scripts/run.sh")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Edit/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Delete" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Copy to Your skills/ }));
    await waitFor(() => expect(actions.onCopyToLibrary).toHaveBeenCalledWith(theirs.filePath));
    expect(await screen.findByText(/Your copy loads instead of the original/)).toBeInTheDocument();
  });

  it("creates a skill only once its name and description are valid", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByRole("button", { name: /New skill/ }));
    const create = screen.getByRole("button", { name: "Create skill" });
    expect(create).toBeDisabled();

    fireEvent.change(screen.getByPlaceholderText("pdf-tools"), { target: { value: "PDF Tools" } });
    expect(screen.getByPlaceholderText("pdf-tools")).toHaveValue("pdf-tools");
    fireEvent.change(screen.getByRole("textbox", { name: /Description/ }), { target: { value: "Reads PDFs." } });
    expect(screen.getByText(/already exists/)).toBeInTheDocument();
    expect(create).toBeDisabled();

    fireEvent.change(screen.getByPlaceholderText("pdf-tools"), { target: { value: "pdf-reader" } });
    fireEvent.change(screen.getByRole("textbox", { name: /Instructions/ }), { target: { value: "Read it." } });
    expect(create).toBeEnabled();
    fireEvent.click(create);
    await waitFor(() => expect(actions.onSave).toHaveBeenCalledWith({ path: undefined, name: "pdf-reader", description: "Reads PDFs.", manual: false, body: "Read it." }));
    expect(await screen.findByRole("region", { name: "Your skills" })).toBeInTheDocument();
  });

  it("edits a skill of yours in place, starting from its saved instructions", async () => {
    const actions = renderSection();
    const yours = await screen.findByRole("region", { name: "Your skills" });
    fireEvent.click(within(yours).getByRole("button", { name: /notes/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Edit/ }));
    expect(screen.getByRole("textbox", { name: /Instructions/ })).toHaveValue("# Steps\n1. Do it.");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(actions.onSave).toHaveBeenCalledWith({
      path: "/Users/me/.agents/skills/notes/SKILL.md", name: "notes", description: "notes description", manual: true, body: "# Steps\n1. Do it."
    }));
  });

  it("moves a deleted skill to the Trash after confirming", async () => {
    const actions = renderSection();
    const yours = await screen.findByRole("region", { name: "Your skills" });
    fireEvent.click(within(yours).getByRole("button", { name: /pdf-tools/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("button", { name: "Move to Trash" }));
    await waitFor(() => expect(actions.onDelete).toHaveBeenCalledWith("/Users/me/.agents/skills/pdf-tools/SKILL.md"));
  });

  it("reports what an import skipped", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByRole("button", { name: "Import skills" }));
    fireEvent.click(screen.getByText("A folder of skills…"));
    await waitFor(() => expect(actions.onImport).toHaveBeenCalledWith("folder"));
    expect(await screen.findByText(/Imported 1 of 2 skills/)).toBeInTheDocument();
  });

  it("switches a package's skill through the package's own resources", async () => {
    const withPackage: SkillsOverview = {
      ...overview,
      packages: [{
        source: "npm:skillful", label: "skillful", diagnostics: [],
        skills: [
          skill("a", { editable: false, resourceName: "skills/a", filePath: "/pkg/skills/a/SKILL.md" }),
          skill("b", { editable: false, resourceName: "skills/b", filePath: "/pkg/skills/b/SKILL.md" })
        ]
      }]
    };
    const actions = renderSection(withPackage, [pkg]);
    const fromPackages = await screen.findByRole("region", { name: "Skills from packages" });
    fireEvent.click(within(fromPackages).getByRole("button", { name: /skillful/ }));
    fireEvent.click(within(fromPackages).getByRole("switch", { name: "Use a" }));
    await waitFor(() => expect(actions.onSetPackageSkills).toHaveBeenCalledWith("npm:skillful", ["skills/b"]));
    await waitFor(() => expect(actions.onList).toHaveBeenCalledTimes(2));
  });

  it("browses pi.dev's skill packages and installs one with only its skills switched on", async () => {
    const actions = renderSection();
    fireEvent.click(await screen.findByRole("tab", { name: "Browse" }));
    await waitFor(() => expect(actions.onSearch).toHaveBeenCalledWith("", "downloads", 1));
    expect(await screen.findByText("skill-pack")).toBeInTheDocument();
    expect(screen.getByText(/1,234\/mo/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Install this package's skills?")).toBeInTheDocument();
    expect(await within(dialog).findByText(/also contains extensions \(code\), which stay off/)).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("checkbox"));
    fireEvent.click(within(dialog).getByRole("button", { name: "Trust and install skills" }));
    await waitFor(() => expect(actions.onInstallSkills).toHaveBeenCalledWith("npm:skill-pack"));
    expect(await screen.findByText(/Installed skill-pack/)).toBeInTheDocument();
  });

  it("says so when pi.dev can't be read and npm keywords stand in", async () => {
    const actions = renderSection();
    actions.onSearch.mockResolvedValue({ results: [], source: "npm", hasMore: false });
    fireEvent.click(await screen.findByRole("tab", { name: "Browse" }));
    expect(await screen.findByText(/pi.dev couldn't be read/)).toBeInTheDocument();
  });
});
