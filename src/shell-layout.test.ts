import { describe, expect, it } from "vitest";
import { CHAT_MIN, PANEL_MIN, SIDEBAR_W, balancedPanelWidth, chatSize, clampPanelDrag, solveShell } from "./shell-layout";

describe("shell layout", () => {
  it("gives the panel its preferred width when the chat keeps its minimum", () => {
    expect(solveShell({ windowWidth: 1440, sidebarOpen: true, panelOpen: true, panelPref: 600 })).toEqual({
      sidebarShown: true, sidebarAutoHidden: false, panelWidth: 600, chatWidth: 600, chatSize: "compact"
    });
  });

  it("hides the sidebar before clamping a wide panel, and brings it back with room", () => {
    const tight = solveShell({ windowWidth: 1280, sidebarOpen: true, panelOpen: true, panelPref: 800 });
    expect(tight).toMatchObject({ sidebarShown: false, sidebarAutoHidden: true, panelWidth: 800, chatWidth: 480 });
    const roomy = solveShell({ windowWidth: 1600, sidebarOpen: true, panelOpen: true, panelPref: 800 });
    expect(roomy).toMatchObject({ sidebarShown: true, sidebarAutoHidden: false, panelWidth: 800 });
  });

  it("clamps the panel so the chat never drops below its minimum, without touching the preference", () => {
    const layout = solveShell({ windowWidth: 980, sidebarOpen: true, panelOpen: true, panelPref: 1200 });
    expect(layout.sidebarShown).toBe(false);
    expect(layout.panelWidth).toBe(980 - CHAT_MIN);
    expect(layout.chatWidth).toBe(CHAT_MIN);
  });

  it("keeps a closed sidebar closed and never auto-hides it", () => {
    const layout = solveShell({ windowWidth: 1440, sidebarOpen: false, panelOpen: true, panelPref: 500 });
    expect(layout).toMatchObject({ sidebarShown: false, sidebarAutoHidden: false, panelWidth: 500, chatWidth: 940 });
  });

  it("leaves everything to the chat and sidebar when the panel is closed", () => {
    expect(solveShell({ windowWidth: 1000, sidebarOpen: true, panelOpen: false, panelPref: 900 })).toMatchObject({
      sidebarShown: true, panelWidth: 0, chatWidth: 1000 - SIDEBAR_W
    });
  });

  it("never makes the panel narrower than its minimum", () => {
    expect(solveShell({ windowWidth: 1440, sidebarOpen: true, panelOpen: true, panelPref: 100 }).panelWidth).toBe(PANEL_MIN);
    expect(clampPanelDrag(50, 1440)).toBe(PANEL_MIN);
  });

  it("lets a drag reach the room left once the sidebar steps aside", () => {
    expect(clampPanelDrag(5000, 1440)).toBe(1440 - CHAT_MIN);
    expect(clampPanelDrag(700.4, 1440)).toBe(700);
  });

  it("balances the panel against the chat", () => {
    expect(balancedPanelWidth(1440, true)).toBe(600);
    expect(balancedPanelWidth(1440, false)).toBe(720);
  });

  it("classes the chat column's width", () => {
    expect(chatSize(400)).toBe("narrow");
    expect(chatSize(520)).toBe("compact");
    expect(chatSize(760)).toBe("wide");
  });
});
