import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GitEditorButton } from "./GitEditorButton";

afterEach(cleanup);

describe("GitEditorButton", () => {
  it("opens the chosen editor from the main button", () => {
    const onOpen = vi.fn();
    render(<GitEditorButton editors={["Visual Studio Code", "Zed"]} editor="Zed" onOpen={onOpen} />);
    fireEvent.click(screen.getByRole("button", { name: "Open in Zed" }));
    expect(onOpen).toHaveBeenCalledWith("Zed");
  });

  it("picks another editor from the chevron menu, ticking the chosen one", () => {
    const onOpen = vi.fn();
    render(<GitEditorButton editors={["Visual Studio Code", "Zed"]} editor="Visual Studio Code" onOpen={onOpen} />);
    fireEvent.click(screen.getByRole("button", { name: "Choose an editor" }));
    expect(screen.getByRole("menuitem", { name: /^Visual Studio Code/ }).textContent).toContain("✓");
    fireEvent.click(screen.getByRole("menuitem", { name: /^Zed/ }));
    expect(onOpen).toHaveBeenCalledWith("Zed");
  });

  it("is inert and says why when no editor is installed", () => {
    const onOpen = vi.fn();
    render(<GitEditorButton editors={[]} onOpen={onOpen} />);
    expect(screen.getByRole("button", { name: "Open in editor" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Choose an editor" })).toBeDisabled();
    expect(screen.getByText("No editor found")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open in editor" }));
    expect(onOpen).not.toHaveBeenCalled();
  });
});
