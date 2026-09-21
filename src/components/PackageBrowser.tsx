import { useEffect, useRef, useState } from "react";
import type { PackageSearchResult } from "../types";

interface Props {
  /** Sources already installed, so those rows offer nothing to do. */
  installed: Set<string>;
  busy: boolean;
  onSearch: (query: string) => Promise<PackageSearchResult[]>;
  onInstall: (source: string) => void;
}

/**
 * Browses the public npm registry for packages carrying the `pi-package` keyword — the same
 * catalogue pi.dev lists. Results are data from a third party: they are rendered as text and
 * nothing in them is executed or followed automatically.
 */
export function PackageBrowser({ installed, busy, onSearch, onInstall }: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<PackageSearchResult[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const latest = useRef(0);

  useEffect(() => {
    const token = ++latest.current;
    const timer = setTimeout(() => {
      setLoading(true);
      setError(undefined);
      void onSearch(query)
        .then((found) => { if (token === latest.current) setResults(found); })
        .catch((reason: unknown) => { if (token === latest.current) setError(String(reason)); })
        .finally(() => { if (token === latest.current) setLoading(false); });
    }, query ? 300 : 0);
    return () => clearTimeout(timer);
  }, [query, onSearch]);

  return (
    <>
      <div className="section-heading-row">
        <div>
          <h3>Browse packages</h3>
          <p>Published Pi packages from the npm registry. Searching contacts npmjs.com.</p>
        </div>
      </div>

      <input
        className="package-search"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Search packages…"
        aria-label="Search packages"
        spellCheck={false}
      />

      {error && <div className="error-banner">{error}</div>}

      {loading && results.length === 0 ? (
        <div className="model-empty">Searching…</div>
      ) : results.length === 0 ? (
        <div className="model-empty">{error ? "Search unavailable." : "No packages matched that search."}</div>
      ) : (
        <div className="search-results">
          {results.map((result) => {
            const source = `npm:${result.name}`;
            const already = installed.has(source);
            return (
              <article className="search-result" key={result.name}>
                <div className="search-result-head">
                  <span className="search-result-name" title={result.name}>{result.name}</span>
                </div>
                <div className="search-result-meta">
                  v{result.version}{result.publisher && ` · ${result.publisher}`}
                </div>
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
      )}
    </>
  );
}
