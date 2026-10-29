# Model reference catalog

`src-tauri/src/model_catalog.rs` owns the versioned, normalized metadata imported from
[Models.dev](https://github.com/anomalyco/models.dev). It is independent of Pi's pinned runtime
catalogue. Both Code and Chat use the same saved connection records; neither receives this
reference catalog or changes when it refreshes. Subscription model lists still belong to Pi.

## Snapshot and refresh

`src-tauri/catalog/models.json` is embedded in Rust. `list_model_catalog` reads it and the app
data folder's `model-catalog.json`, choosing the newer valid capture. A missing, invalid or
incompatible cache falls back to the bundle. There is no expiry, launch-time fetch or timer.

`refresh_model_catalog` alone performs a credential-free GET to
`https://models.dev/catalog.json`: no redirects, a 20-second timeout, and a 16 MiB bound checked
both before and during reading. It normalizes and validates before publishing through an
atomic temporary-file replacement. Failure preserves the previous snapshot. It never writes
`wackcode.json`, touches worker state or installs anything.

The importer retains only text/tool-calling models, names, canonical identity, metadata-source
profiles, limits, reasoning controls, image modalities and endpoint/protocol matching hints.
It drops keys, headers, runtime compatibility flags, pricing and other executable/request
configuration. Missing limits stay unknown. Explicit reasoning efforts map to WackCode levels
(`none` becomes `off`); a toggle permits `off` alongside explicit efforts. Toggle/budget-only
or unfamiliar controls require manual setup, rather than inventing an effort list.

Canonical IDs group hosts and aliases; unlinked entries remain provider-qualified. The
importer's few native endpoint hints and SDK-to-protocol mappings are code-owned, used only
for matching, and never contacted. Unknown protocols or ambiguous base paths require a source
choice. A source's URL never becomes a connection endpoint.

## Settings

Lookup previews the chosen source and its limits before applying to a draft. It keeps an
existing model ID and API override and fills only known limits. Unknown effort controls leave
the user's choices for manual review. Fetch models continues importing IDs with blank limits.
Catalog refresh changes suggestions only; saved models change through lookup and Save.

## Updating the bundle

Run `pnpm catalog:update` to explicitly download and regenerate the bundled snapshot using the
same Rust importer. For a previously downloaded feed, use
`pnpm catalog:update --input /absolute/path/to/catalog.json`. Review the snapshot diff and run
the catalog tests. Normal builds never download model metadata. Models.dev's MIT notice is
in `src-tauri/catalog/LICENSE.models.dev` and included in the app's resources.

Pi remains pinned: `worker/src/catalog-models.ts` supplies runtime compatibility such as
adaptive thinking and gateway request shaping. A new API or protocol behavior may still
require a Pi or WackCode update even when fresh metadata is available.
