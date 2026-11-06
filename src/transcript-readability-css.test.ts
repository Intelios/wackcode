/// <reference types="node" />
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

// jsdom does not resolve inherited CSS variables. Check the stylesheet's real selectors
// against both conversation kinds and the boundaries that must retain solid-theme ink.
const styles = readFileSync(resolve(__dirname, "styles.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const rules = [...styles.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, declarations]) => ({ selector: selector.trim(), declarations }));
const foregroundRules = rules.filter(({ selector, declarations }) =>
  !["from", "to"].includes(selector)
  && /\b(?:color|background(?:-image)?):[^;{}]*var\(--wc-transcript-/.test(declarations)
);
const resets = rules.filter(({ declarations }) => declarations.includes("--wc-transcript-body: var(--wc-text-body)"));

function fixture(screen = "workspace", enabled = true): HTMLDivElement {
  const shell = document.createElement("div");
  shell.className = "app-shell";
  shell.dataset.screen = screen;
  if (enabled) shell.dataset.imageReadability = "dark";
  shell.innerHTML = `
    <div class="chat-transcript"><div class="msg assistant">
      <div class="assistant-text" data-test="prose"><h2>Heading</h2>
        <div class="code-block" data-test="code"><pre>code</pre></div>
        <table><thead><tr><th data-test="header">Heading</th></tr></thead></table>
      </div>
      <div class="compaction-row" data-test="compaction"><div class="assistant-text">Summary</div></div>
      <div class="tool-row" data-test="tool">Read file</div>
    </div></div>
    <div class="chat-thread">
      <div class="chat-name" data-test="name">Agent</div>
      <div class="chat-thought-text" data-test="thought">Thinking</div>
      <div class="chat-bubble agent" data-test="bubble">Reply</div>
      <div class="chat-compaction-body"><p>Summary</p>
        <table><thead><tr><th data-test="summary-header"><a href="#" data-test="summary-link">Reference</a></th></tr></thead></table>
      </div>
    </div>
    <div class="side-panel" data-test="panel"><div class="assistant-text" data-test="child">Child reply</div></div>`;
  document.body.append(shell);
  return shell;
}

afterEach(() => {
  document.body.replaceChildren();
  delete document.documentElement.dataset.messageBubbles;
});

describe("transcript image-readability CSS boundaries", () => {
  it("adapts exposed Code and Chat text only in an enabled workspace", () => {
    expect(foregroundRules.length).toBeGreaterThan(0);
    for (const [screen, enabled, expected] of [["workspace", true, true], ["settings", true, false], ["workspace", false, false]] as const) {
      const shell = fixture(screen, enabled);
      for (const name of ["prose", "name", "thought"]) {
        const element = shell.querySelector(`[data-test="${name}"]`)!;
        expect(foregroundRules.some(({ selector }) => element.matches(selector)), `${screen}/${enabled}/${name}`).toBe(expected);
      }
      expect(foregroundRules.some(({ selector }) => shell.querySelector('[data-test="child"]')!.matches(selector))).toBe(false);
      shell.remove();
    }
  });

  it("restores the solid theme at bubbles, code, cards, table headers and side panels", () => {
    const shell = fixture();
    for (const name of ["code", "header", "compaction", "tool", "bubble", "panel"]) {
      const element = shell.querySelector(`[data-test="${name}"]`)!;
      const rule = resets.find(({ selector }) => element.matches(selector));
      expect(rule, name).toBeDefined();
      for (const token of ["text", "body", "soft", "dim", "faint", "accent"]) {
        expect(rule?.declarations, `${name}/${token}`).toContain(`--wc-transcript-${token}: var(`);
      }
    }
  });

  it("restores solid ink for optional Code assistant bubbles", () => {
    const shell = fixture();
    const prose = shell.querySelector('[data-test="prose"]')!;
    expect(resets.some(({ selector }) => prose.matches(selector))).toBe(false);
    document.documentElement.dataset.messageBubbles = "on";
    expect(resets.some(({ selector }) => prose.matches(selector))).toBe(true);
  });

  it("adapts Chat compaction links without treating their transparent table headings as filled", () => {
    const shell = fixture();
    const header = shell.querySelector('[data-test="summary-header"]')!;
    const link = shell.querySelector('[data-test="summary-link"]')!;
    expect(resets.some(({ selector }) => header.matches(selector))).toBe(false);
    expect(foregroundRules.some(({ selector, declarations }) => link.matches(selector) && declarations.includes("var(--wc-transcript-accent"))).toBe(true);
  });
});
