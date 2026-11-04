# MCP startup experiment

The first model request used to await every configured MCP server's handshake and
`tools/list`, even when the worker had already started those connections at `ready`.
The wait was needed to describe tools accurately; starting connections earlier alone
could not remove it when Rust sent `init` and `prompt` back to back.

The worker now keeps an advisory tool catalogue across chats. Previously discovered
tools are in the first request while each chat's own MCP connections start in the
background. Calling an MCP tool early waits for that server only. New servers and
cache misses still need discovery before the first request. Code and Chat mode use
the same startup behavior; no server process is shared between chats.

## Measurement

Measured on this Apple Silicon Mac on 4 November 2026, using real worker processes,
a disposable Git workspace per chat, a local Chat Completions provider, and the
repository's stdio MCP fixture with a deliberate 5,000 ms startup delay. No real
provider credentials, paid model calls or production MCP servers were used.

The primary metric is worker spawn → provider receipt of the first model request.
It includes Node boot and worker initialization; it excludes renderer/Rust dispatch,
provider network latency and model generation. This isolates the startup dependency,
rather than predicting the speed of a particular real-world server.

Baseline samples before the change (milliseconds):

| Run | Spawn → ready | Spawn → first model request | Ready → request |
| --- | ---: | ---: | ---: |
| 1 | 368 | 5,499 | 5,131 |
| 2 | 317 | 5,436 | 5,119 |
| 3 | 323 | 5,440 | 5,117 |
| 4 | 325 | 5,414 | 5,089 |

The benchmark uses the upper middle sample for even-sized medians: **5,440 ms**
spawn → request and **5,119 ms** ready → request. Every baseline chat paid discovery.

After the change, in a separate six-run measurement:

| Run | Catalogue | Spawn → first model request | Ready → request | Spawn → MCP ready | MCP tools in first request |
| --- | --- | ---: | ---: | ---: | ---: |
| 0 | Empty | 5,491 | 5,136 | 5,425 | 5 |
| 1 | Cached | 384 | 40 | 5,411 | 5 |
| 2 | Cached | 371 | 39 | 5,405 | 5 |
| 3 | Cached | 681 | 57 | 5,713 | 5 |
| 4 | Cached | 385 | 39 | 5,420 | 5 |
| 5 | Cached | 364 | 40 | 5,392 | 5 |

The five cached chats have a median of **384 ms** spawn → request and **40 ms**
ready → request. Compared with the baseline, that removes **5,056 ms (93%)** from
time to the first model request. MCP itself still becomes ready at a median of
**5,411 ms**; its boot overlaps the model. The one empty-catalogue chat still waits
for discovery (**5,491 ms**). These are fixture measurements, not production-server
or end-to-end app latency measurements.

## Reproduce

```sh
pnpm build:worker
MCP_DELAY_MS=5000 RUNS=6 node worker/scripts/bench-startup.mjs
```

Run 0 starts with an empty temporary catalogue. Runs 1–5 are new workers and new
workspaces sharing only that catalogue, as independent chats do in the app.
`spawn→mcp` observes a fixture marker written when `tools/list` is answered (within
the benchmark's 5 ms polling interval). `mcpTools` must be 5 in every first model
request; the script exits unsuccessfully for a missing request, missing readiness
marker or incomplete tool inventory. Run without `MCP_DELAY_MS` for the ordinary
worker-startup benchmark. Temporary workspaces and the shared catalogue are removed.

## Correctness and limits

Gated integration tests verify that the model starts while discovery remains blocked,
that an early tool call waits only for its own server while another remains blocked,
and that Stop cancels the wait and a subsequent message reconnects. They exercise
Code and Chat mode, configuration/credential changes, server removal and discovery
failures. Removed tools, changed input schemas and changed read-only hints are checked
against the live server before execution. Tool declarations refresh between runs.

The private cache stores declarations only. Its connection fingerprint includes
transport, command, arguments, URL, headers and environment, without persisting those
values. Credential-bearing schemas are skipped; missing, damaged, oversized, symlinked,
expired or unwritable cache files fall back to discovery. Catalogues expire after
seven days. Every worker still performs its own handshake and tool listing, so the
optimization removes the dependency on startup, rather than making servers boot faster.
