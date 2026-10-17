import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const source = readFileSync("src-tauri/src/browser/dom.js", "utf8");
let inspect: (point: { x: number; y: number } | null) => { targets: { selector: string; label: string; dom: string; truncated: boolean; path: { kind: string; selector: string }[] }[] };

beforeEach(() => {
  vi.stubGlobal("CSS", { escape: (value: string) => value.replace(/[^a-zA-Z0-9_-]/g, "\\$&") });
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 200, 100));
  inspect = window.eval(source);
});
afterEach(() => { document.body.innerHTML = ""; vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("isolated browser evidence collector", () => {
  it("captures inert structure while excluding executable and sensitive content", () => {
    document.body.innerHTML = '<main id="main"><script>secretCode()</script><style>.secret{}</style><input type="password" value="secret-password"><input type="hidden" value="hidden-secret"><textarea>private-text</textarea><div contenteditable="true">private-edit</div><button onclick="secretHandler()" data-token="private-token">Save</button></main>';
    const dom = inspect(null).targets.map((target) => target.dom).join("\n");
    for (const secret of ["secretCode", "secret-password", "hidden-secret", "private-text", "secretHandler", "private-token", ".secret"]) expect(dom).not.toContain(secret);
    expect(dom).toContain("Save");
  });
  it("records open-shadow locator boundaries and unique selector fallbacks", () => {
    document.body.innerHTML = '<div id="host"></div><button id="duplicate">First</button><button id="duplicate">Second</button>';
    const root = document.getElementById("host")!.attachShadow({ mode: "open" });
    root.innerHTML = '<button data-testid="save">Save</button>';
    const targets = inspect(null).targets;
    const shadow = targets.find((target) => target.selector.includes("data-testid"));
    expect(shadow?.path).toEqual([{ kind: "shadow", selector: '[id="host"]' }]);
    expect(targets.filter((target) => target.selector === '[id="duplicate"]')).toHaveLength(0);
  });
  it("keeps cross-origin/inaccessible frames as containing visual targets", () => {
    document.body.innerHTML = '<iframe id="frame"></iframe>';
    const frame = document.querySelector("iframe")!;
    Object.defineProperty(frame, "contentDocument", { get() { throw new Error("cross origin"); } });
    expect(inspect(null).targets.some((target) => target.selector === '[id="frame"]')).toBe(true);
  });
  it("does not run custom-element constructors while inspecting their DOM", () => {
    let constructed = 0;
    customElements.define("wc-capture-probe", class extends HTMLElement { constructor() { super(); constructed += 1; } });
    document.body.innerHTML = '<main><wc-capture-probe><button>Save</button></wc-capture-probe></main>';
    const before = constructed;
    expect(inspect(null).targets.length).toBeGreaterThan(0);
    expect(constructed).toBe(before);
  });
  it("clips Unicode safely and reports text shortened before the total DOM budget", () => {
    document.body.innerHTML = `<button id="save" aria-label="${"a".repeat(89)}🦆">${"a".repeat(399)}🦆 text</button>`;
    const target = inspect(null).targets.find((item) => item.selector === '[id="save"]')!;
    expect(target.truncated).toBe(true);
    expect(target.label).toBe("a".repeat(89));
    expect(target.dom).not.toMatch(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])/);
  });
});
