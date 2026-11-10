// SPDX-License-Identifier: Apache-2.0
// api.js — the smoketest server's JSON API (ops/smoketest/internal/server).

async function fetchJSON(url, init) {
  const res = await fetch(url, init);
  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    const err = new Error(`${res.status}: ${text}`);
    err.status = res.status;
    throw err;
  }
  return res.json();
}

// The server reads the mode from the query string.
export function triggerRun(mode) {
  return fetchJSON(`/api/run?mode=${encodeURIComponent(mode)}`, { method: "POST" });
}

export const getRunStatus = () => fetchJSON("/api/run/status");
export const getResults = (limit = 20) => fetchJSON(`/api/results?limit=${limit}`);
export const getLatestResult = () => fetchJSON("/api/results/latest");
export const getPeers = () => fetchJSON("/api/peers");
export const getServices = () => fetchJSON("/api/services");
export const getCatalogue = () => fetchJSON("/api/catalogue");
