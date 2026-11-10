import { shell } from "./shell.js";
import { setNotice } from "./app-kit.js";

const element = (id) => {
  const node = document.getElementById(id);
  if (!node) throw new Error("Missing interface element");
  return node;
};
let token = "";
let nextPage = null;
let nextLocationPage = null;
let loading = false;
let syncState = null;
let listController = null;
let detailGeneration = 0;
let syncController = null;
const objectUrls = new Set();
const base = new URL(document.baseURI);
const formatDate = (value) => value ? new Date(value).toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) : "Not available";
const scanLabel = (status) => ({ none: "Not requested", processing: "Scanning", completed: "Scan ready" })[status] || "Status unavailable";
const plural = (count, singular, multiple = `${singular}s`) => `${count} ${count === 1 ? singular : multiple}`;
const storageDue = (item) => item.clear_at && Date.parse(item.clear_at) <= Date.now() + 72 * 60 * 60 * 1000;
const syncProblem = (code) => ({
  location_required: "Choose a mailbox to start syncing.",
  provider_authentication_required: "Check the Stable connection settings.",
  provider_rate_limited: "Stable is limiting requests. Sync will retry later.",
  provider_location_mismatch: "Stable returned mail for another location. That page was not saved.",
})[code] || "Sync could not finish. Existing mail is still available.";
function notice(text) { setNotice(element("notice"), text); }
function showError(text, target = "error") { element(target).textContent = text; element(target).hidden = !text; }
function expiredSession() {
  token = "";
  if (element("detail").open) element("detail").close();
  element("detail-content").replaceChildren();
  element("items").replaceChildren();
  element("app").hidden = true; element("app-footer").hidden = true; element("refresh").hidden = true;
  element("login").hidden = base.pathname !== "/";
  notice(base.pathname === "/" ? "Connect to view mail." : "The Home Assistant session expired. Reload Home Assistant to reconnect.");
  if (!element("login").hidden) element("token").focus();
}
async function request(path, options = {}) {
  const headers = new Headers(options.headers);
  if (token) headers.set("Authorization", `Bearer ${token}`);
  headers.set("X-Mail-Request", "1");
  if (options.body) headers.set("Content-Type", "application/json");
  const response = await fetch(new URL(path, base), { ...options, headers, cache: "no-store", redirect: "error" });
  if (!response.ok) {
    if (response.status === 401) expiredSession();
    const error = new Error(response.status === 401 ? "Reconnect to continue." : response.status === 403 ? "Administrator access is required." : response.status === 409 ? "A sync is already running, or this mailbox is already configured." : "Mail is unavailable. Try again shortly.");
    error.status = response.status;
    throw error;
  }
  return response;
}
async function json(path, options) { return (await request(path, options)).json(); }
async function action(task, target = "error") {
  showError("", target);
  try { await task(); } catch (error) { showError(error.message || "The request could not be completed.", target); }
}
function setView(settings) {
  element("mail-view").hidden = settings;
  element("settings-view").hidden = !settings;
  element("mail-tab").setAttribute("aria-pressed", String(!settings));
  element("settings-tab").setAttribute("aria-pressed", String(settings));
}
function text(tag, content, className) {
  const node = document.createElement(tag); node.textContent = content;
  if (className) node.className = className;
  return node;
}
function releaseDocuments() {
  for (const url of objectUrls) URL.revokeObjectURL(url);
  objectUrls.clear();
}
async function documentBlob(path) {
  const blob = await (await request(path)).blob();
  const url = URL.createObjectURL(blob); objectUrls.add(url); return url;
}
async function showDetail(id) {
  const generation = ++detailGeneration;
  releaseDocuments();
  showError("", "detail-error");
  const item = await json(`v1/items/${encodeURIComponent(id)}`);
  if (generation !== detailGeneration) return;
  element("detail-title").textContent = item.sender || "Mail item";
  const content = element("detail-content"); content.replaceChildren();
  content.append(text("p", scanLabel(item.scan_status), "muted"));
  content.append(text("p", `Received: ${item.received_at ? formatDate(item.received_at) : "Not supplied by Stable"}`));
  if (!item.received_at) content.append(text("p", `First seen: ${formatDate(item.first_seen_at * 1000)}`, "muted"));
  if (item.shred_status === "completed") content.append(text("p", "Physical mail shredded"));
  if (item.clear_at && item.shred_status !== "completed") content.append(text("p", `Storage deadline: ${formatDate(item.clear_at)}`, storageDue(item) ? "deadline" : "muted"));
  if (item.scan_notice) content.append(text("p", "This scan needs attention. Check its scan notice in Stable.", "scan-notice"));
  content.append(text("h3", "Summary"));
  if (item.summary) content.append(text("p", item.summary, "summary"));
  else content.append(text("p", "No summary available", "muted"));
  if (item.has_envelope) {
    const image = document.createElement("img"); image.alt = "Envelope scan"; image.className = "envelope"; content.append(image);
    documentBlob(`v1/items/${encodeURIComponent(id)}/envelope`).then((url) => {
      if (generation !== detailGeneration) { URL.revokeObjectURL(url); objectUrls.delete(url); return; }
      image.src = url;
    }).catch(() => { if (generation === detailGeneration) image.replaceWith(text("p", "Envelope image is unavailable.", "muted")); });
  }
  if (item.has_pdf) {
    const button = text("button", "Download scan PDF"); button.type = "button";
    button.addEventListener("click", () => action(async () => {
      button.disabled = true;
      try {
        const url = await documentBlob(`v1/items/${encodeURIComponent(id)}/pdf`);
        const link = document.createElement("a"); link.href = url; link.download = "mail-scan.pdf"; link.click();
      } finally { button.disabled = false; }
    }, "detail-error")); content.append(button);
  }
  if (item.has_ocr) {
    const button = text("button", "Read OCR text", "quiet"); button.type = "button";
    button.addEventListener("click", () => action(async () => {
      button.disabled = true;
      try {
        const result = await json(`v1/items/${encodeURIComponent(id)}/ocr`);
        if (generation === detailGeneration) {
          const heading = text("h3", "OCR text"); heading.tabIndex = -1;
          content.append(heading, text("pre", result.pages.join("\n\n"), "ocr"));
          heading.focus(); button.remove();
        }
      } finally { button.disabled = false; }
    }, "detail-error")); content.append(button);
  }
  if (!element("detail").open) element("detail").showModal();
}
function renderItem(item) {
  const card = text("article", "", "mail-card");
  const button = text("button", item.sender || "Unknown sender", "item-open"); button.type = "button";
  button.addEventListener("click", () => action(async () => {
    if (button.getAttribute("aria-busy") === "true") return;
    button.setAttribute("aria-busy", "true");
    try { await showDetail(item.id); }
    finally { button.removeAttribute("aria-busy"); }
  }));
  if (!item.read_at) { card.classList.add("unread"); card.append(text("span", "Unread", "unread-label")); }
  card.append(button, text("p", item.summary || "No summary available", "muted"));
  card.append(text("p", `${scanLabel(item.scan_status)}${item.scan_notice ? " · Needs attention" : ""}`, "status"));
  card.append(text("p", item.received_at ? `Received ${formatDate(item.received_at)}` : `First seen ${formatDate(item.first_seen_at * 1000)}`, "date"));
  if (item.clear_at && item.shred_status !== "completed") {
    const urgent = storageDue(item);
    const label = urgent ? "Storage deadline" : "Stored until";
    if (urgent) card.classList.add("storage-due");
    card.append(text("p", `${label}: ${formatDate(item.clear_at)}`, urgent ? "deadline" : "date"));
  }
  return card;
}
async function loadMail(append = false) {
  if (append && loading) return;
  listController?.abort();
  const controller = new AbortController();
  listController = controller;
  loading = true; element("load-more").disabled = true;
  try {
    element("items").setAttribute("aria-busy", "true");
    const query = new URLSearchParams({ limit: "50", archived: String(element("archived").checked) });
    if (element("search").value) query.set("query", element("search").value);
    if (element("scan-status").value) query.set("status", element("scan-status").value);
    if (element("unread").checked) query.set("unread", "true");
    if (append && nextPage) query.set("page_token", nextPage);
    const result = await json(`v1/items?${query}`, { signal: controller.signal });
    if (listController !== controller) return;
    if (!append) element("items").replaceChildren();
    for (const item of result.items) element("items").append(renderItem(item));
    nextPage = result.next_page_token;
    element("load-more").hidden = !nextPage;
    element("empty").hidden = element("items").children.length !== 0 || !syncState?.initial_sync_complete;
    notice(syncState?.error_code ? "Sync is incomplete. Showing last known mail." : !syncState?.initial_sync_complete ? "Waiting for the first complete sync. Mail shown so far may be incomplete." : "");
  } catch (error) {
    if (error.name !== "AbortError") {
      if (error.status !== 401) notice("Mail could not be refreshed. Showing the last loaded results.");
      throw error;
    }
  } finally {
    if (listController === controller) { loading = false; element("load-more").disabled = false; element("items").setAttribute("aria-busy", "false"); }
  }
}
async function loadLocations() {
  const query = nextLocationPage ? `?page_token=${encodeURIComponent(nextLocationPage)}` : "";
  const result = await json(`v1/locations${query}`);
  for (const location of result.items) {
    const option = text("option", [location.address.line1, location.address.line2, location.address.city, location.address.state, location.address.postalCode, location.onboarding?.status].filter(Boolean).join(", "));
    option.value = location.id; element("location").append(option);
  }
  nextLocationPage = result.next_page_token; element("more-locations").hidden = !nextLocationPage;
}
async function loadSettings(system) {
  element("environment").textContent = system.environment === "sandbox" ? "Sandbox" : "Production";
  element("binding").textContent = system.binding ? "Mailbox configured" : "Choose the mailbox for this deployment.";
  if (system.binding) {
    json("v1/installation/location").then((location) => {
      element("binding").textContent = [location.address.line1, location.address.line2, location.address.city, location.address.state, location.address.postalCode].filter(Boolean).join(", ");
    }).catch(() => {});
  }
  element("location-form").hidden = Boolean(system.binding);
  element("sync").disabled = !system.binding || syncController !== null;
  if (!system.binding && element("location").options.length === 1) await loadLocations();
  const status = await json("v1/sync");
  syncState = status;
  element("sync-status").textContent = status.error_code ? syncProblem(status.error_code) : status.last_success_at ? `Last complete sync: ${formatDate(status.last_success_at * 1000)}` : "The initial sync has not completed.";
  shell.setStatus(status.error_code ? "Sync needs attention" : status.initial_sync_complete ? "Connected" : "Waiting for initial sync", status.error_code ? "error" : status.initial_sync_complete ? "ok" : "stale");
}
async function refresh() {
  const system = await json("v1/system");
  element("login").hidden = true; element("app").hidden = false; element("refresh").hidden = false; element("app-footer").hidden = false;
  shell.setVersion(system.version);
  await loadSettings(system);
  if (!system.binding) setView(true);
  const summary = await json("v1/summary");
  element("counts").textContent = `${plural(summary.unread, "unread item")} · ${plural(summary.pending_scans, "scan")} pending · ${plural(summary.ready_scans, "scan")} ready${summary.storage_due ? ` · ${plural(summary.storage_due, "storage deadline")} soon (including archived)` : ""}`;
  await loadMail();
}
element("login").addEventListener("submit", (event) => { event.preventDefault(); token = element("token").value; element("token").value = ""; action(refresh); });
element("filters").addEventListener("submit", (event) => { event.preventDefault(); action(() => loadMail()); });
element("refresh").addEventListener("click", () => action(refresh));
element("load-more").addEventListener("click", () => action(() => loadMail(true)));
element("mail-tab").addEventListener("click", () => setView(false));
element("settings-tab").addEventListener("click", () => setView(true));
element("more-locations").addEventListener("click", () => action(loadLocations));
element("location-form").addEventListener("submit", (event) => { event.preventDefault(); action(async () => {
  await json("v1/installation/location", { method: "POST", body: JSON.stringify({ location_id: element("location").value }) });
  await refresh();
}); });
element("sync").addEventListener("click", () => action(async () => {
  element("sync").disabled = true;
  syncController?.abort();
  const controller = new AbortController(); syncController = controller;
  try {
    const operation = await json("v1/sync", { method: "POST", body: JSON.stringify({ idempotency_key: crypto.randomUUID() }), signal: controller.signal });
    notice("Sync queued.");
    for (let attempt = 0; attempt < 60 && !controller.signal.aborted; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 2000));
      if (controller.signal.aborted) return;
      const current = await json(`v1/operations/${operation.id}`, { signal: controller.signal });
      if (current.state === "completed" || current.state === "failed") {
        await refresh(); notice(current.state === "completed" ? "Mail synced." : "Sync could not finish. Check the connection status."); return;
      }
    }
    if (!controller.signal.aborted) notice("Sync is still running. Refresh to check its progress.");
  } catch (error) { if (error.name !== "AbortError") throw error; }
  finally { if (syncController === controller) { syncController = null; element("sync").disabled = false; } }
}));
element("close-detail").addEventListener("click", () => element("detail").close());
element("detail").addEventListener("close", () => { detailGeneration += 1; releaseDocuments(); });
document.querySelector(".wordmark").addEventListener("click", (event) => {
  if (event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) { event.preventDefault(); setView(false); }
});
window.addEventListener("pagehide", () => { listController?.abort(); syncController?.abort(); releaseDocuments(); });
refresh().catch((error) => {
  if (error.status !== 401) showError(error.message || "Mail is unavailable.");
});
