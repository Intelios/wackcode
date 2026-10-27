import { createServer } from "node:http";

const port = Number(process.env.WACKCODE_MOCK_PORT ?? 43127);

function send(response, value) {
  response.write(`data: ${JSON.stringify(value)}\n\n`);
}

function textOf(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content.map((part) => typeof part?.text === "string" ? part.text : "").join("");
}

function promptOf(content) {
  return textOf(content)
    .replace(/<wackcode-image-references-v1>[\s\S]*?<\/wackcode-image-references-v1>/g, "")
    .replace(/\(image omitted: model does not support images\)/g, "")
    .trimEnd();
}

function toolImageCarrier(message) {
  return message?.role === "user" && Array.isArray(message.content) && message.content.length > 1
    && message.content[0]?.type === "text" && message.content[0].text === "Attached image(s) from tool result:"
    && message.content.slice(1).every((part) => part?.type === "image_url");
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://${request.headers.host ?? "localhost"}`);
  if (request.method === "GET" && url.pathname === "/v1/models") {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ object: "list", data: [{ id: "wackcode-fixture-model", object: "model" }] }));
    return;
  }
  // A SearXNG-shaped search endpoint, so Settings › Web can be tried without a paid provider:
  // add a SearXNG provider at http://127.0.0.1:43127 (no key).
  if (request.method === "GET" && url.pathname === "/search") {
    if (url.searchParams.get("format") !== "json") {
      response.writeHead(403).end("JSON format is disabled");
      return;
    }
    const query = url.searchParams.get("q") ?? "";
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({
      query,
      results: [
        { title: `${query} — fixture result one`, url: "https://example.com/one", content: `The first fixture page about ${query}.`, publishedDate: "2026-06-01T00:00:00" },
        { title: "Fixture docs", url: "https://docs.example.org/guide", content: "A second, documentation-shaped result." },
        { title: "Fixture forum thread", url: "https://forum.example.net/t/42", content: "Someone asked the same question." },
      ],
    }));
    return;
  }
  if (request.method !== "POST" || url.pathname !== "/v1/chat/completions") {
    response.writeHead(404).end();
    return;
  }
  const chunks = [];
  for await (const chunk of request) chunks.push(Buffer.from(chunk));
  const body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  const messages = Array.isArray(body.messages) ? body.messages : [];
  // Pi's synthetic vision carrier is not a new user prompt. It may follow an assistant bridge.
  const latestUser = messages.findLast((message) => message?.role === "user" && !toolImageCarrier(message));
  const prompt = promptOf(latestUser?.content);
  response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache", connection: "keep-alive" });
  if (prompt.toLowerCase().includes("wait until stopped")) {
    response.write(": waiting for cancellation\n\n");
    process.stdout.write("slow request started\n");
    return;
  }
  // A long, tool-free turn for dynamic backgrounds and streaming/scroll performance checks.
  // Stays entirely on localhost and can be stopped at any time from either area.
  if (prompt.trim().toLowerCase() === "dynamic background fixture") {
    const chunk = (delta, finish_reason = null) => send(response, {
      id: "fixture-dynamic", object: "chat.completion.chunk", created: 1, model: body.model,
      choices: [{ index: 0, delta, finish_reason }]
    });
    chunk({ role: "assistant" });
    for (let n = 0; n < 16 && !response.destroyed; n++) {
      chunk({ reasoning_content: "Considering the shapes, motion, and contrast. " });
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    for (let n = 0; n < 80 && !response.destroyed; n++) {
      chunk({ content: `Paragraph ${n + 1}: A quiet background moves behind this reply while the words stay readable. You can scroll, type a draft, or switch chats as this local fixture continues.\n\n` });
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    if (!response.destroyed) { chunk({}, "stop"); response.end("data: [DONE]\n\n"); }
    return;
  }
  const hasToolResult = messages.at(-1)?.role === "tool" || toolImageCarrier(messages.at(-1));
  const background = prompt.toLowerCase().includes("background fixture");
  // Chat mode's confinement check: the write aims one folder above the workspace.
  const escape = prompt.toLowerCase().includes("escape fixture");
  const child = prompt.startsWith("background-fixture-child");
  // "suggest fixture" files one suggested task (when the built-in is on), then finishes the turn.
  const suggestFixture = /^suggest fixture/i.test(prompt);
  const suggest = suggestFixture && !hasToolResult && body.tools?.some((tool) => tool.function?.name === "suggest_task");
  // "canvas fixture" draws a base artboard and two theme variants, looks at them, then answers.
  // It also matches inside /canvas's generated prompt ("/canvas canvas fixture").
  const canvasFixture = prompt.toLowerCase().includes("canvas fixture");
  const sinceUser = messages.slice(messages.lastIndexOf(latestUser) + 1);
  const calledTool = (name) => sinceUser.some((message) => message.role === "assistant" && message.tool_calls?.some((call) => call.function?.name === name));
  const offers = (name) => body.tools?.some((tool) => tool.function?.name === name);
  // Attach an image, or name a local image after the colon, to exercise inline replies in
  // either area. Read still goes through the real worker (including Chat's confinement).
  const imageFixture = /^image reply fixture(?:[ \t]*:[ \t]*([^\n]+))?(?:\n|$)/i.exec(prompt);
  const imageCall = imageFixture?.[1] && offers("read") && !calledTool("read")
    ? { name: "read", arguments: JSON.stringify({ path: imageFixture[1].trim() }) }
    : undefined;
  const canvasArgs = canvasFixtureArguments();
  const canvasCall = canvasFixture && offers("canvas") && !calledTool("canvas")
    ? { name: "canvas", arguments: JSON.stringify(prompt.toLowerCase().includes("revise")
      ? { canvas: "billing-settings-themes", artboards: [{ ...canvasArgs.artboards[0], id: "base" }] }
      : canvasArgs) }
    : canvasFixture && offers("canvas_look") && calledTool("canvas") && !calledTool("canvas_look")
      ? { name: "canvas_look", arguments: JSON.stringify({ canvas: "billing-settings-themes" }) }
      : undefined;
  // Slow authoring exposes the long argument-generation phase before tool execution. Use
  // "slow canvas fixture pause" to include a quiet stream, or Stop before the call is accepted.
  if (canvasCall?.name === "canvas" && prompt.toLowerCase().includes("slow canvas fixture")) {
    const chunk = (delta, finish_reason = null) => send(response, {
      id: "fixture-canvas-stream", object: "chat.completion.chunk", created: 1, model: body.model,
      choices: [{ index: 0, delta, finish_reason }]
    });
    const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    chunk({ role: "assistant", reasoning_content: "Preparing the canvas directions." });
    await wait(150);
    chunk({ tool_calls: [{ index: 0, id: `fixture-canvas-${messages.length}`, type: "function", function: { name: "canvas", arguments: "" } }] });
    for (let offset = 0; offset < canvasCall.arguments.length && !response.destroyed; offset += 80) {
      chunk({ tool_calls: [{ index: 0, function: { arguments: canvasCall.arguments.slice(offset, offset + 80) } }] });
      await wait(offset === 240 && prompt.toLowerCase().includes("pause") ? 18_000 : Number(process.env.WACKCODE_MOCK_CANVAS_CHUNK_MS ?? 180));
    }
    if (!response.destroyed) { chunk({}, "tool_calls"); response.end("data: [DONE]\n\n"); }
    return;
  }
  // "search fixture: <query>" calls web_search when Settings › Web offers it.
  const searchQuery = /^search fixture:\s*(.+)$/i.exec(prompt)?.[1];
  const search = searchQuery && body.tools?.some((tool) => tool.function?.name === "web_search") ? searchQuery : undefined;
  // Explicit browser fixtures drive the real host through the app without a paid model.
  const browserMatch = /^browser fixture:[ \t]*(browser_(?:open|snapshot|act|screenshot|console))[ \t]*(\{[^\n]*\})?(?:\n|$)/.exec(prompt);
  const browser = browserMatch && body.tools?.some((tool) => tool.function?.name === browserMatch[1])
    ? { name: browserMatch[1], arguments: browserMatch[2] || "{}" } : undefined;
  // Computer fixtures exercise the actual app-consent boundary, never a mock permission store.
  const computerMatch = /^computer fixture:[ \t]*(computer_(?:apps|open|snapshot|screenshot|act))[ \t]*(\{[^\n]*\})?(?:\n|$)/.exec(prompt);
  const computer = computerMatch && body.tools?.some((tool) => tool.function?.name === computerMatch[1])
    ? { name: computerMatch[1], arguments: computerMatch[2] || "{}" } : undefined;
  // "commit fixture" exercises the transcript's commit row: one bash call, all Git, committing
  // --allow-empty into a throwaway repo under /tmp so no open workspace's Git is touched.
  const commitFixture = /^commit fixture$/i.test(prompt);
  const commitCall = commitFixture && !hasToolResult && body.tools?.some((tool) => tool.function?.name === "bash")
    ? {
        name: "bash",
        arguments: JSON.stringify({
          command: "git init -q /tmp/wackcode-commit-fixture && git -C /tmp/wackcode-commit-fixture config user.email fixture@wackcode.dev && git -C /tmp/wackcode-commit-fixture config user.name \"WackCode Fixture\" && git -C /tmp/wackcode-commit-fixture commit --allow-empty -m \"Ship the billing page\""
        })
      }
    : undefined;
  if (child) {
    response.write(": background child working\n\n");
    if (prompt.includes("until stopped")) return;
    await new Promise((resolve) => setTimeout(resolve, Number(process.env.WACKCODE_MOCK_DELAY_MS ?? 25_000)));
    if (response.destroyed) return;
  }
  const subagent = body.tools?.find((tool) => tool.function?.name === "subagent");
  const roles = subagent?.function.parameters.properties.agent.enum ?? [];
  // Chat Completions tool replies have tool_call_id but may omit name. Match the actual
  // assistant call so the fixture never repeats a launch after either success or rejection.
  const backgroundLaunched = messages.slice(messages.lastIndexOf(latestUser) + 1).some((message) => message.role === "assistant" && message.tool_calls?.some((call) => call.function?.name === "subagent"));
  const resultsArrived = prompt.includes("Job ");
  const backgroundLaunch = background && subagent && !backgroundLaunched && !resultsArrived;
  if (backgroundLaunch || suggest || canvasCall || imageCall || commitCall || (!background && !suggestFixture && !canvasFixture && !imageFixture && !child && !resultsArrived && !hasToolResult && body.tools?.length && (!browserMatch || browser) && (!computerMatch || computer))) {
    send(response, {
      id: "fixture-tool",
      object: "chat.completion.chunk",
      created: 1,
      model: body.model,
      choices: [{
        index: 0,
        delta: {
          role: "assistant",
          tool_calls: [{
            index: 0,
            id: `fixture-tool-${messages.length}`,
            type: "function",
            function: {
              name: backgroundLaunch ? "subagent" : imageCall ? imageCall.name : canvasCall ? canvasCall.name : suggest ? "suggest_task" : browser ? browser.name : computer ? computer.name : commitCall ? commitCall.name : search ? "web_search" : "write",
              arguments: imageCall ? imageCall.arguments : canvasCall ? canvasCall.arguments : suggest
                ? JSON.stringify({
                  title: "Dismiss button stays disabled after a failed save",
                  kind: "bug",
                  description: "**Where:** `src/components/Composer.tsx:88`\n\nThe stop button stays disabled after a save fails, so the chat can never be stopped again. Re-enable it when the send settles, whatever the outcome."
                })
                : browser ? browser.arguments : computer ? computer.arguments : commitCall ? commitCall.arguments : JSON.stringify(backgroundLaunch
                ? { agent: roles.includes("worker") ? "worker" : roles[0], task: `background-fixture-child${prompt.includes("until stopped") ? " until stopped" : ""}`, background: true }
                : search ? { query: search, count: 3 }
                : escape ? { path: "../wackcode-escape.txt", content: "Written outside the workspace.\n" }
                : { path: "wackcode-live.txt", content: "Edited through WackCode and Pi.\n" })
            }
          }]
        },
        finish_reason: null
      }]
    });
    send(response, {
      id: "fixture-tool",
      object: "chat.completion.chunk",
      created: 1,
      model: body.model,
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      usage: { prompt_tokens: 30, completion_tokens: 12, total_tokens: 42 }
    });
  } else {
    const imageReply = imageFixture ? imageFixtureReply([latestUser, ...sinceUser]) : undefined;
    send(response, {
      id: "fixture-done",
      object: "chat.completion.chunk",
      created: 2,
      model: body.model,
      choices: [{ index: 0, delta: { role: "assistant", content: imageReply ?? (canvasFixture ? "Three themes for the billing page are on the canvas: **Base**, **Ember** and **Pine**, plus Base on a phone. Which direction should I take forward?" : suggestFixture ? "Layout fixed. I noted one follow-up below." : browserMatch ? "Browser fixture complete." : computerMatch ? "Computer fixture complete." : commitFixture && hasToolResult ? "Committed in the throwaway repo under /tmp. The transcript's commit row carries git's own summary: the hash in a pill and the message." : child ? "Background fixture child completed." : searchQuery ? "I searched the web; the results are in the card above." : resultsArrived ? "The background child finished; its result has arrived." : background ? "I launched a background helper and finished my own work. You can send another message while it runs." : escape ? "I tried to write `../wackcode-escape.txt`, outside the workspace." : "Created `wackcode-live.txt`.") }, finish_reason: null }]
    });
    send(response, {
      id: "fixture-done",
      object: "chat.completion.chunk",
      created: 2,
      model: body.model,
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: { prompt_tokens: 40, completion_tokens: 6, total_tokens: 46 }
    });
  }
  response.end("data: [DONE]\n\n");
});

server.listen(port, "127.0.0.1", () => {
  process.stdout.write(`WackCode mock provider listening at http://127.0.0.1:${server.address().port}/v1\n`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}

function imageFixtureReply(messages) {
  const references = new Set(messages.flatMap((message) =>
    textOf(message?.content).match(/wackcode-image:\/\/entry\/[A-Za-z0-9_-]{1,256}\/(?:0|[1-9]\d*)(?!\d)/g) ?? []
  ));
  if (!references.size) return "Attach an image, or use `image reply fixture: /absolute/path/image.png`.";
  const images = [...references].slice(0, 3).map((reference, index) =>
    `![Selected image ${index + 1}](${reference} "Image ${index + 1}")`
  );
  return `Here are the selected images.\n\n${images.join("\n\n")}\n\nOpen an image to inspect it.`;
}

/** The canvas fixture's one drawing call: a themable base, two variants and a phone artboard. */
function canvasFixtureArguments() {
  const html = `<style>
:root{--bg:#f4f4f1;--surface:#fff;--text:#1d1d1b;--muted:#6b6b66;--line:#e2e2dc;--accent:#2f6df6;--on-accent:#fff;--radius:10px}
*{box-sizing:border-box}body{margin:0;min-height:100vh;background:var(--bg);color:var(--text);font:14px/1.5 -apple-system,"SF Pro Text",system-ui,sans-serif}
.shell{display:grid;grid-template-columns:220px 1fr;min-height:100vh}
nav{padding:28px 18px;border-right:1px solid var(--line)}nav b{display:block;margin-bottom:24px;font-size:15px}
nav a{display:block;padding:7px 10px;border-radius:var(--radius);color:var(--muted);text-decoration:none}nav a.on{background:var(--surface);color:var(--text);font-weight:600}
main{padding:40px 48px;max-width:860px}h1{margin:0 0 4px;font-size:26px}p.lead{margin:0 0 28px;color:var(--muted)}
.card{background:var(--surface);border:1px solid var(--line);border-radius:var(--radius);padding:22px 24px;margin-bottom:18px}
.row{display:flex;align-items:center;justify-content:space-between;gap:16px}.plan{font-size:20px;font-weight:650}.muted{color:var(--muted)}
button{border:0;border-radius:calc(var(--radius) - 2px);padding:9px 16px;font:inherit;font-weight:600;background:var(--accent);color:var(--on-accent);cursor:pointer}
button.ghost{background:transparent;color:var(--text);border:1px solid var(--line)}
table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}td{padding:10px 0;border-top:1px solid var(--line)}td:last-child{text-align:right}
@media (max-width:600px){.shell{grid-template-columns:1fr}nav{display:none}main{padding:28px 20px}}
</style>
<div class="shell"><nav><b>Acme</b><a>Profile</a><a>Team</a><a class="on">Billing</a><a>Security</a></nav>
<main><h1>Billing</h1><p class="lead">Your plan, payment method and invoices.</p>
<div class="card row"><div><div class="muted">Current plan</div><div class="plan">Team · $48/month</div><div class="muted">Renews on 1 November · 6 of 10 seats used</div></div><div class="row"><button class="ghost">Change plan</button><button>Add seats</button></div></div>
<div class="card row"><div><div class="muted">Payment method</div><div>Visa ending 4242 · expires 08/28</div></div><button class="ghost">Update</button></div>
<div class="card"><div class="muted" style="margin-bottom:6px">Invoices</div><table><tr><td>October 2026</td><td>$48.00</td></tr><tr><td>September 2026</td><td>$48.00</td></tr><tr><td>August 2026</td><td>$40.00</td></tr></table></div>
</main></div>`;
  return {
    title: "Billing settings — themes",
    artboards: [
      { name: "Base", group: "Desktop", width: 1280, height: 800, notes: "Neutral reference", html },
      { name: "Ember", group: "Desktop", width: 1280, height: 800, basedOn: "base", notes: "Warm and high-contrast", css: ":root{--bg:#17110d;--surface:#221913;--text:#f6eee8;--muted:#b8a598;--line:#3a2c22;--accent:#ff7a3d;--on-accent:#1b0e05;--radius:6px}" },
      { name: "Pine", group: "Desktop", width: 1280, height: 800, basedOn: "base", notes: "Calm, rounded", css: ":root{--bg:#eef3ef;--surface:#fbfdfb;--text:#13241a;--muted:#5d7466;--line:#d6e2d9;--accent:#1f7a4d;--on-accent:#fff;--radius:16px}" },
      { name: "Base · Phone", group: "Phone", width: 390, height: 844, basedOn: "base" }
    ]
  };
}
