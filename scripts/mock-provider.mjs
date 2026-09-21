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
  const hasToolResult = messages.at(-1)?.role === "tool";
  if (!hasToolResult) {
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
            id: "fixture-write",
            type: "function",
            function: {
              name: "write",
              arguments: JSON.stringify({ path: "wackcode-live.txt", content: "Edited through WackCode and Pi.\n" })
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
      choices: [{ index: 0, delta: { role: "assistant", content: "Created `wackcode-live.txt`." }, finish_reason: null }]
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
  process.stdout.write(`WackCode mock provider listening at http://127.0.0.1:${port}/v1\n`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
