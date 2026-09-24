import { useEffect, useRef, useState, type ReactNode } from "react";
import type { PackageSearchResult } from "../types";

/** One page of results. `notice` explains a degraded source (e.g. pi.dev was unreachable). */
export interface BrowsePage {
  results: PackageSearchResult[];
  hasMore: boolean;
  notice?: string;
}

interface Props {
  /** Sources already installed, so those rows offer nothing to do. */
  installed: Set<string>;
  busy: boolean;
  /** 1-based pages. A new `onSearch` (say, another sort order) starts over at page 1. */
  onSearch: (query: string, page: number) => Promise<BrowsePage>;
  onInstall: (source: string) => void;
  heading?: string;
  blurb?: ReactNode;
  placeholder?: string;
  /** Beside the search field, e.g. a sort order. */
  toolbar?: ReactNode;
}

/**
 * Browses published Pi packages. Results are data from a third party: they are rendered as text
 * and nothing in them is executed or followed automatically.
 */
export function PackageBrowser({
  installed, busy, onSearch, onInstall,
  heading = "Browse packages",
  blurb = "Published Pi packages from the npm registry. Searching contacts npmjs.com.",
  placeholder = "Search packages…",
  toolbar
}: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<PackageSearchResult[]>([]);
  const [page, setPage] = useState(1);
  const [hasMore, setHasMore] = useState(false);
  const [notice, setNotice] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const latest = useRef(0);

  useEffect(() => {
    const token = ++latest.current;
    const timer = setTimeout(() => {
      setLoading(true);
      setError(undefined);
      void onSearch(query, 1)
        .then((found) => {
          if (token !== latest.current) return;
          setResults(found.results);
          setHasMore(found.hasMore);
          setNotice(found.notice);
          setPage(1);
        })
        .catch((reason: unknown) => { if (token === latest.current) setError(String(reason)); })
        .finally(() => { if (token === latest.current) setLoading(false); });
    }, query ? 300 : 0);
    return () => clearTimeout(timer);
  }, [query, onSearch]);

  function loadMore(): void {
    const token = ++latest.current;
    const next = page + 1;
    setLoading(true);
    setError(undefined);
    void onSearch(query, next)
      .then((found) => {
        if (token !== latest.current) return;
        // A catalogue that shifted between pages can repeat a row; keep the first.
        setResults((current) => {
          const seen = new Set(current.map((result) => result.name));
          return [...current, ...found.results.filter((result) => !seen.has(result.name))];
        });
        setHasMore(found.hasMore);
        setNotice(found.notice);
        setPage(next);
      })
      .catch((reason: unknown) => { if (token === latest.current) setError(String(reason)); })
      .finally(() => { if (token === latest.current) setLoading(false); });
  }

  return (
    <>
      <div className="section-heading-row">
        <div>
          <h3>{heading}</h3>
          <p>{blurb}</p>
        </div>
      </div>

      <div className="package-search-row">
        <input
          className="package-search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={placeholder}
          aria-label={placeholder.replace(/…$/, "")}
          spellCheck={false}
        />
        {toolbar}
      </div>

      {error && <div className="error-banner">{error}</div>}
      {notice && !error && <div className="package-notice" role="status">{notice}</div>}

      {loading && results.length === 0 ? (
        <div className="model-empty">Searching…</div>
      ) : results.length === 0 ? (
        <div className="model-empty">{error ? "Search unavailable." : "No packages matched that search."}</div>
      ) : (
        <>
          <div className="search-results">
            {results.map((result) => {
              const source = `npm:${result.name}`;
              const already = installed.has(source);
              const meta = [
                result.version && `v${result.version}`,
                result.publisher,
                result.downloads !== undefined && `${result.downloads.toLocaleString()}/mo`
              ].filter(Boolean).join(" · ");
              return (
                <article className="search-result" key={result.name}>
                  <div className="search-result-head">
                    <span className="search-result-name" title={result.name}>{result.name}</span>
                  </div>
                  {meta && <div className="search-result-meta">{meta}</div>}
                  {result.types && result.types.length > 0 && (
                    <div className="search-result-types" aria-label="Contains">
                      {result.types.map((kind) => (
                        <span className={`package-chip ${kind === "skill" ? "" : "idle"}`} key={kind}>{kind}</span>
                      ))}
                    </div>
                  )}
                  {result.description && (
                    <p className="search-result-description" title={result.description}>
                      {result.description}
                    </p>
                  )}
                  <div className="search-result-actions">
                    {already ? (
                      <span className="package-meta installed-badge">Installed</span>
                    ) : (
                      <button type="button" className="secondary-button" disabled={busy} onClick={() => onInstall(source)}>
                        Install
                      </button>
                    )}
                  </div>
                </article>
              );
            })}
          </div>
          {hasMore && (
            <div className="search-more">
              <button type="button" className="secondary-button" disabled={loading} onClick={loadMore}>
                {loading ? "Loading…" : "More results"}
              </button>
            </div>
          )}
        </>
      )}
    </>
  );
}
