/**
 * Provider failures arrive as raw HTTP text. A 413 from a gateway is an nginx HTML page inside a
 * JSON wrapper — accurate, but it tells the user nothing they can act on, and it looks like a
 * WackCode crash rather than a body the endpoint refused.
 *
 * Only errors with a known, actionable cause are rewritten. Everything else is returned as it
 * came, so an unfamiliar failure keeps its original detail.
 */
const TOO_LARGE = /^(?:HTTP\s+)?413\b|Request Entity Too Large|request_too_large|Payload Too Large/i;

export function describeProviderError(message: string): string {
  if (!TOO_LARGE.test(message.trim())) return message;
  return "This chat is too large for this endpoint, so the provider rejected the request (HTTP 413). Start a new chat, or compact this one, then send again. Removing large attachments also helps.";
}
