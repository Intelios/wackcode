/**
 * The network half of `web_fetch`: one GET, following redirects by hand. It uses `node:http(s)`
 * rather than global `fetch` so it can own DNS: the `lookup` hook resolves the host, refuses the
 * whole host if any address is private, and hands the socket exactly the addresses it checked, so
 * there is no gap between validation and connection for DNS rebinding to slip through. IP-literal
 * hosts skip `lookup`, so they are checked before the request. Every redirect hop repeats both.
 *
 * Nothing about the user travels with the request: no cookies, no auth headers, and never a
 * credential the worker holds. Proxy environment variables are ignored.
 */
import { lookup as dnsLookup, type LookupAddress } from "node:dns";
import { request as httpRequest, type IncomingMessage } from "node:http";
import { request as httpsRequest } from "node:https";
import { isIP, type LookupFunction } from "node:net";
import type { Readable } from "node:stream";
import { createBrotliDecompress, createGunzip, createInflate } from "node:zlib";
import { isBlockedAddress, isBlockedHostname } from "./address.js";

export const FETCH_TIMEOUT_MS = 30_000;
/** Decoded body bytes kept; the rest of a larger response is dropped. */
export const MAX_BODY_BYTES = 5 * 1024 * 1024;
export const MAX_REDIRECTS = 5;
const USER_AGENT = "WackCode-web_fetch/1.0";
const ACCEPT = "text/html,application/xhtml+xml,text/markdown,text/plain;q=0.9,application/json;q=0.9,*/*;q=0.5";

export interface FetchOptions {
  signal?: AbortSignal;
  timeoutMs?: number;
  maxBytes?: number;
  maxRedirects?: number;
  /** Tests widen this to reach a local server; production always uses the public-only check. */
  isAllowedAddress?: (address: string) => boolean;
}

export interface FetchedResponse {
  finalUrl: string;
  status: number;
  contentType: string;
  body: Buffer;
  /** The body hit `maxBytes` and was cut off there. */
  bodyTruncated: boolean;
}

/** An error whose message is safe to show the model and the user as is. */
export class WebFetchError extends Error {}

export async function fetchUrl(rawUrl: string, options: FetchOptions = {}): Promise<FetchedResponse> {
  const timeoutMs = options.timeoutMs ?? FETCH_TIMEOUT_MS;
  const maxRedirects = options.maxRedirects ?? MAX_REDIRECTS;
  const isAllowed = options.isAllowedAddress ?? ((address: string) => !isBlockedAddress(address));
  const timeout = AbortSignal.timeout(timeoutMs);
  const signal = options.signal ? AbortSignal.any([options.signal, timeout]) : timeout;

  let url = parseUrl(rawUrl);
  try {
    for (let hop = 0; ; hop += 1) {
      const response = await get(url, signal, isAllowed);
      const location = response.headers.location;
      if (isRedirect(response.statusCode) && location) {
        response.destroy();
        if (hop >= maxRedirects) throw new WebFetchError(`Stopped after ${maxRedirects} redirects.`);
        url = parseUrl(new URL(location, url).href);
        continue;
      }
      const status = response.statusCode ?? 0;
      if (status < 200 || status >= 300) {
        response.destroy();
        throw new WebFetchError(`${url.href} returned HTTP ${status}${response.statusMessage ? ` ${response.statusMessage}` : ""}.`);
      }
      const { body, truncated } = await readBody(response, options.maxBytes ?? MAX_BODY_BYTES);
      return {
        finalUrl: url.href,
        status,
        contentType: String(response.headers["content-type"] ?? ""),
        body,
        bodyTruncated: truncated,
      };
    }
  } catch (error) {
    if (error instanceof WebFetchError) throw error;
    if (timeout.aborted) throw new WebFetchError(`Timed out after ${Math.round(timeoutMs / 1000)} s fetching ${url.href}.`);
    if (options.signal?.aborted) throw new WebFetchError("The fetch was stopped.");
    throw new WebFetchError(`Could not fetch ${url.href}: ${describe(error)}`);
  }
}

/** Validate a URL the model or a redirect supplied. */
export function parseUrl(raw: string): URL {
  let url: URL;
  try {
    url = new URL(raw.trim());
  } catch {
    throw new WebFetchError(`Not a valid URL: ${raw}`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new WebFetchError(`Only http and https URLs can be fetched, not ${url.protocol.replace(/:$/, "")}.`);
  }
  if (url.username || url.password) throw new WebFetchError("URLs with embedded credentials are not fetched.");
  if (isBlockedHostname(url.hostname)) throw new WebFetchError(privateMessage(url.hostname));
  return url;
}

function privateMessage(host: string): string {
  return `${host} is a local or private network address. web_fetch only reads public web pages.`;
}

function isRedirect(status: number | undefined): boolean {
  return status === 301 || status === 302 || status === 303 || status === 307 || status === 308;
}

function get(url: URL, signal: AbortSignal, isAllowed: (address: string) => boolean): Promise<IncomingMessage> {
  const hostname = url.hostname.replace(/^\[|\]$/g, "");
  if (isIP(hostname) && !isAllowed(hostname)) return Promise.reject(new WebFetchError(privateMessage(hostname)));

  const lookup: LookupFunction = (host, lookupOptions, callback) => {
    dnsLookup(host, { all: true, family: lookupOptions.family, hints: lookupOptions.hints }, (error, addresses: LookupAddress[]) => {
      if (error) return callback(error, "", 0);
      if (addresses.length === 0 || addresses.some((entry) => !isAllowed(entry.address))) {
        return callback(new WebFetchError(privateMessage(host)), "", 0);
      }
      if (lookupOptions.all) return (callback as unknown as (error: null, addresses: LookupAddress[]) => void)(null, addresses);
      callback(null, addresses[0].address, addresses[0].family);
    });
  };

  const send = url.protocol === "https:" ? httpsRequest : httpRequest;
  return new Promise((resolve, reject) => {
    const req = send(url, {
      method: "GET",
      agent: false,
      lookup,
      signal,
      headers: { "user-agent": USER_AGENT, accept: ACCEPT, "accept-encoding": "gzip, deflate, br" },
    });
    req.once("response", resolve);
    req.once("error", reject);
    req.end();
  });
}

async function readBody(response: IncomingMessage, maxBytes: number): Promise<{ body: Buffer; truncated: boolean }> {
  const stream = decode(response);
  const chunks: Buffer[] = [];
  let size = 0;
  let truncated = false;
  try {
    for await (const chunk of stream as AsyncIterable<Buffer>) {
      if (size + chunk.length > maxBytes) {
        chunks.push(chunk.subarray(0, maxBytes - size));
        size = maxBytes;
        truncated = true;
        break;
      }
      chunks.push(chunk);
      size += chunk.length;
    }
  } finally {
    stream.destroy();
    response.destroy();
  }
  return { body: Buffer.concat(chunks, size), truncated };
}

/** Undo Content-Encoding. The size cap applies to the decoded bytes, so a small bomb can't expand past it. */
function decode(response: IncomingMessage): Readable {
  const encoding = String(response.headers["content-encoding"] ?? "").trim().toLowerCase();
  const decoder =
    encoding === "gzip" || encoding === "x-gzip"
      ? createGunzip()
      : encoding === "deflate"
        ? createInflate()
        : encoding === "br"
          ? createBrotliDecompress()
          : undefined;
  if (!decoder) return response;
  response.on("error", (error) => decoder.destroy(error));
  return response.pipe(decoder);
}

function describe(error: unknown): string {
  const code = (error as { code?: unknown })?.code;
  if (code === "ENOTFOUND" || code === "EAI_AGAIN") return "the host name could not be resolved.";
  if (code === "ECONNREFUSED") return "the connection was refused.";
  if (code === "ECONNRESET") return "the connection was reset.";
  if (typeof code === "string" && (code.startsWith("ERR_TLS") || code.includes("CERT"))) {
    return "its TLS certificate could not be verified.";
  }
  return error instanceof Error ? error.message : String(error);
}
