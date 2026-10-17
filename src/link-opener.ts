/**
 * Opening a web link a tool result shows, such as web_search's results. Inside a chat, App
 * provides an opener that shows the page in that chat's browser preview when the Browser
 * built-in is on; everywhere else (and with ⌘ held) it goes to the default browser through Rust,
 * which opens only http(s) URLs. The renderer itself never navigates or fetches.
 */
import { createContext, useContext } from "react";
import { api } from "./api";

export type LinkOpener = (url: string, options?: { external?: boolean }) => void;

export function openExternally(url: string): void {
  if (/^https?:\/\//i.test(url)) void api.revealPath(url).catch(() => undefined);
}

export const LinkOpenerContext = createContext<LinkOpener>((url) => openExternally(url));

export const useLinkOpener = (): LinkOpener => useContext(LinkOpenerContext);
