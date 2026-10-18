import { describe, expect, it } from "vitest";
import { describeProviderError } from "./provider-error.js";

const nginx413 = "413: {\"message\":\"<html>\\r\\n<head><title>413 Request Entity Too Large</title></head>\\r\\n<body>\\r\\n<center><h1>413 Request Entity Too Large</h1></center>\\r\\n<hr><center>nginx</center>\\r\\n</body>\\r\\n</html>\",\"type\":\"invalid_request_error\"}";

describe("provider errors", () => {
  it("turns a gateway's 413 into something the user can act on", () => {
    const friendly = describeProviderError(nginx413);
    expect(friendly).toContain("too large");
    expect(friendly).toContain("HTTP 413");
    expect(friendly).toContain("Start a new chat");
    expect(friendly).not.toContain("<html>");
    expect(describeProviderError("413 Request Entity Too Large")).toBe(friendly);
    expect(describeProviderError("HTTP 413: request rejected")).toBe(friendly);
    expect(describeProviderError("413")).toBe(friendly);
  });

  it("keeps every unfamiliar failure exactly as it came", () => {
    expect(describeProviderError("400: invalid request: max_tokens is too large")).toBe("400: invalid request: max_tokens is too large");
    expect(describeProviderError("Request failed after 413ms")).toBe("Request failed after 413ms");
    expect(describeProviderError("maximum_context_length")).toBe("maximum_context_length");
  });
});
