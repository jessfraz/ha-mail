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
let systemState = null;
let pendingOperation = null;
let manualFailure = null;
let active = false;
let closed = false;
let listController = null;
let detailController = null;
let detailGeneration = 0;
let detailOpener = null;
let refreshController = null;
let refreshPromise = null;
let refreshTimer = null;
let operationController = null;
let bindingController = null;
let locationController = null;
let locationsRequested = false;
let bindingLocation = null;
let loadedPages = 1;
let appliedFilters = null;
const itemCards = new Map();
const objectUrls = new Set();
const base = new URL(document.baseURI);
const formatDate = (value) => value ? new Date(value).toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) : "Not available";
const scanLabel = (status) => ({ none: "Not requested", processing: "Scanning", completed: "Scan ready" })[status] || "Status unavailable";
const plural = (count, singular, multiple = `${singular}s`) => `${count} ${count === 1 ? singular : multiple}`;
const syncProblem = (code) => ({
  location_required: "Choose a mailbox to start syncing.",
  provider_authentication_required: "Check the Stable connection settings.",
  provider_rate_limited: "Stable is limiting requests. Sync will retry later.",
  provider_location_mismatch: "Stable returned mail for another location. That page was not saved.",
})[code] || "Sync could not finish. Saved mail is still available.";
function notice(value) { setNotice(element("notice"), value); }
function showError(value, target = "error") { element(target).textContent = value; element(target).hidden = !value; }
function stopRefresh() {
  clearTimeout(refreshTimer);
  refreshController?.abort();
  listController?.abort();
  bindingController?.abort();
  locationController?.abort();
}
function expiredSession() {
  active = false;
  stopRefresh();
  operationController?.abort();
  token = "";
  detailGeneration += 1;
  detailController?.abort();
  releaseDocuments();
  if (element("detail").open) element("detail").close();
  element("detail-content").replaceChildren();
  element("items").replaceChildren();
  itemCards.clear();
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
  try { await task(); } catch (error) {
    if (error.name !== "AbortError") showError(error.message || "The request could not be completed.", target);
  }
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
function storageState(item) {
  if (!item.clear_at || item.shred_status === "completed") return null;
  const deadline = Date.parse(item.clear_at);
  if (!Number.isFinite(deadline)) return null;
  return deadline <= Date.now() ? "past" : deadline <= Date.now() + 72 * 60 * 60 * 1000 ? "soon" : "stored";
}
function storageLabel(item) {
  return { past: "Past storage date", soon: "Storage ends soon", stored: "Storage until" }[storageState(item)];
}
function releaseDocuments() {
  for (const url of objectUrls) URL.revokeObjectURL(url);
  objectUrls.clear();
}
async function documentBlob(path, signal) {
  const blob = await (await request(path, { signal })).blob();
  if (signal.aborted) throw new DOMException("Details closed", "AbortError");
  const url = URL.createObjectURL(blob); objectUrls.add(url); return url;
}
async function showDetail(id) {
  const generation = ++detailGeneration;
  detailController?.abort();
  const controller = new AbortController();
  detailController = controller;
  releaseDocuments();
  showError("", "detail-error");
  const item = await json(`v1/items/${encodeURIComponent(id)}`, { signal: controller.signal });
  if (generation !== detailGeneration) return;
  element("detail-title").textContent = item.sender || "Mail item";
  const content = element("detail-content"); content.replaceChildren();
  const badges = text("div", "", "item-badges");
  if (!item.read_at) badges.append(text("span", "Unread", "unread-label"));
  badges.append(text("span", scanLabel(item.scan_status), "badge"));
  if (item.shred_status === "completed") badges.append(text("span", "Shredded", "badge"));
  content.append(badges);
  content.append(text("p", `Received ${item.received_at ? formatDate(item.received_at) : "date not supplied by Stable"}`, "muted"));
  if (!item.received_at) content.append(text("p", `First seen ${formatDate(item.first_seen_at * 1000)}`, "muted"));
  const storage = storageState(item);
  if (storage) {
    const storageNote = text("div", "", `storage-note ${storage === "soon" ? "attention" : ""}`);
    storageNote.append(text("strong", storageLabel(item)), text("p", formatDate(item.clear_at), "storage-time"));
    if (storage === "past") storageNote.append(text("p", "The date has passed. Stable has not reported this item shredded. Check its current status in Stable."));
    if (storage === "soon") storageNote.append(text("p", "Review this item in Stable before the storage period ends."));
    content.append(storageNote);
  }
  if (item.scan_notice) content.append(text("p", "This scan needs attention. Check its scan notice in Stable.", "scan-notice"));
  if (item.summary) content.append(text("h3", "Summary"), text("p", item.summary, "summary"));
  if (item.has_envelope) {
    const image = document.createElement("img"); image.alt = "Envelope scan"; image.className = "envelope"; content.append(image);
    documentBlob(`v1/items/${encodeURIComponent(id)}/envelope`, controller.signal).then((url) => {
      if (generation !== detailGeneration) { URL.revokeObjectURL(url); objectUrls.delete(url); return; }
      image.src = url;
    }).catch((error) => { if (error.name !== "AbortError" && generation === detailGeneration) image.replaceWith(text("p", "Envelope image is unavailable.", "muted")); });
  }
  const actions = text("div", "", "document-actions");
  if (item.has_pdf) {
    const button = text("button", "Download scan PDF"); button.type = "button";
    button.addEventListener("click", () => action(async () => {
      button.disabled = true;
      try {
        const url = await documentBlob(`v1/items/${encodeURIComponent(id)}/pdf`, controller.signal);
        if (generation !== detailGeneration) return;
        const link = document.createElement("a"); link.href = url; link.download = "mail-scan.pdf"; link.click();
      } finally { button.disabled = false; }
    }, "detail-error")); actions.append(button);
  }
  if (item.has_ocr) {
    const button = text("button", "Read OCR text", "quiet"); button.type = "button";
    button.addEventListener("click", () => action(async () => {
      button.disabled = true;
      try {
        const result = await json(`v1/items/${encodeURIComponent(id)}/ocr`, { signal: controller.signal });
        if (generation === detailGeneration) {
          const heading = text("h3", "OCR text"); heading.tabIndex = -1;
          content.append(heading, text("pre", result.pages.join("\n\n"), "ocr"));
          heading.focus(); button.remove();
        }
      } finally { button.disabled = false; }
    }, "detail-error")); actions.append(button);
  }
  if (actions.children.length) content.append(actions);
  if (!element("detail").open) element("detail").showModal();
}
function updateItemCard(card, item) {
  const storage = storageState(item);
  card.className = `mail-card${!item.read_at ? " unread" : ""}${storage === "soon" ? " storage-due" : ""}`;
  const button = card.querySelector("button");
  button.textContent = item.sender || "Unknown sender";
  const badges = text("div", "", "item-badges");
  if (!item.read_at) badges.append(text("span", "Unread", "unread-label"));
  if (item.scan_status !== "none") badges.append(text("span", scanLabel(item.scan_status), "badge"));
  if (item.scan_notice) badges.append(text("span", "Scan needs attention", "badge attention"));
  if (item.shred_status === "completed") badges.append(text("span", "Shredded", "badge"));
  const body = card.querySelector(".item-content");
  body.replaceChildren(badges);
  if (item.summary) body.append(text("p", item.summary, "item-summary"));
  body.append(text("p", item.received_at ? `Received ${formatDate(item.received_at)}` : `First seen ${formatDate(item.first_seen_at * 1000)}`, "date"));
  if (storage) body.append(text("p", `${storageLabel(item)} · ${formatDate(item.clear_at)}`, storage === "soon" ? "deadline" : "date storage-date"));
}
function renderItem(item) {
  const card = text("article", "", "mail-card");
  const button = text("button", "", "item-open"); button.type = "button";
  button.addEventListener("click", () => action(async () => {
    if (button.getAttribute("aria-busy") === "true") return;
    button.setAttribute("aria-busy", "true");
    detailOpener = button;
    try { await showDetail(item.id); }
    finally { button.removeAttribute("aria-busy"); }
  }));
  card.append(button, text("div", "", "item-content"));
  updateItemCard(card, item);
  return card;
}
function readFilters() {
  const query = new URLSearchParams({ archived: String(element("archived").checked) });
  if (element("search").value) query.set("query", element("search").value);
  if (element("scan-status").value) query.set("status", element("scan-status").value);
  if (element("unread").checked) query.set("unread", "true");
  return query.toString();
}
function renderEmpty() {
  const empty = element("empty");
  empty.hidden = itemCards.size !== 0;
  const waiting = !syncState?.initial_sync_complete && itemCards.size === 0;
  element("counts").hidden = waiting;
  element("filters").hidden = waiting;
  const title = !systemState?.binding ? "Choose your mailbox" : !syncState?.initial_sync_complete ? "Getting your mail" : "No mail here";
  const description = !systemState?.binding ? "Choose a Stable location in Settings to get started." : !syncState?.initial_sync_complete ? "Mail will appear automatically as the first sync completes." : "No mail matches these filters.";
  empty.replaceChildren(text("h2", title), text("p", description, "muted"));
}
async function loadMail(append = false, background = false) {
  if (loading && (append || background)) return;
  listController?.abort();
  const controller = new AbortController();
  listController = controller;
  const timeout = setTimeout(() => controller.abort(new DOMException("Mail could not be refreshed.", "TimeoutError")), 20000);
  loading = true; element("load-more").disabled = true;
  const filters = appliedFilters ?? readFilters();
  appliedFilters ??= filters;
  const pages = append ? 1 : background ? loadedPages : 1;
  try {
    if (!background) element("items").setAttribute("aria-busy", "true");
    let cursor = append ? nextPage : null;
    const incoming = [];
    for (let page = 0; page < pages; page += 1) {
      const query = new URLSearchParams(filters); query.set("limit", "50");
      if (cursor) query.set("page_token", cursor);
      const result = await json(`v1/items?${query}`, { signal: controller.signal });
      incoming.push(...result.items); cursor = result.next_page_token;
      if (!cursor) break;
    }
    if (listController !== controller || controller.signal.aborted || !active) return;
    let removedFocus = false;
    if (!append) {
      const keep = new Set(incoming.map((item) => item.id));
      for (const [id, { card }] of itemCards) if (!keep.has(id)) {
        removedFocus ||= card.contains(document.activeElement);
        card.remove(); itemCards.delete(id);
      }
    }
    for (const [index, item] of incoming.entries()) {
      let entry = itemCards.get(item.id);
      if (!entry) { entry = { card: renderItem(item) }; itemCards.set(item.id, entry); }
      else updateItemCard(entry.card, item);
      // Existing card buttons retain focus and dialog return targets across reads.
      const container = element("items");
      if (append) container.append(entry.card);
      else if (container.children[index] !== entry.card) container.insertBefore(entry.card, container.children[index] ?? null);
    }
    nextPage = cursor;
    if (append) loadedPages += 1;
    else if (!background) loadedPages = 1;
    element("load-more").hidden = !nextPage;
    renderEmpty();
    if (background && removedFocus && !element("detail").open) element("mail-heading").focus();
  } catch (error) {
    if (error.name !== "AbortError") {
      if (error.status !== 401) notice("Mail could not be refreshed. Showing saved results.");
      throw error;
    }
  } finally {
    clearTimeout(timeout);
    if (listController === controller) { loading = false; element("load-more").disabled = false; element("items").setAttribute("aria-busy", "false"); }
  }
}
async function loadLocations(ownerSignal) {
  if (locationController) return;
  const controller = new AbortController(); locationController = controller;
  const signal = ownerSignal ? AbortSignal.any([ownerSignal, controller.signal]) : controller.signal;
  const timeout = setTimeout(() => controller.abort(new DOMException("Mailboxes could not be loaded. Refresh to try again.", "TimeoutError")), 20000);
  locationsRequested = true;
  element("more-locations").disabled = true;
  try {
    const query = nextLocationPage ? `?page_token=${encodeURIComponent(nextLocationPage)}` : "";
    const result = await json(`v1/locations${query}`, { signal });
    if (signal.aborted || !active || closed || systemState?.binding) return;
    for (const location of result.items) {
      const option = text("option", [location.address.line1, location.address.line2, location.address.city, location.address.state, location.address.postalCode, location.onboarding?.status].filter(Boolean).join(", "));
      option.value = location.id; element("location").append(option);
    }
    nextLocationPage = result.next_page_token; element("more-locations").hidden = !nextLocationPage;
  } finally {
    clearTimeout(timeout);
    if (locationController === controller) { locationController = null; element("more-locations").disabled = false; }
  }
}
function renderSyncStatus() {
  if (!syncState) return;
  if (manualFailure && syncState.last_success_at !== manualFailure.lastSuccess) manualFailure = null;
  const failure = syncState.error_code || manualFailure?.code;
  const state = pendingOperation ? pendingOperation.state === "running" ? "Syncing" : "Sync queued" : failure ? "Sync needs attention" : !systemState?.binding ? "Choose mailbox" : syncState.initial_sync_complete ? "Up to date" : "First sync";
  const kind = failure ? "attention" : !pendingOperation && syncState.initial_sync_complete ? "good" : "";
  for (const id of ["mail-sync-status", "sync-status"]) {
    const status = element(id);
    status.replaceChildren(text("span", state, `badge ${kind}`));
    if (failure) status.append(text("span", syncProblem(failure), "sync-description"));
    if (syncState.last_success_at) status.append(text("span", `Updated ${formatDate(syncState.last_success_at * 1000)}`, "muted"));

  }
  element("sync").disabled = !systemState?.binding || operationController !== null || pendingOperation !== null;
  shell.setStatus("Connected to Mail");
}
function loadBinding(system) {
  element("environment").textContent = system.environment === "sandbox" ? "Sandbox" : "Production";
  element("location-form").hidden = Boolean(system.binding);
  if (!system.binding) { element("binding").textContent = "Choose the mailbox for this deployment."; return; }
  locationController?.abort();
  if (bindingLocation === system.binding.location_id) return;
  bindingController?.abort();
  const controller = new AbortController(); bindingController = controller;
  const locationId = system.binding.location_id;
  json("v1/installation/location", { signal: controller.signal }).then((location) => {
    if (!active || controller.signal.aborted || systemState?.binding?.location_id !== locationId) return;
    element("binding").textContent = [location.address.line1, location.address.line2, location.address.city, location.address.state, location.address.postalCode].filter(Boolean).join(", ");
    bindingLocation = locationId;
  }).catch(() => {});
}
function scheduleRefresh() {
  clearTimeout(refreshTimer);
  if (!active || closed || document.hidden) return;
  const firstSync = systemState?.binding && !syncState?.initial_sync_complete && !syncState?.error_code;
  const delay = pendingOperation || firstSync ? 2000 : 30000;
  refreshTimer = setTimeout(() => refresh(true).catch(() => {}), delay);
}
function refresh(background = false, retryLocations = false) {
  if (refreshPromise) return refreshPromise;
  if (closed || (background && (!active || document.hidden))) return Promise.resolve();
  const controller = new AbortController(); refreshController = controller;
  clearTimeout(refreshTimer);
  const timeout = setTimeout(() => controller.abort(new DOMException("Mail could not be refreshed.", "TimeoutError")), 20000);
  element("refresh").disabled = true;
  refreshPromise = (async () => {
    try {
      const options = { signal: controller.signal };
      const [system, status, summary, operation] = await Promise.all([
        json("v1/system", options), json("v1/sync", options), json("v1/summary", options),
        pendingOperation ? json(`v1/operations/${pendingOperation.id}`, options) : null,
      ]);
      if (controller.signal.aborted || closed) return;
      active = true; systemState = system; syncState = status;
      if (operation) {
        if (operation.state === "failed") manualFailure = { code: operation.error_code || "sync_failed", lastSuccess: status.last_success_at };
        pendingOperation = ["queued", "running"].includes(operation.state) ? { ...pendingOperation, ...operation } : null;
      }
      element("login").hidden = true; element("app").hidden = false; element("refresh").hidden = false; element("app-footer").hidden = false;
      shell.setVersion(system.version);
      loadBinding(system); renderSyncStatus();
      if (!system.binding) {
        setView(true);
        // Discovery contacts Stable. Automatic projection reads must never repeat it.
        if ((!background && !locationsRequested) || (retryLocations && element("location").options.length === 1)) await loadLocations(controller.signal);
      }
      element("counts").replaceChildren(...[[summary.total, "Mail items"], [summary.unread, "Unread"], [summary.ready_scans, "Scans ready"], [summary.pending_scans, "Scanning"]].map(([count, label]) => {
        const metric = text("div", "", "mail-count"); metric.append(text("strong", String(count)), text("span", label)); return metric;
      }));
      notice(""); showError("");
      await loadMail(false, background);
    } catch (error) {
      if (error.name !== "AbortError" && error.status !== 401) {
        shell.setStatus("Connection lost · showing saved mail", "stale");
        if (background) notice("Mail could not be refreshed. Showing saved results.");
        else throw error;
      }
    } finally {
      clearTimeout(timeout);
      if (refreshController === controller) refreshController = null;
      refreshPromise = null;
      element("refresh").disabled = false;
      scheduleRefresh();
    }
  })();
  return refreshPromise;
}
element("login").addEventListener("submit", (event) => { event.preventDefault(); token = element("token").value; element("token").value = ""; action(() => refresh()); });
element("filters").addEventListener("submit", (event) => { event.preventDefault(); appliedFilters = readFilters(); action(() => loadMail()); });
element("refresh").addEventListener("click", () => action(() => refresh(true, true)));
element("load-more").addEventListener("click", () => action(() => loadMail(true)));
element("mail-tab").addEventListener("click", () => setView(false));
element("settings-tab").addEventListener("click", () => setView(true));
element("more-locations").addEventListener("click", () => action(loadLocations));
element("location-form").addEventListener("submit", (event) => { event.preventDefault(); action(async () => {
  await json("v1/installation/location", { method: "POST", body: JSON.stringify({ location_id: element("location").value }) });
  // The binding changed while a local read may still describe the unbound state.
  stopRefresh();
  await refreshPromise?.catch(() => {});
  await refresh();
}); });
element("sync").addEventListener("click", () => action(async () => {
  if (operationController || pendingOperation || !systemState?.binding) return;
  const controller = new AbortController(); operationController = controller;
  renderSyncStatus();
  try {
    const operation = await json("v1/sync", { method: "POST", body: JSON.stringify({ idempotency_key: crypto.randomUUID() }), signal: controller.signal });
    pendingOperation = { ...operation, state: operation.state || "queued" };
    manualFailure = null;
  } finally {
    if (operationController === controller) operationController = null;
    renderSyncStatus(); scheduleRefresh();
  }
}));
element("close-detail").addEventListener("click", () => element("detail").close());
element("detail").addEventListener("close", () => {
  detailGeneration += 1; detailController?.abort(); releaseDocuments();
  if (active && detailOpener && !detailOpener.isConnected) element("mail-heading").focus();
});
document.querySelector(".wordmark").addEventListener("click", (event) => {
  if (event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) { event.preventDefault(); setView(false); }
});
document.addEventListener("visibilitychange", () => {
  if (document.hidden) stopRefresh();
  else if (active && !closed) {
    Promise.resolve(refreshPromise).catch(() => {}).then(() => { if (active && !closed && !document.hidden) refresh(true).catch(() => {}); });
  }
});
window.addEventListener("pagehide", () => { closed = true; active = false; stopRefresh(); operationController?.abort(); detailGeneration += 1; detailController?.abort(); releaseDocuments(); });
window.addEventListener("pageshow", (event) => { if (event.persisted) { closed = false; refresh().catch(() => {}); } });
refresh().catch((error) => {
  if (error.status !== 401) showError(error.message || "Mail is unavailable.");
});
