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
  const latestUser = [...messages].reverse().find((message) => message?.role === "user");
  response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache", connection: "keep-alive" });
  if (textOf(latestUser?.content).toLowerCase().includes("wait until stopped")) {
    response.write(": waiting for cancellation\n\n");
    process.stdout.write("slow request started\n");
    return;
  }
  // A long, tool-free turn for dynamic backgrounds and streaming/scroll performance checks.
  // Stays entirely on localhost and can be stopped at any time from either area.
  if (textOf(latestUser?.content).trim().toLowerCase() === "dynamic background fixture") {
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
  const hasToolResult = messages.at(-1)?.role === "tool";
  const prompt = textOf(latestUser?.content);
  const background = prompt.toLowerCase().includes("background fixture");
  // Chat mode's confinement check: the write aims one folder above the workspace.
  const escape = prompt.toLowerCase().includes("escape fixture");
  const child = prompt.startsWith("background-fixture-child");
  // "search fixture: <query>" calls web_search when Settings › Web offers it.
  const searchQuery = /^search fixture:\s*(.+)$/i.exec(prompt)?.[1];
  const search = searchQuery && body.tools?.some((tool) => tool.function?.name === "web_search") ? searchQuery : undefined;
  // Explicit browser fixtures drive the real host through the app without a paid model.
  const browserMatch = /^browser fixture:[ \t]*(browser_(?:open|snapshot|act|screenshot|console))[ \t]*(\{[^\n]*\})?(?:\n|$)/.exec(prompt);
  const browser = browserMatch && body.tools?.some((tool) => tool.function?.name === browserMatch[1])
    ? { name: browserMatch[1], arguments: browserMatch[2] || "{}" } : undefined;
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
  if (backgroundLaunch || (!background && !child && !resultsArrived && !hasToolResult && body.tools?.length && (!browserMatch || browser))) {
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
              name: backgroundLaunch ? "subagent" : browser ? browser.name : search ? "web_search" : "write",
              arguments: browser ? browser.arguments : JSON.stringify(backgroundLaunch
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
    send(response, {
      id: "fixture-done",
      object: "chat.completion.chunk",
      created: 2,
      model: body.model,
      choices: [{ index: 0, delta: { role: "assistant", content: browserMatch ? "Browser fixture complete." : child ? "Background fixture child completed." : searchQuery ? "I searched the web; the results are in the card above." : resultsArrived ? "The background child finished; its result has arrived." : background ? "I launched a background helper and finished my own work. You can send another message while it runs." : escape ? "I tried to write `../wackcode-escape.txt`, outside the workspace." : "Created `wackcode-live.txt`." }, finish_reason: null }]
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
