// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

// Talks to LabDesk on this computer (127.0.0.1 only). Everything LabDesk offers goes through here, so a future feature
// (for example "open this patient in LabDesk") is one more function in this file and one more message in background.js.
const api = globalThis.browser ?? globalThis.chrome;
const FIRST_PORT = 47653, PORT_COUNT = 10;
const HEADERS = { "X-LabDesk-Bridge": "1" };
let knownPort = null;

async function call(port, path, timeoutMs = 2500) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), timeoutMs);
  try { return await fetch(`http://127.0.0.1:${port}${path}`, { headers: HEADERS, signal: ctl.signal, cache: "no-store" }); }
  finally { clearTimeout(t); }
}

/** Find the port LabDesk listens on. Returns { port, version } or null when LabDesk is not running. */
export async function findLabDesk() {
  const ports = knownPort ? [knownPort] : [];
  for (let i = 0; i < PORT_COUNT; i++) if (FIRST_PORT + i !== knownPort) ports.push(FIRST_PORT + i);
  for (const port of ports) {
    try {
      const r = await call(port, "/hello", 800);
      if (!r.ok) continue;
      const j = await r.json();
      if (j && j.app === "LabDesk") { knownPort = port; return { port, version: j.version }; }
    } catch (_) { /* not this port */ }
  }
  knownPort = null;
  return null;
}

/** Fetch the one-time report. Returns { name, b64 } or null if LabDesk has no such report any more. */
export async function takeReport(id) {
  const found = await findLabDesk();
  if (!found) return { error: "LabDesk is not running on this computer." };
  const r = await call(found.port, `/take?id=${encodeURIComponent(id)}`, 15000);
  if (r.status === 404) return { error: "LabDesk no longer has this report. Press WHATSAPP in LabDesk again." };
  if (!r.ok) return { error: `LabDesk answered with an error (${r.status}).` };
  const name = decodeURIComponent(r.headers.get("X-File-Name") || "report.pdf");
  const bytes = new Uint8Array(await r.arrayBuffer());
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return { name, b64: btoa(bin) };
}
