import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectRecord } from "../types";
import { ProjectPicker } from "./ProjectPicker";

afterEach(cleanup);

const project = (id: string, name: string, path: string): ProjectRecord => ({ id, name, path, gitRoot: null, gitHasHead: false, runCommand: null, branch: null, createdAt: "now" });
const projects = [
  project("p1", "TokenTrail", "/Users/jack/code/tokentrail"),
  project("p2", "StickCity", "/Users/jack/code/stickcity"),
  project("p3", "wackcode", "/Users/jack/Documents/GitHub/wackcode")
];

function setup(overrides: Partial<React.ComponentProps<typeof ProjectPicker>> = {}) {
  const props = {
    projects,
    projectId: "p1" as string | null,
    pinned: new Set<string>(),
    onSelect: vi.fn(),
    onSetPinned: vi.fn(),
    onAddProject: vi.fn(),
    ...overrides
  };
  render(<ProjectPicker {...props} />);
  return props;
}

const trigger = () => screen.getByRole("button", { name: /Switch project/ });
const open = () => { fireEvent.click(trigger()); return screen.getByRole("dialog", { name: "Choose project" }); };

describe("ProjectPicker", () => {
  it("names the current project on the trigger and opens a dialog", () => {
    setup();
    expect(trigger()).toHaveAccessibleName("Project: TokenTrail. Switch project");
    expect(trigger()).toHaveAttribute("aria-expanded", "false");
    const dialog = open();
    expect(trigger()).toHaveAttribute("aria-expanded", "true");
    expect(within(dialog).getByRole("textbox", { name: "Filter projects" })).toBeInTheDocument();
  });

  it("shows No project on the trigger when none is selected", () => {
    setup({ projectId: null });
    expect(trigger()).toHaveAccessibleName("Project: No project. Switch project");
  });

  it("shows each project with its shortened path and marks the current one", () => {
    setup();
    const dialog = open();
    expect(within(dialog).getByText("~/Documents/GitHub/wackcode")).toBeInTheDocument();
    expect(within(dialog).getByText("~/code/tokentrail")).toBeInTheDocument();
    const current = within(dialog).getAllByRole("button").filter((button) => button.getAttribute("aria-current") === "true");
    expect(current).toHaveLength(1);
    expect(current[0]).toHaveTextContent("TokenTrail");
  });

  it("lists pinned projects first under a Pinned heading", () => {
    setup({ pinned: new Set(["p3"]) });
    const dialog = open();
    expect(within(dialog).getByText("Pinned")).toBeInTheDocument();
    expect(within(dialog).getByText("Projects")).toBeInTheDocument();
    const names = within(dialog).getAllByRole("button").map((button) => button.textContent ?? "").filter((text) => /wackcode|TokenTrail|StickCity/.test(text));
    expect(names[0]).toContain("wackcode");
    expect(names[1]).toContain("TokenTrail");
  });

  it("filters by name and by path, and says when nothing matches", () => {
    setup();
    const dialog = open();
    const filter = within(dialog).getByRole("textbox", { name: "Filter projects" });
    fireEvent.change(filter, { target: { value: "stick" } });
    expect(within(dialog).getByText("StickCity")).toBeInTheDocument();
    expect(within(dialog).queryByText("TokenTrail")).toBeNull();
    fireEvent.change(filter, { target: { value: "documents/github" } });
    expect(within(dialog).getByText("wackcode")).toBeInTheDocument();
    expect(within(dialog).queryByText("StickCity")).toBeNull();
    fireEvent.change(filter, { target: { value: "zzz" } });
    expect(within(dialog).getByText("No matching projects")).toBeInTheDocument();
  });

  it("hides No project while filtering unless the query matches it", () => {
    setup();
    const dialog = open();
    const filter = within(dialog).getByRole("textbox", { name: "Filter projects" });
    expect(within(dialog).getByText("Chat without a folder")).toBeInTheDocument();
    fireEvent.change(filter, { target: { value: "stick" } });
    expect(within(dialog).queryByText("Chat without a folder")).toBeNull();
    fireEvent.change(filter, { target: { value: "no proj" } });
    expect(within(dialog).getByText("Chat without a folder")).toBeInTheDocument();
  });

  it("selects a project on click and closes", () => {
    const props = setup();
    const dialog = open();
    fireEvent.click(within(dialog).getByText("StickCity"));
    expect(props.onSelect).toHaveBeenCalledWith("p2");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("selects No project", () => {
    const props = setup();
    fireEvent.click(within(open()).getByText("No project"));
    expect(props.onSelect).toHaveBeenCalledWith(null);
  });

  it("moves with the arrow keys and chooses with Enter", () => {
    const props = setup();
    const dialog = open();
    const filter = within(dialog).getByRole("textbox", { name: "Filter projects" });
    // The current project (TokenTrail, first) starts active.
    fireEvent.keyDown(filter, { key: "ArrowDown" });
    fireEvent.keyDown(filter, { key: "Enter" });
    expect(props.onSelect).toHaveBeenCalledWith("p2");
  });

  it("reaches No project with End", () => {
    const props = setup();
    const filter = within(open()).getByRole("textbox", { name: "Filter projects" });
    fireEvent.keyDown(filter, { key: "End" });
    fireEvent.keyDown(filter, { key: "Enter" });
    expect(props.onSelect).toHaveBeenCalledWith(null);
  });

  it("closes on Escape and returns focus to the trigger", () => {
    setup();
    open();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(trigger()).toHaveFocus();
  });

  it("pins and unpins through labelled buttons", () => {
    const props = setup({ pinned: new Set(["p3"]) });
    const dialog = open();
    fireEvent.click(within(dialog).getByRole("button", { name: "Pin StickCity" }));
    expect(props.onSetPinned).toHaveBeenCalledWith("p2", true);
    const unpin = within(dialog).getByRole("button", { name: "Unpin wackcode" });
    expect(unpin).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(unpin);
    expect(props.onSetPinned).toHaveBeenCalledWith("p3", false);
    expect(props.onSelect).not.toHaveBeenCalled();
  });

  it("adds a project from the footer", () => {
    const props = setup();
    fireEvent.click(within(open()).getByRole("button", { name: /Add project/ }));
    expect(props.onAddProject).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("explains an empty list and still offers Add project", () => {
    setup({ projects: [], projectId: null });
    const dialog = open();
    expect(within(dialog).getByText("No projects yet")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /Add project/ })).toBeInTheDocument();
  });
});
