// Cross-browser background script.
// Works as a Manifest V3 service_worker (Chrome/Edge) and as a
// Manifest V2 background script (Firefox).

const api = typeof browser !== "undefined" ? browser : chrome;

const HOST = "dev.docbunker.viewer";
const MENU_ID = "docbunker-open-attachment";
const PENDING_PREFIX = "pending:";
const ACTIVE_PREFIX = "active:";
const MAX_PENDING = 3;
const MAX_AGE_MS = 60 * 60 * 1000;
const MAX_RETRIES = 2;
const RETRY_DELAY_MS = [2000, 5000];

const ALLOWED_ORIGINS = [
  "https://mail.google.com",
  "https://outlook.live.com",
  "https://outlook.office.com",
  "https://outlook.office365.com",
  "https://mail.yahoo.com",
  "https://mail.proton.me",
  "https://mail.protonmail.com",
  "https://www.icloud.com",
  "https://mail.zoho.com",
  "https://gmx.com",
  "https://gmx.net",
  "https://www.web.de",
  "https://mail.com",
  "https://mail.aol.com",
];

const ALLOWED_URL_PATTERNS = ALLOWED_ORIGINS.map((o) => `${o}/*`);

// --- storage helpers ---
function storageGet(keys) {
  if (api.storage.session) return api.storage.session.get(keys);
  return api.storage.local.get(keys);
}

function storageSet(items) {
  if (api.storage.session) return api.storage.session.set(items);
  return api.storage.local.set(items);
}

function storageRemove(keys) {
  if (api.storage.session) return api.storage.session.remove(keys);
  return api.storage.local.remove(keys);
}

// --- badge ---
async function updateBadge() {
  const state = await storageGet(null);
  const count = Object.keys(state).filter((k) =>
    k.startsWith(PENDING_PREFIX),
  ).length;
  api.action.setBadgeText({ text: count > 0 ? String(count) : "" });
  api.action.setBadgeBackgroundColor({
    color: count > 0 ? "#174a70" : "#999",
  });
}

// --- setup ---
api.runtime.onInstalled.addListener(async () => {
  await api.contextMenus.removeAll();
  api.contextMenus.create({
    id: MENU_ID,
    title: "Open attachment in DocBunker",
    contexts: ["link"],
    documentUrlPatterns: ALLOWED_URL_PATTERNS,
  });
  api.action.setBadgeText({ text: "" });
});

void reconcilePendingDownloads();

// --- context menu ---
api.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId === MENU_ID && info.linkUrl) {
    void downloadAndOpen(info.linkUrl, tab?.id);
  }
});

// Update context menu title to show hovered link's filename
api.contextMenus.onShown?.addListener(async (info, tab) => {
  if (!tab?.id || !isAllowedWebmailUrl(tab.url ?? "")) return;
  try {
    api.tabs.sendMessage(
      tab.id,
      { type: "getContextMenuFilename" },
      (response) => {
        if (chrome.runtime.lastError || !response?.filename) return;
        api.contextMenus.update(MENU_ID, {
          title: `Open "${response.filename}" in DocBunker`,
        });
      },
    );
  } catch {}
});

// --- keyboard shortcut ---
api.commands.onCommand.addListener((command) => {
  if (command === "open-focused-attachment") {
    void openFocusedAttachment();
  }
});

async function openFocusedAttachment() {
  const [tab] = await api.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id) return;
  try {
    api.tabs.sendMessage(
      tab.id,
      { type: "getFocusedAttachment" },
      (response) => {
        if (chrome.runtime.lastError || !response?.url) return;
        void downloadAndOpen(response.url, tab.id);
      },
    );
  } catch {}
}

// --- messages from content script ---
api.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type === "openAllAttachments" && Array.isArray(message.urls)) {
    if (!sender.tab?.id || !sender.url) {
      sendResponse({ ok: false, message: "Untrusted message source" });
      return;
    }
    if (!isAllowedWebmailUrl(sender.url)) {
      sendResponse({ ok: false, message: "Untrusted message source" });
      return;
    }
    void openAllAttachments(message.urls, sender.tab.id).then(sendResponse);
    return true;
  }

  if (message?.type !== "openAttachment" || typeof message.url !== "string")
    return;
  if (!sender.tab?.id || !sender.url) {
    sendResponse({ ok: false, message: "Untrusted message source" });
    return;
  }
  if (!isAllowedWebmailUrl(sender.url)) {
    sendResponse({ ok: false, message: "Untrusted message source" });
    return;
  }
  void downloadAndOpen(message.url, sender.tab.id).then(sendResponse);
  return true;
});

// --- batch open ---
async function openAllAttachments(urls, tabId) {
  const results = [];
  for (const url of urls) {
    const result = await downloadAndOpen(url, tabId);
    results.push(result);
  }
  const opened = results.filter((r) => r.ok).length;
  const failed = results.length - opened;
  if (failed > 0) {
    notify(`${failed} attachment${failed > 1 ? "s" : ""} could not be opened`);
  }
  return { ok: true, opened, failed };
}

// --- core flow ---
async function downloadAndOpen(url, tabId, retryCount = 0) {
  if (!isAllowedWebmailUrl(url))
    return { ok: false, message: "Unsupported attachment URL" };

  const urlHash = await hashUrl(url);
  const activeKey = `${ACTIVE_PREFIX}${urlHash}`;
  const existing = (await storageGet(activeKey))[activeKey];
  if (existing) return { ok: true, requestId: existing, status: "downloading" };

  const allState = await storageGet(null);
  const pendingCount = Object.keys(allState).filter((k) =>
    k.startsWith(PENDING_PREFIX),
  ).length;
  if (pendingCount >= MAX_PENDING) {
    return {
      ok: false,
      message: "Wait for another DocBunker transfer to finish",
    };
  }

  const requestId = crypto.randomUUID();
  try {
    const id = await api.downloads.download({
      url,
      saveAs: false,
      conflictAction: "uniquify",
    });
    await storageSet({
      [`${PENDING_PREFIX}${id}`]: {
        requestId,
        tabId,
        urlHash,
        url,
        filename: extractFilename(url),
        retryCount,
        startedAt: Date.now(),
      },
      [activeKey]: requestId,
    });
    await updateBadge();
    return { ok: true, requestId, status: "downloading" };
  } catch (error) {
    const msg = `Could not download attachment: ${String(error)}`;
    notify(msg);
    return { ok: false, message: msg };
  }
}

// --- download completion ---
api.downloads.onChanged.addListener((change) => {
  if (change.error || change.state?.current === "complete") {
    void finishDownload(change.id, change.error?.current);
  }
});

async function finishDownload(id, downloadError) {
  const key = `${PENDING_PREFIX}${id}`;
  const pending = (await storageGet(key))[key];
  if (!pending) return;

  if (downloadError) {
    if (pending.retryCount < MAX_RETRIES) {
      const delay = RETRY_DELAY_MS[pending.retryCount] ?? 3000;
      await storageRemove([
        key,
        `${ACTIVE_PREFIX}${pending.urlHash}`,
      ]);
      await updateBadge();
      setTimeout(
        () => void downloadAndOpen(pending.url, pending.tabId, pending.retryCount + 1),
        delay,
      );
      return;
    }
    await finishPending(
      id,
      pending,
      "failed",
      `${pending.filename ?? "Attachment"} download failed: ${downloadError}`,
    );
    return;
  }

  const [download] = await api.downloads.search({ id });
  if (!download?.filename || !isAllowedWebmailUrl(download.url)) {
    await finishPending(id, pending, "failed", "Invalid attachment download.");
    return;
  }

  const displayName = pending.filename ?? extractFilename(download.url);
  const { downloadDir = "" } = await api.storage.local.get("downloadDir");
  const response = await sendNativeMessage({
    action: "open_file",
    path: download.filename,
    ...(downloadDir ? { allowedDir: downloadDir } : {}),
  });
  if (!response.ok) {
    await finishPending(
      id,
      pending,
      "failed",
      `${displayName}: ${response.message}`,
    );
    return;
  }

  const { deleteAfterOpen = false } = await api.storage.local.get(
    "deleteAfterOpen",
  );
  if (deleteAfterOpen) {
    await api.downloads.removeFile(id).catch(() => undefined);
    await api.downloads.erase({ id });
  }
  await finishPending(
    id,
    pending,
    "opened",
    `${displayName} opened in DocBunker`,
  );
}

async function finishPending(id, pending, status, message) {
  await storageRemove([
    `${PENDING_PREFIX}${id}`,
    `${ACTIVE_PREFIX}${pending.urlHash}`,
  ]);
  await updateBadge();
  if (pending.tabId) {
    api.tabs.sendMessage(
      pending.tabId,
      {
        type: "docBunkerStatus",
        requestId: pending.requestId,
        status,
        message,
      },
      () => void chrome.runtime.lastError,
    );
  }
  if (status === "failed") notify(message);
}

// --- reconcile stale pending transfers from a previous session ---
async function reconcilePendingDownloads() {
  const state = await storageGet(null);
  for (const [key, pending] of Object.entries(state)) {
    if (!key.startsWith(PENDING_PREFIX)) continue;
    const id = Number(key.slice(PENDING_PREFIX.length));
    if (!Number.isInteger(id) || Date.now() - pending.startedAt > MAX_AGE_MS) {
      await finishPending(id, pending, "failed", "DocBunker transfer expired.");
      continue;
    }
    const [download] = await api.downloads.search({ id });
    if (!download || download.state === "interrupted") {
      if (pending.retryCount < MAX_RETRIES) {
        const delay = RETRY_DELAY_MS[pending.retryCount] ?? 3000;
        await storageRemove([key, `${ACTIVE_PREFIX}${pending.urlHash}`]);
        await updateBadge();
        setTimeout(
          () =>
            void downloadAndOpen(pending.url, pending.tabId, pending.retryCount + 1),
          delay,
        );
        continue;
      }
      await finishPending(
        id,
        pending,
        "failed",
        `${pending.filename ?? "Attachment"} download was interrupted.`,
      );
    } else if (download.state === "complete") {
      await finishDownload(id);
    }
  }
  await updateBadge();
}

// --- native messaging ---
function sendNativeMessage(message) {
  return new Promise((resolve) => {
    api.runtime.sendNativeMessage(HOST, message, (response) => {
      if (chrome.runtime.lastError) {
        resolve({ ok: false, message: chrome.runtime.lastError.message });
      } else {
        resolve(
          response?.ok
            ? response
            : {
                ok: false,
                message:
                  response?.message ?? "DocBunker rejected the attachment",
              },
        );
      }
    });
  });
}

// --- utilities ---
function extractFilename(url) {
  try {
    const pathname = new URL(url).pathname;
    const segments = pathname.split("/");
    const last = segments[segments.length - 1];
    if (last) return decodeURIComponent(last);
  } catch {}
  return "attachment";
}

async function hashUrl(url) {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(url),
  );
  return Array.from(new Uint8Array(digest), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}

function isAllowedWebmailUrl(value) {
  try {
    const origin = new URL(value).origin;
    return ALLOWED_ORIGINS.includes(origin);
  } catch {
    return false;
  }
}

function notify(message) {
  api.notifications.create({
    type: "basic",
    iconUrl: "icon-128.png",
    title: "DocBunker",
    message,
  });
}
