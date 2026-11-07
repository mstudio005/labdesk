# LabDesk Bridge

A small browser extension for LabDesk. When you press **WHATSAPP** on a report, LabDesk opens the patient's chat in
WhatsApp Web and this extension attaches the report PDF. **You check it and press Send yourself.** The extension never
sends anything, never reads your chats and never talks to the internet. It only talks to LabDesk on the same computer
(127.0.0.1, ports 47653 to 47662).

## How it works

1. LabDesk saves the report, puts the PDF in a one-time slot and opens
   `https://web.whatsapp.com/send?phone=…&text=…&ld=<one-time code>`.
2. The extension (only on `web.whatsapp.com/send` pages that carry a valid `ld` code) waits for the chat to open,
   fetches the PDF from LabDesk with that code (it works once, within ten minutes) and pastes it into the message box.
3. If pasting does not work it tries the paper-clip menu. If that fails too, a red note says so, and the report is
   also on the Windows clipboard (press Ctrl+V).

Without the extension nothing breaks: the clipboard way still works.

## Build

```
python3 build.py
```

Makes `dist/chrome` (Chrome, Brave, Edge), `dist/firefox`, and three zips for the stores.

## Install

**Chrome / Brave** (not on a store): open `chrome://extensions` (Brave: `brave://extensions`), switch on *Developer mode*,
press *Load unpacked*, choose the `chrome` folder. Keep that folder in a place that will not be deleted.

**Edge**: upload `dist/labdesk-bridge-edge.zip` at <https://partner.microsoft.com/dashboard/microsoftedge> (free).
Until it is published: `edge://extensions` > *Developer mode* > *Load unpacked* > the `chrome` folder.

**Firefox**: upload `dist/labdesk-bridge-firefox.zip` at <https://addons.mozilla.org/developers/> (free, listed or
self-distributed, Mozilla signs it). To try it first: `about:debugging` > *This Firefox* > *Load Temporary Add-on* >
`dist/firefox/manifest.json`.

Click the extension's icon to see whether it can find LabDesk.

## Adding features later

All talking to LabDesk is in `src/bridge.js`; `src/background.js` has one handler per message type. A new feature
(for example "open this patient in LabDesk") is a new function in `bridge.js`, a new handler, and a new endpoint in
`crates/labdesk-desktop/src/bridge.rs`.
