import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { gzipSync } from "node:zlib";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { isBlockedAddress, isBlockedHostname } from "./builtin/web-fetch/address.js";
import { convertBody } from "./builtin/web-fetch/convert.js";
import { WebFetchError, fetchUrl, parseUrl } from "./builtin/web-fetch/fetch.js";
import { createWebFetchExtension, formatPage, normalizeWebFetchParams } from "./builtin/web-fetch/index.js";

describe("web_fetch address policy", () => {
  it.each([
    "127.0.0.1", "127.8.9.10", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254",
    "100.64.0.1", "0.0.0.0", "224.0.0.1", "255.255.255.255", "198.18.0.1",
    "::", "::1", "fe80::1", "fe80::1%en0", "fc00::1", "fd12:3456::1", "ff02::1",
    "::ffff:127.0.0.1", "::ffff:7f00:1", "::ffff:10.0.0.1", "::127.0.0.1", "64:ff9b::a00:1", "64:ff9b:1::1",
    "2002:c0a8:101::1", "2001:db8::1", "2001::1", "not an ip", "",
  ])("blocks %s", (address) => {
    expect(isBlockedAddress(address)).toBe(true);
  });

  it.each(["93.184.215.14", "1.1.1.1", "172.32.0.1", "8.8.8.8", "2606:4700::1111", "::ffff:8.8.8.8", "64:ff9b::808:808", "2002:808:808::1", "[2606:4700::1111]"])(
    "allows %s",
    (address) => {
      expect(isBlockedAddress(address)).toBe(false);
    },
  );

  it("refuses local host names before any lookup", () => {
    for (const name of ["localhost", "LOCALHOST.", "app.localhost", "printer.local", "db.internal", "nas.home.arpa"]) {
      expect(isBlockedHostname(name)).toBe(true);
    }
    expect(isBlockedHostname("example.com")).toBe(false);
    expect(isBlockedHostname("localhost.example.com")).toBe(false);
  });

  it("accepts only plain http and https URLs", () => {
    expect(parseUrl("https://example.com/a?b").href).toBe("https://example.com/a?b");
    expect(() => parseUrl("file:///etc/passwd")).toThrow(/Only http and https/);
    expect(() => parseUrl("ftp://example.com")).toThrow(/Only http and https/);
    expect(() => parseUrl("https://user:pass@example.com")).toThrow(/credentials/);
    expect(() => parseUrl("not a url")).toThrow(/Not a valid URL/);
    expect(() => parseUrl("http://localhost:3000")).toThrow(/private network/);
  });
});

describe("web_fetch requests", () => {
  let server: Server;
  let base: string;
  // Tests run against loopback, which production refuses; only 127.0.0.1 is let through here.
  const onlyLoopback = { isAllowedAddress: (address: string) => address === "127.0.0.1" || address === "::ffff:127.0.0.1" };

  beforeAll(async () => {
    server = createServer((request, response) => {
      const path = request.url ?? "/";
      if (path === "/page") {
        response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
        response.end("<html><head><title>Hi</title></head><body><p>Hello</p></body></html>");
      } else if (path === "/json") {
        response.writeHead(200, { "content-type": "application/json" });
        response.end('{"ok":true}');
      } else if (path === "/gzip") {
        response.writeHead(200, { "content-type": "text/plain", "content-encoding": "gzip" });
        response.end(gzipSync("compressed text"));
      } else if (path === "/redirect") {
        response.writeHead(302, { location: "/json" });
        response.end();
      } else if (path === "/redirect-private") {
        response.writeHead(301, { location: "http://10.0.0.1/admin" });
        response.end();
      } else if (path === "/redirect-file") {
        response.writeHead(301, { location: "file:///etc/passwd" });
        response.end();
      } else if (path.startsWith("/loop")) {
        response.writeHead(302, { location: `/loop${path.length}` });
        response.end();
      } else if (path === "/big") {
        response.writeHead(200, { "content-type": "text/plain" });
        response.end("x".repeat(10_000));
      } else if (path === "/slow") {
        response.writeHead(200, { "content-type": "text/plain" });
        response.write("start");
        // Never ends; the client's timeout must.
      } else if (path === "/image") {
        response.writeHead(200, { "content-type": "image/png" });
        response.end(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0, 0, 0]));
      } else {
        response.writeHead(404, { "content-type": "text/plain" });
        response.end("nope");
      }
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  });

  afterAll(async () => {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  it("refuses loopback under the production policy", async () => {
    await expect(fetchUrl(`${base}/page`)).rejects.toThrow(/local or private network address/);
  });

  it("reads a page, decodes gzip, and follows a redirect", async () => {
    const page = await fetchUrl(`${base}/page`, onlyLoopback);
    expect(page.status).toBe(200);
    expect(page.contentType).toContain("text/html");
    expect(page.body.toString()).toContain("<p>Hello</p>");

    expect((await fetchUrl(`${base}/gzip`, onlyLoopback)).body.toString()).toBe("compressed text");

    const redirected = await fetchUrl(`${base}/redirect`, onlyLoopback);
    expect(redirected.finalUrl).toBe(`${base}/json`);
    expect(redirected.body.toString()).toBe('{"ok":true}');
  });

  it("checks every redirect hop", async () => {
    await expect(fetchUrl(`${base}/redirect-private`, onlyLoopback)).rejects.toThrow(/10\.0\.0\.1 is a local or private/);
    await expect(fetchUrl(`${base}/redirect-file`, onlyLoopback)).rejects.toThrow(/Only http and https/);
    await expect(fetchUrl(`${base}/loop`, onlyLoopback)).rejects.toThrow(/Stopped after 5 redirects/);
  });

  it("caps the body, times out, honours abort, and reports HTTP errors", async () => {
    const big = await fetchUrl(`${base}/big`, { ...onlyLoopback, maxBytes: 1_000 });
    expect(big.body.length).toBe(1_000);
    expect(big.bodyTruncated).toBe(true);

    await expect(fetchUrl(`${base}/slow`, { ...onlyLoopback, timeoutMs: 200 })).rejects.toThrow(/Timed out/);

    const controller = new AbortController();
    const pending = fetchUrl(`${base}/slow`, { ...onlyLoopback, signal: controller.signal });
    setTimeout(() => controller.abort(), 50);
    await expect(pending).rejects.toThrow(/stopped/);

    await expect(fetchUrl(`${base}/missing`, onlyLoopback)).rejects.toThrow(/HTTP 404/);
  });

  it("runs as a tool: converts pages and refuses binaries and bad input", async () => {
    const tool = registerTool(createWebFetchExtension(onlyLoopback));
    const result = await tool.execute("call-1", { url: `${base}/page` }, undefined);
    expect(result.content[0].text).toContain("Title: Hi");
    expect(result.content[0].text).toContain("Hello");
    expect(result.details).toMatchObject({ finalUrl: `${base}/page`, status: 200, truncated: false });

    await expect(tool.execute("call-2", { url: `${base}/image` }, undefined)).rejects.toThrow(/image\/png, which web_fetch can't read/);
    await expect(tool.execute("call-3", { url: "" }, undefined)).rejects.toThrow(/non-empty/);
    await expect(tool.execute("call-4", { url: `${base}/page`, offset: -1 }, undefined)).rejects.toThrow(/offset/);
  });
});

describe("web_fetch conversion", () => {
  const article = `<!doctype html><html><head><title>Guide</title><script>alert(1)</script></head><body>
    <nav><a href="/home">Home</a></nav>
    <article><h1>Install</h1>${"<p>Run the installer and follow the prompts to finish setting things up properly. </p>".repeat(8)}
    <p>See <a href="/docs/more">the docs</a> and <a href="javascript:evil()">this</a>.</p>
    <img src="data:image/png;base64,AAAA" alt="inline"><img src="/logo.png" alt="Logo">
    <pre><code>pnpm install</code></pre></article>
    <footer>Copyright</footer></body></html>`;

  it("extracts the article as Markdown with absolute links", () => {
    const page = convertBody(Buffer.from(article), "text/html; charset=utf-8", "https://example.com/guide/");
    expect(page.format).toBe("html");
    expect(page.title).toBe("Guide");
    expect(page.text).toContain("Install");
    expect(page.text).toContain("[the docs](https://example.com/docs/more)");
    expect(page.text).toContain("![Logo](https://example.com/logo.png)");
    expect(page.text).toContain("pnpm install");
    expect(page.text).not.toContain("alert(1)");
    expect(page.text).not.toContain("base64");
    expect(page.text).not.toContain("javascript:");
    expect(page.text).not.toContain("Copyright");
  });

  it("falls back to the whole body for pages Readability rejects", () => {
    const page = convertBody(Buffer.from("<html><body><nav>Menu</nav><p>Short note.</p></body></html>"), "text/html", "https://example.com/");
    expect(page.text).toBe("Short note.");
  });

  it("passes text through, honours charsets, sniffs untyped bodies, and refuses binaries", () => {
    expect(convertBody(Buffer.from('{"a":1}'), "application/json", "https://x.test/")).toEqual({ text: '{"a":1}', format: "text" });
    expect(convertBody(Buffer.from([0x63, 0x61, 0x66, 0xe9]), "text/plain; charset=iso-8859-1", "https://x.test/").text).toBe("café");
    expect(convertBody(Buffer.from("<!DOCTYPE html><html><body><p>Sniffed</p></body></html>"), "", "https://x.test/").text).toBe("Sniffed");
    expect(() => convertBody(Buffer.from([1, 0, 2]), "", "https://x.test/")).toThrow(WebFetchError);
    expect(() => convertBody(Buffer.from("PDF"), "application/pdf", "https://x.test/")).toThrow(/application\/pdf/);
  });
});

describe("web_fetch output", () => {
  const entry = (text: string) => ({
    finalUrl: "https://example.com/",
    status: 200,
    contentType: "text/plain",
    bodyTruncated: false,
    page: { text, format: "text" as const },
  });

  it("pages through long content with offsets that line up", () => {
    // Pi's limit is 2,000 lines, so this is two pages.
    const text = Array.from({ length: 3_000 }, (_, index) => `line ${index}`).join("\n");
    const first = formatPage(entry(text), 0);
    expect(first.truncated).toBe(true);
    const next = Number(/Use offset=(\d+) to continue/.exec(first.text)?.[1]);
    expect(text.slice(0, next)).toBe(`${first.text.split("\n\n")[1]}\n`);

    const second = formatPage(entry(text), next);
    expect(second.text.split("\n\n")[1].startsWith("line 2000\n")).toBe(true);
    expect(second.text).toContain("this is the end of the page");
    expect(second.truncated).toBe(false);

    expect(formatPage(entry(text), text.length + 5).text).toContain("past the end");
  });

  it("hard-cuts a single enormous line instead of returning nothing", () => {
    const result = formatPage(entry("y".repeat(200_000)), 0);
    expect(result.truncated).toBe(true);
    expect(result.text).toMatch(/y{1000}/);
  });

  it("validates parameters", () => {
    expect(normalizeWebFetchParams({ url: " https://a.test " })).toEqual({ ok: true, url: "https://a.test", offset: 0 });
    expect(normalizeWebFetchParams({ url: "https://a.test", offset: 1.5 }).ok).toBe(false);
    expect(normalizeWebFetchParams(null).ok).toBe(false);
  });
});

interface CapturedTool {
  execute(id: string, params: unknown, signal: AbortSignal | undefined): Promise<{ content: Array<{ text: string }>; details: unknown }>;
}

function registerTool(factory: ReturnType<typeof createWebFetchExtension>): CapturedTool {
  let tool: CapturedTool | undefined;
  factory({ registerTool: (definition: CapturedTool) => { tool = definition; } } as never);
  if (!tool) throw new Error("web_fetch did not register a tool");
  return tool;
}
