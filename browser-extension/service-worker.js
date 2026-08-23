const HOST = "dev.docbunker.viewer";
const MENU_ID = "docbunker-open-attachment";
const PENDING_PREFIX = "pending:";
const ACTIVE_PREFIX = "active:";
const MAX_PENDING = 3;
const MAX_AGE_MS = 60 * 60 * 1000;

chrome.runtime.onInstalled.addListener(async () => {
  await chrome.contextMenus.removeAll();
  chrome.contextMenus.create({
    id: MENU_ID,
    title: "Open attachment in DocBunker",
    contexts: ["link"],
    documentUrlPatterns: ["https://mail.google.com/*"],
  });
});

void reconcilePendingDownloads();

chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId === MENU_ID && info.linkUrl) {
    void downloadAndOpen(info.linkUrl, tab?.id);
  }
});

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type !== "openAttachment" || typeof message.url !== "string") return;
  if (!sender.url?.startsWith("https://mail.google.com/") || !sender.tab?.id) {
    sendResponse({ ok: false, message: "Untrusted message source" });
    return;
  }
  void downloadAndOpen(message.url, sender.tab.id).then(sendResponse);
  return true;
});

async function downloadAndOpen(url, tabId) {
  if (!isGmailUrl(url)) return { ok: false, message: "Unsupported attachment URL" };

  const urlHash = await hashUrl(url);
  const activeKey = `${ACTIVE_PREFIX}${urlHash}`;
  const existing = (await chrome.storage.session.get(activeKey))[activeKey];
  if (existing) return { ok: true, requestId: existing, status: "downloading" };

  const allState = await chrome.storage.session.get(null);
  const pendingCount = Object.keys(allState).filter((key) => key.startsWith(PENDING_PREFIX)).length;
  if (pendingCount >= MAX_PENDING) {
    return { ok: false, message: "Wait for another DocBunker transfer to finish" };
  }

  const requestId = crypto.randomUUID();
  try {
    const id = await chrome.downloads.download({ url, saveAs: false, conflictAction: "uniquify" });
    await chrome.storage.session.set({
      [`${PENDING_PREFIX}${id}`]: { requestId, tabId, urlHash, startedAt: Date.now() },
      [activeKey]: requestId,
    });
    return { ok: true, requestId, status: "downloading" };
  } catch (error) {
    const message = `Could not download attachment: ${String(error)}`;
    notify(message);
    return { ok: false, message };
  }
}

chrome.downloads.onChanged.addListener((change) => {
  if (change.error || change.state?.current === "complete") {
    void finishDownload(change.id, change.error?.current);
  }
});

async function finishDownload(id, downloadError) {
  const key = `${PENDING_PREFIX}${id}`;
  const pending = (await chrome.storage.session.get(key))[key];
  if (!pending) return;

  if (downloadError) {
    await finishPending(id, pending, "failed", `Attachment download failed: ${downloadError}`);
    return;
  }

  const [download] = await chrome.downloads.search({ id });
  if (!download?.filename || !isGmailUrl(download.url)) {
    await finishPending(id, pending, "failed", "Chrome returned an invalid attachment download.");
    return;
  }

  const response = await sendNativeMessage({ action: "open_file", path: download.filename });
  if (!response.ok) {
    await finishPending(id, pending, "failed", response.message);
    return;
  }

  const { deleteAfterOpen = false } = await chrome.storage.local.get("deleteAfterOpen");
  if (deleteAfterOpen) {
    await chrome.downloads.removeFile(id).catch(() => undefined);
    await chrome.downloads.erase({ id });
  }
  await finishPending(id, pending, "opened", "Opened securely in DocBunker");
}

async function finishPending(id, pending, status, message) {
  await chrome.storage.session.remove([
    `${PENDING_PREFIX}${id}`,
    `${ACTIVE_PREFIX}${pending.urlHash}`,
  ]);
  if (pending.tabId) {
    chrome.tabs.sendMessage(
      pending.tabId,
      { type: "docBunkerStatus", requestId: pending.requestId, status, message },
      () => void chrome.runtime.lastError,
    );
  }
  if (status === "failed") notify(message);
}

async function reconcilePendingDownloads() {
  const state = await chrome.storage.session.get(null);
  for (const [key, pending] of Object.entries(state)) {
    if (!key.startsWith(PENDING_PREFIX)) continue;
    const id = Number(key.slice(PENDING_PREFIX.length));
    if (!Number.isInteger(id) || Date.now() - pending.startedAt > MAX_AGE_MS) {
      await finishPending(id, pending, "failed", "DocBunker transfer expired.");
      continue;
    }
    const [download] = await chrome.downloads.search({ id });
    if (!download || download.state === "interrupted") {
      await finishPending(id, pending, "failed", "Attachment download was interrupted.");
    } else if (download.state === "complete") {
      await finishDownload(id);
    }
  }
}

function sendNativeMessage(message) {
  return new Promise((resolve) => {
    chrome.runtime.sendNativeMessage(HOST, message, (response) => {
      if (chrome.runtime.lastError) {
        resolve({ ok: false, message: chrome.runtime.lastError.message });
      } else {
        resolve(response?.ok ? response : { ok: false, message: response?.message ?? "DocBunker rejected the attachment" });
      }
    });
  });
}

async function hashUrl(url) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(url));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function isGmailUrl(value) {
  try {
    return new URL(value).origin === "https://mail.google.com";
  } catch {
    return false;
  }
}

function notify(message) {
  chrome.notifications.create({
    type: "basic",
    iconUrl: "icon-128.png",
    title: "DocBunker",
    message,
  });
}
