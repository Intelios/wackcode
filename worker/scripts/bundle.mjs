/**
 * Bundles the chat worker into dist/worker.js after `tsc`, because start-up time was mostly
 * Node opening and compiling ~2,200 separate files. Run by `pnpm build`.
 *
 * Pi itself is NOT bundled here. Its code finds its own files (package.json, the extension
 * loader's aliases, wasm, worker threads) relative to where it sits on disk, so every
 * `@earendil-works/pi-coding-agent` import points at Pi's own pre-built bundle instead: the
 * build its `pi` CLI runs, where installed extensions get Pi's in-memory modules. Everything else
 * (WackCode's code, the MCP client, web-fetch's parsers, pi-ai's pure helpers) goes in one file.
 *
 * node_modules still ships next to dist/: Pi's bundle imports a few packages at runtime, and the
 * read-only session helpers (`saved-session.ts`, `chat-refs.ts`) deep-import Pi's unbundled files.
 */
import { build } from "esbuild";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
// Relative to dist/worker.js. Resolves in the dev checkout (worker/node_modules) and in the
// app's hoisted copy (resources/worker/node_modules). Pi's exports map hides this path from a
// bare specifier, so it has to be a file path.
const PI_BUNDLE = "../node_modules/@earendil-works/pi-coding-agent/dist/bundle/index.js";

// Libraries only the web_fetch HTML path and the MCP client need, kept out of the bundle and
// loaded lazily at first use. Bundling them cost every worker spawn ~110 ms and ~60 ms of
// eval even when the chat never fetched a page or had no MCP servers. They are real
// dependencies in package.json, so `pnpm deploy` ships them under resources/worker and Node
// resolves them from node_modules next to dist/ in dev and in the packaged app.
const lazy = [
  "linkedom",
  "turndown",
  "@mozilla/readability",
  "@modelcontextprotocol/client",
  "@modelcontextprotocol/client/*",
];

await build({
  entryPoints: [join(root, "dist", "index.js")],
  outfile: join(root, "dist", "worker.js"),
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node24",
  sourcemap: true,
  logLevel: "warning",
  external: lazy,
  // CommonJS dependencies call require(); an ES module has none of its own.
  banner: { js: "import { createRequire as __wcCreateRequire } from \"node:module\"; const require = __wcCreateRequire(import.meta.url);" },
  plugins: [{
    name: "pi-bundle",
    setup(builder) {
      builder.onResolve({ filter: /^@earendil-works\/pi-coding-agent$/ }, () => ({ path: PI_BUNDLE, external: true }));
    }
  }]
});
