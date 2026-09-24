/**
 * Turns a fetched body into text the model can read. HTML goes through Mozilla's Readability
 * (the Firefox reader view) to drop navigation, ads and footers, then Turndown to Markdown;
 * pages Readability can't make sense of fall back to their whole cleaned-up body. Text formats
 * pass through unchanged, and anything binary is refused rather than dumped into the context.
 */
import { Readability } from "@mozilla/readability";
import { parseHTML } from "linkedom";
import TurndownService from "turndown";
import { WebFetchError } from "./fetch.js";

export interface ConvertedPage {
  title?: string;
  /** Markdown for HTML, otherwise the decoded text. */
  text: string;
  /** How the body was read, for the result header. */
  format: "html" | "text";
}

/** Tags whose content is never useful to the model. */
const NOISE_TAGS = ["script", "style", "noscript", "template", "iframe", "object", "embed", "svg", "canvas", "form", "button", "select", "dialog"];
const FALLBACK_NOISE = ["nav", "header", "footer", "aside"];

export function convertBody(body: Buffer, contentType: string, url: string): ConvertedPage {
  const mime = contentType.split(";")[0].trim().toLowerCase();
  const charset = /charset\s*=\s*"?([\w-]+)/i.exec(contentType)?.[1];
  if (isHtml(mime) || (!mime && sniffHtml(body))) {
    const html = decodeText(body, charset ?? metaCharset(body));
    return htmlToMarkdown(html, url);
  }
  if (isText(mime) || (!mime && !looksBinary(body))) {
    return { text: decodeText(body, charset).trim(), format: "text" };
  }
  throw new WebFetchError(`${url} is ${mime || "binary content"}, which web_fetch can't read. It only reads web pages and text.`);
}

function isHtml(mime: string): boolean {
  return mime === "text/html" || mime === "application/xhtml+xml";
}

function isText(mime: string): boolean {
  return (
    mime.startsWith("text/") ||
    mime === "application/json" ||
    mime.endsWith("+json") ||
    mime === "application/xml" ||
    mime.endsWith("+xml") ||
    mime === "application/javascript" ||
    mime === "application/ecmascript" ||
    mime === "application/x-javascript" ||
    mime === "application/yaml" ||
    mime === "application/x-yaml" ||
    mime === "application/toml" ||
    mime === "application/x-sh" ||
    mime === "application/graphql"
  );
}

function sniffHtml(body: Buffer): boolean {
  const head = body.subarray(0, 512).toString("latin1").trimStart().toLowerCase();
  return head.startsWith("<!doctype html") || head.startsWith("<html");
}

function looksBinary(body: Buffer): boolean {
  return body.subarray(0, 8192).includes(0);
}

/** `<meta charset>` or its http-equiv form, looked for where browsers look: near the top. */
function metaCharset(body: Buffer): string | undefined {
  const head = body.subarray(0, 4096).toString("latin1");
  return /<meta[^>]+charset\s*=\s*["']?([\w-]+)/i.exec(head)?.[1];
}

function decodeText(body: Buffer, charset: string | undefined): string {
  try {
    return new TextDecoder(charset ?? "utf-8").decode(body);
  } catch {
    return new TextDecoder("utf-8").decode(body);
  }
}

function htmlToMarkdown(html: string, url: string): ConvertedPage {
  const turndown = createTurndown();

  const { document } = parseHTML(html);
  absolutizeLinks(document, url);
  const pageTitle = document.title?.trim() || undefined;
  let article: ReturnType<Readability["parse"]> = null;
  try {
    article = new Readability(document as unknown as Document).parse();
  } catch {
    article = null;
  }
  if (article?.content && (article.textContent?.trim().length ?? 0) >= 200) {
    return { title: article.title?.trim() || pageTitle, text: tidy(turndown.turndown(article.content)), format: "html" };
  }

  // Readability mutates the document it reads, so the fallback starts from a fresh parse.
  const { document: fresh } = parseHTML(html);
  absolutizeLinks(fresh, url);
  for (const tag of FALLBACK_NOISE) for (const node of [...fresh.querySelectorAll(tag)]) node.remove();
  const root = fresh.body ?? fresh.documentElement;
  const text = root ? tidy(turndown.turndown(root.innerHTML ?? "")) : "";
  return { title: pageTitle, text, format: "html" };
}

function createTurndown(): TurndownService {
  const turndown = new TurndownService({ headingStyle: "atx", codeBlockStyle: "fenced", bulletListMarker: "-", emDelimiter: "*" });
  turndown.remove(NOISE_TAGS as TurndownService.Filter);
  // Inline data: images would flood the context with base64. Keep only real images with alt text.
  turndown.addRule("images", {
    filter: "img",
    replacement: (_content, node) => {
      const element = node as HTMLElement;
      const src = element.getAttribute("src") ?? "";
      const alt = (element.getAttribute("alt") ?? "").replace(/\s+/g, " ").trim();
      return alt && src && !src.startsWith("data:") ? `![${alt}](${src})` : "";
    },
  });
  return turndown;
}

/** Make links usable outside the page: the model only ever sees them as text. */
function absolutizeLinks(document: { querySelectorAll(selector: string): Iterable<unknown> }, base: string): void {
  for (const [selector, attribute] of [["a[href]", "href"], ["img[src]", "src"]] as const) {
    for (const node of document.querySelectorAll(selector)) {
      const element = node as { getAttribute(name: string): string | null; setAttribute(name: string, value: string): void; removeAttribute(name: string): void };
      const value = element.getAttribute(attribute) ?? "";
      if (/^\s*javascript:/i.test(value)) {
        element.removeAttribute(attribute);
        continue;
      }
      try {
        element.setAttribute(attribute, new URL(value, base).href);
      } catch {
        // Leave unparseable references as they were.
      }
    }
  }
}

function tidy(markdown: string): string {
  return markdown.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim();
}
