// goat-zenoh-portal — Server-Sent Events read with fetch.
//
// Streaming uses fetch so callers can set Accept and cancel the request.
// The browser sends its same-origin HttpOnly session cookie. Credentials
// never go in the URL or browser storage.
//
// This covers the part of the SSE wire format the portal's stream writes
// (internal/zenohproxy StreamSSE): `event:` and `data:` fields, `:` comment
// lines for the heartbeat, and a blank line ending each event.

// parseEvents splits the complete events off the front of buffer. It returns
// them with the unterminated remainder, which the caller keeps and prepends
// to the next chunk.
export function parseEvents(buffer) {
  const blocks = buffer.split(/\r?\n\r?\n/);
  const rest = blocks.pop();
  const events = blocks.map(parseBlock).filter(Boolean);
  return { events, rest };
}

// parseBlock reads one event. A block with no data line (a heartbeat) is not
// an event, as in the SSE spec.
function parseBlock(block) {
  let event = "message";
  const data = [];
  for (const line of block.split(/\r?\n/)) {
    if (line === "" || line.startsWith(":")) continue;
    const colon = line.indexOf(":");
    const field = colon < 0 ? line : line.slice(0, colon);
    const value = colon < 0 ? "" : line.slice(colon + 1).replace(/^ /, "");
    if (field === "event") event = value;
    else if (field === "data") data.push(value);
  }
  return data.length ? { event, data: data.join("\n") } : null;
}

// openEventStream reads url as an event stream and calls onEvent for each
// event. It rejects with "HTTP <status>" when the server refuses the request,
// and resolves when the server ends the stream. Aborting signal stops it and
// rejects with an AbortError.
export async function openEventStream(url, { headers = {}, signal, onEvent }) {
  const res = await fetch(url, { headers, signal, cache: "no-store" });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  if (!res.body) throw new Error("the stream response has no body");
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    const parsed = parseEvents(buffer + decoder.decode(value, { stream: true }));
    buffer = parsed.rest;
    parsed.events.forEach(onEvent);
  }
}
