// goat-zenoh-portal — the Inspector's live stream.
//
// Streams /api/topics/stream with the portal browser session.
// header (sse.js), and writes each sample into the stream log newest-first.

import { openEventStream } from "/sse.js";

let streamAbort = null;

// wireStream connects the Live stream button. decode renders one sample for
// the log; the Inspector passes in its format dispatch.
export function wireStream(decode) {
  document.getElementById("toggle-stream").addEventListener("click", () => toggleStream(decode));
}

function toggleStream(decode) {
  const btn = document.getElementById("toggle-stream");
  if (streamAbort) {
    streamAbort.abort();
    streamAbort = null;
    btn.textContent = "Live stream: off";
    return;
  }
  const key = document.getElementById("key-expr").value.trim();
  if (!key) return;
  const log = document.getElementById("stream-log");
  log.textContent = "";
  const ctrl = new AbortController();
  streamAbort = ctrl;
  btn.textContent = "Live stream: ON (click to stop)";
  const headers = { Accept: "text/event-stream" };
  openEventStream(`/api/topics/stream?key=${encodeURIComponent(key)}`, {
    headers,
    signal: ctrl.signal,
    onEvent: (ev) => onStreamEvent(ev, log, decode),
  })
    .then(() => { log.textContent = "# stream closed\n\n" + log.textContent; })
    .catch((e) => {
      if (e.name === "AbortError") return;
      const hint = e.message === "HTTP 401" ? " (check your API key)" : "";
      log.textContent = `# stream failed: ${e.message}${hint}\n\n` + log.textContent;
    })
    .finally(() => {
      // A stop followed by a fresh start has already replaced the controller.
      if (streamAbort !== ctrl) return;
      streamAbort = null;
      btn.textContent = "Live stream: off";
    });
}

async function onStreamEvent(ev, log, decode) {
  if (ev.event === "error") {
    log.textContent = `# stream error: ${ev.data}\n\n` + log.textContent;
    return;
  }
  if (ev.event !== "sample") return;
  try {
    const s = JSON.parse(ev.data);
    const fmt = (document.getElementById("format-override").value || "json");
    const decoded = await decode(s, fmt);
    log.textContent = `# ${s.key} @ ${s.timestamp}\n${decoded}\n\n` + log.textContent;
  } catch (e) {
    log.textContent = `# parse error: ${e.message}\n${ev.data}\n\n` + log.textContent;
  }
}
