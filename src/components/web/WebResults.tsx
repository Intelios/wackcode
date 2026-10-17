import { motion, useReducedMotion } from "motion/react";
import type { MouseEvent } from "react";
import { siteInitial } from "../../web-presets";

export interface WebResultItem {
  title: string;
  url: string;
  host: string;
  snippet?: string;
  date?: string;
}

/** A provider's date as a short day: "1 Jun 2026" for a stamp, otherwise as given. */
function shortDate(date: string): string {
  const parsed = /^\d{4}-\d{2}-\d{2}/.test(date) ? new Date(date) : undefined;
  if (!parsed || Number.isNaN(parsed.getTime())) return date;
  return parsed.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
}

/**
 * A stack of search results: a site monogram (never the site's own icon: the renderer fetches
 * nothing), the title as a real link, the host and date, and the snippet. The cards drop in one
 * after another. Used by Settings › Web's Test search and by the transcript's results sheet.
 */
export function WebResults({ results, onOpen, label = "Search results" }: {
  results: readonly WebResultItem[];
  /** Opens a result; the anchor's own navigation is always prevented. */
  onOpen: (url: string, event: MouseEvent<HTMLAnchorElement>) => void;
  label?: string;
}) {
  const reduce = useReducedMotion();
  return (
    <ol className="web-results" aria-label={label}>
      {results.map((result, index) => (
        <motion.li
          key={`${result.url}-${index}`}
          className="web-result"
          initial={reduce ? { opacity: 0 } : { opacity: 0, y: 8, scale: 0.98 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={reduce ? { duration: 0.15 } : { type: "spring", stiffness: 420, damping: 30, delay: Math.min(index, 8) * 0.05 }}
        >
          <span className="web-result-mark" aria-hidden="true">{siteInitial(result.host)}</span>
          <div className="web-result-text">
            <a
              href={result.url}
              className="web-result-title"
              onClick={(event) => { event.preventDefault(); onOpen(result.url, event); }}
            >
              {result.title}
            </a>
            <span className="web-result-meta">
              {result.host}{result.date ? <> · {shortDate(result.date)}</> : null}
            </span>
            {result.snippet && <p className="web-result-snippet">{result.snippet}</p>}
          </div>
        </motion.li>
      ))}
    </ol>
  );
}
