// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

import { findLabDesk, takeReport } from "./bridge.js";
const api = globalThis.browser ?? globalThis.chrome;

// One handler per kind of message. New features add an entry here.
const handlers = {
  "labdesk:ping": () => findLabDesk().then((f) => ({ found: !!f, version: f ? f.version : null })),
  "labdesk:take-report": (m) => takeReport(String(m.id || "")),
};

api.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  const h = msg && handlers[msg.type];
  if (!h) return false;
  Promise.resolve(h(msg)).then(sendResponse, (e) => sendResponse({ error: String(e && e.message || e) }));
  return true; // answer later
});
