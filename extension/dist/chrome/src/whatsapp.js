// Runs on web.whatsapp.com. It only acts when LabDesk opened the chat: the address then carries ?ld=<one-time code>.
// It attaches the report to the chat that is open and shows a note. It never presses Send and never reads messages.
(() => {
  const api = globalThis.browser ?? globalThis.chrome;
  const params = new URLSearchParams(location.search);
  const code = params.get("ld");
  if (!code || !/^[0-9a-f]{32}$/.test(code) || location.pathname !== "/send") return;

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const ask = (msg) => { try { return Promise.resolve(api.runtime.sendMessage(msg)).then((r) => r || { error: "No answer from the extension." }, (e) => ({ error: String(e) })); } catch (e) { return Promise.resolve({ error: String(e) }); } };

  function note(text, bad) {
    let el = document.getElementById("labdesk-bridge-note");
    if (!el) {
      el = document.createElement("div");
      el.id = "labdesk-bridge-note";
      el.style.cssText = "position:fixed;z-index:2147483647;left:50%;top:12px;transform:translateX(-50%);max-width:560px;padding:10px 16px;font:14px/1.4 system-ui,sans-serif;color:#fff;box-shadow:0 2px 10px rgba(0,0,0,.3);cursor:pointer";
      el.title = "Click to close";
      el.addEventListener("click", () => el.remove());
      (document.body || document.documentElement).appendChild(el);
    }
    el.style.background = bad ? "#b3261e" : "#1f7a4d";
    el.textContent = "LabDesk Bridge: " + text;
  }

  // The message box of the open chat.
  const composer = () =>
    document.querySelector('#main footer div[contenteditable="true"]') ||
    document.querySelector('footer div[contenteditable="true"][role="textbox"]') ||
    document.querySelector('#main div[contenteditable="true"][data-tab]');

  async function waitFor(fn, ms) {
    const end = Date.now() + ms;
    while (Date.now() < end) { const v = fn(); if (v) return v; await sleep(500); }
    return null;
  }

  function toFile(b64, name) {
    const bin = atob(b64), bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return new File([bytes], name, { type: "application/pdf" });
  }

  // Way 1: paste the file into the message box, as Ctrl+V would.
  function pasteFile(box, file) {
    try {
      const dt = new DataTransfer();
      dt.items.add(file);
      box.focus();
      const ev = new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true });
      if (!ev.clipboardData || ev.clipboardData.files.length === 0) return false; // this browser ignores clipboardData here
      box.dispatchEvent(ev);
      return true;
    } catch (_) { return false; }
  }

  // Way 2: use the paper-clip menu's hidden file chooser.
  async function attachWithMenu(file) {
    const clip = document.querySelector('[data-icon="plus-rounded"], [data-icon="attach-menu-plus"], [data-icon="clip"], button[aria-label="Attach"]');
    const btn = clip && (clip.closest("button") || clip.closest('[role="button"]') || clip);
    if (!btn) return false;
    btn.click();
    await sleep(600);
    const inputs = [...document.querySelectorAll('input[type="file"]')];
    const input = inputs.find((i) => !i.accept || i.accept === "*" || i.accept.includes("application")) || inputs[0];
    if (!input) return false;
    const dt = new DataTransfer();
    dt.items.add(file);
    input.files = dt.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  }

  (async () => {
    note("waiting for the chat to open… (scan the QR code first if WhatsApp asks)");
    const box = await waitFor(composer, 5 * 60 * 1000);
    if (!box) return note("the chat did not open, so nothing was attached. Press WHATSAPP in LabDesk again.", true);
    await sleep(800); // let the chat finish loading
    const got = await ask({ type: "labdesk:take-report", id: code });
    if (!got || got.error || !got.b64) return note((got && got.error) || "could not get the report from LabDesk.", true);
    const file = toFile(got.b64, got.name);
    // Attach exactly once. The paper-clip way is used only when this browser cannot paste a file at all;
    // trying a second way after a paste that already worked would attach the report twice.
    let done = pasteFile(box, file);
    if (!done) done = await attachWithMenu(file);
    if (done) note("the report is attached (" + got.name + "). Check the name and press Send yourself.");
    else note("could not attach by itself. The report is on your clipboard: click the message box and press Ctrl+V.", true);
  })();
})();
