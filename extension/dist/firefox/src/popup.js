const api = globalThis.browser ?? globalThis.chrome;
const s = document.getElementById("s");
const show = (r) => {
  if (r && r.found) { s.className = "ok"; s.textContent = "LabDesk is running" + (r.version ? " (version " + r.version + ")" : "") + "."; }
  else { s.className = "no"; s.textContent = "LabDesk is not running. Open LabDesk on this computer."; }
};
Promise.resolve(api.runtime.sendMessage({ type: "labdesk:ping" })).then(show, () => show(null));
