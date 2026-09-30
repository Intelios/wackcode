import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { Markdown } from "./Markdown";

vi.mock("../api", () => ({ api: { revealPath: vi.fn().mockResolvedValue(undefined) } }));

afterEach(cleanup);
beforeEach(() => vi.mocked(api.revealPath).mockClear());

describe("Markdown", () => {
  it("highlights a closed fenced block with its language", () => {
    render(<Markdown>{"Here:\n\n```ts\nconst x = 1;\n```\n"}</Markdown>);
    expect(screen.getByText("const")).toHaveClass("hljs-keyword");
    expect(document.querySelector(".hljs-number")?.textContent).toBe("1");
  });

  it("leaves inline code alone", () => {
    render(<Markdown>{"Run `npm ci` now"}</Markdown>);
    expect(screen.getByText("npm ci")).not.toHaveClass("hljs");
    expect(document.querySelector(".hljs")).toBeNull();
  });

  it("keeps the text of an unknown language", () => {
    render(<Markdown>{"```\nplainish content\n```\n"}</Markdown>);
    expect(screen.getByText("plainish content")).toBeInTheDocument();
  });

  it("highlights closed fences while streaming but keeps the open one plain", () => {
    render(<Markdown streaming>{"```ts\nconst a = 1;\n```\n\n```js\nlet b = "}</Markdown>);
    expect(screen.getByText("const")).toHaveClass("hljs-keyword");
    // The trailing fence has no closer yet: plain text, no highlighting.
    expect(screen.getByText("let b =").closest(".hljs")).toBeNull();
  });

  it("highlights everything once the stream settles", () => {
    render(<Markdown>{"```js\nlet b = 2;\n```"}</Markdown>);
    expect(screen.getByText("let")).toHaveClass("hljs-keyword");
  });

  it("opens http links through Rust instead of navigating the webview", () => {
    render(<Markdown>{"See [the docs](https://example.com/guide)"}</Markdown>);
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    screen.getByRole("link", { name: "the docs" }).dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(api.revealPath).toHaveBeenCalledWith("https://example.com/guide");
  });

  it("never hands links with another scheme to the system", () => {
    render(<Markdown>{"[mail me](mailto:x@example.com) or [rel](./other)"}</Markdown>);
    for (const name of ["mail me", "rel"]) {
      const click = new MouseEvent("click", { bubbles: true, cancelable: true });
      screen.getByRole("link", { name }).dispatchEvent(click);
      expect(click.defaultPrevented).toBe(true);
    }
    expect(api.revealPath).not.toHaveBeenCalled();
  });
});
