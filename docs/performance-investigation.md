# Energy investigation — 9 October 2026

Reported symptom: fans and high temperatures while an agent was running. The two
Activity Monitor screenshots show WackCode Energy Impact of **1,940.4** and
**542.6**, with **295.64** in the 12 hr Power column.

Apple defines [Energy Impact](https://support.apple.com/guide/activity-monitor/view-energy-consumption-actmntr43697/mac)
as a relative current-consumption measure; 12 hr Power is its average over that
period. These figures establish high app-attributed activity, but are not watts,
CPU percentages or temperature readings. They cannot identify the responsible
worker, renderer, browser page or agent-launched command.

## Confirmed fault

Two development workers started on 8 October were still running on 9 October,
about 22 hours later, with parent PID 1. Their command was this checkout's
`worker/dist/index.js`. Each had accumulated nearly six hours of CPU time.
A later process sample measured **98.5%** and **97.9%** CPU: roughly two cores
between them. Their original host was gone; the installed app running during
this investigation was a separate, newer process.

Three-second `sample` traces of both processes showed the main thread repeatedly
entering `TriggerUncaughtException`, formatting errors and writing to a stream.
The worker's uncaught-exception handler emitted `worker_error` to stdout and only
set `process.exitCode`. A broken stdout pipe can therefore raise another
exception on the next event-loop tick, which tries the same broken write again.
There was also no stdin EOF handler, so host death did not cancel work. SIGTERM
cleanup had no overall deadline and could wait indefinitely for an abort.

Process-level regression tests reproduced survival after a broken stdout pipe,
stdin EOF with a live timer, an uncaught exception and an unhandled rejection.
All four failed before the fix and passed after it.

The two old workers did not finish after SIGTERM and were stopped with SIGKILL.
Both were confirmed absent afterwards. At that later snapshot the installed
WackCode host was at 0.3% CPU, its WebContent process at 1.0%, and its GPU process
at 0.9%. This is an idle observation, not a before/after active-agent benchmark.

## Fix and limits

Code and Chat workers now share an idempotent shutdown path that cancels owned
work, stops on stdin EOF or stream errors, and exits within three seconds if
cleanup stalls. Broken pipes are never reported back through stdout. Fatal
errors trigger termination rather than just setting an exit code. The shutdown
command bypasses both the active prompt queue and the initialization gate, and
queued commands cannot start after shutdown begins.

This removes a confirmed source of sustained machine load. It does not prove
that these orphan workers account for every spike in the supplied screenshots.
Agent-launched builds/tests and live rendering still consume resources during
ordinary work. The source and development worker build contain the fix; an
already installed app requires a new build to receive it.

Validation: `pnpm check` passed, as did the full worker suite (500 tests at that
point). The final regression checks cover broken stdout, stdin EOF, fatal
exceptions/rejections, shutdown before initialization, active provider streams
in both chat kinds, and an extension tool hook that ignores cancellation so
the three-second exit deadline is exercised.

If heating returns with this fix, capture the CPU pane with the process tree
expanded while it is happening, and a short stack sample of the busy process.
That distinguishes a worker loop from Rust work, WebKit rendering or a command
the agent launched. Energy screenshots alone cannot make that distinction.
