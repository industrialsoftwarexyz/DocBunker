// Generic webmail attachment scanner.
// Each provider defines how to find attachment links and verify the URL
// belongs to that provider.

const BUTTON_CLASS = "docbunker-open-button";
const BATCH_BUTTON_CLASS = "docbunker-open-all";

const SUPPORTED_EXT = /\.(pdf|png|jpe?g|webp|docx|pptx|xlsx)$/i;

const PROVIDERS = [
  {
    name: "gmail",
    test: (a) => a.href.includes("mail.google.com/"),
    isAttachment: (a, text) =>
      a.hasAttribute("download") || SUPPORTED_EXT.test(text),
  },
  {
    name: "outlook",
    test: (a) =>
      a.href.includes("outlook.live.com/") ||
      a.href.includes("outlook.office.com/") ||
      a.href.includes("outlook.office365.com/"),
    isAttachment: (a, text) =>
      a.hasAttribute("download") ||
      /attachment|viewasmessage/i.test(a.className) ||
      SUPPORTED_EXT.test(text) ||
      SUPPORTED_EXT.test(a.href),
  },
  {
    name: "yahoo",
    test: (a) => a.href.includes("mail.yahoo.com/"),
    isAttachment: (a, text) =>
      a.hasAttribute("download") ||
      SUPPORTED_EXT.test(text) ||
      SUPPORTED_EXT.test(a.href),
  },
  {
    name: "proton",
    test: (a) =>
      a.href.includes("mail.proton.me/") ||
      a.href.includes("mail.protonmail.com/"),
    isAttachment: (a, text) =>
      a.hasAttribute("download") ||
      SUPPORTED_EXT.test(text) ||
      SUPPORTED_EXT.test(a.href),
  },
  {
    name: "icloud",
    test: (a) => a.href.includes("icloud.com/"),
    isAttachment: (a, text) =>
      a.hasAttribute("download") ||
      a.getAttribute("data-qa") === "attachment-download" ||
      SUPPORTED_EXT.test(text) ||
      SUPPORTED_EXT.test(a.href),
  },
  {
    name: "zoho",
    test: (a) => a.href.includes("mail.zoho."),
    isAttachment: (a, text) =>
      a.hasAttribute("download") || SUPPORTED_EXT.test(text),
  },
  {
    name: "gmx",
    test: (a) =>
      a.href.includes("gmx.com/") || a.href.includes("gmx.net/"),
    isAttachment: (a, text) =>
      a.hasAttribute("download") || SUPPORTED_EXT.test(text),
  },
  {
    name: "webde",
    test: (a) => a.href.includes("web.de/"),
    isAttachment: (a, text) =>
      a.hasAttribute("download") || SUPPORTED_EXT.test(text),
  },
  {
    name: "mail-com",
    test: (a) => {
      try {
        return new URL(a.href).hostname === "mail.com";
      } catch {
        return false;
      }
    },
    isAttachment: (a, text) =>
      a.hasAttribute("download") || SUPPORTED_EXT.test(text),
  },
  {
    name: "aol",
    test: (a) => a.href.includes("mail.aol.com/"),
    isAttachment: (a, text) =>
      a.hasAttribute("download") || SUPPORTED_EXT.test(text),
  },
];

// --- file type filter ---
let enabledTypes = null;

async function loadTypeFilter() {
  try {
    const result = await chrome.storage.local.get({ enabledTypes: null });
    if (result.enabledTypes && Array.isArray(result.enabledTypes)) {
      enabledTypes = new Set(result.enabledTypes);
    }
  } catch {}
}

function isTypeAllowed(filename) {
  if (!enabledTypes) return true; // no filter = allow all
  const ext = filename.split(".").pop()?.toLowerCase();
  return enabledTypes.has(ext);
}

// --- find current provider ---
function detectProvider() {
  const host = location.hostname;
  return PROVIDERS.find((p) => {
    if (p.name === "gmail") return host === "mail.google.com";
    if (p.name === "outlook")
      return (
        host === "outlook.live.com" ||
        host === "outlook.office.com" ||
        host === "outlook.office365.com"
      );
    if (p.name === "yahoo") return host === "mail.yahoo.com";
    if (p.name === "proton")
      return host === "mail.proton.me" || host === "mail.protonmail.com";
    if (p.name === "icloud") return host === "icloud.com";
    if (p.name === "zoho") return host.startsWith("mail.zoho.");
    if (p.name === "gmx") return host === "gmx.com" || host === "gmx.net";
    if (p.name === "webde") return host === "www.web.de";
    if (p.name === "mail-com") return host === "mail.com";
    if (p.name === "aol") return host === "mail.aol.com";
    return false;
  });
}

const currentProvider = detectProvider();

// --- helpers ---
function extractFilenameFromLink(link) {
  const text = `${link.textContent ?? ""} ${link.getAttribute("aria-label") ?? ""}`.toLowerCase();
  const match = text.match(/\b[\w.-]+\.(pdf|png|jpe?g|webp|docx|pptx|xlsx)\b/);
  if (match) return match[0];
  try {
    const pathname = new URL(link.href).pathname;
    const segments = pathname.split("/");
    const last = segments[segments.length - 1];
    if (last) return decodeURIComponent(last);
  } catch {}
  return "attachment";
}

// --- attachment link scanning ---
function getAttachmentLinks() {
  const links = [];
  for (const link of document.querySelectorAll("a[href]")) {
    if (!(link instanceof HTMLAnchorElement) || link.dataset.docbunkerReady)
      continue;
    if (!currentProvider || !currentProvider.test(link)) continue;
    const text = `${link.textContent ?? ""} ${link.getAttribute("aria-label") ?? ""}`.toLowerCase();
    if (!currentProvider.isAttachment(link, text)) continue;
    if (!isTypeAllowed(extractFilenameFromLink(link))) continue;
    links.push(link);
  }
  return links;
}

function addButtons(root = document) {
  for (const link of root.querySelectorAll("a[href]")) {
    if (!(link instanceof HTMLAnchorElement) || link.dataset.docbunkerReady)
      continue;
    if (!currentProvider || !currentProvider.test(link)) continue;
    const text = `${link.textContent ?? ""} ${link.getAttribute("aria-label") ?? ""}`.toLowerCase();
    if (!currentProvider.isAttachment(link, text)) continue;
    if (!isTypeAllowed(extractFilenameFromLink(link))) continue;

    link.dataset.docbunkerReady = "true";
    const button = document.createElement("button");
    button.type = "button";
    button.className = BUTTON_CLASS;
    button.textContent = "Open in DocBunker";
    button.title =
      "Download and open this attachment in the isolated DocBunker viewer";
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      button.disabled = true;
      button.textContent = "Downloading\u2026";
      chrome.runtime.sendMessage(
        { type: "openAttachment", url: link.href },
        (response) => {
          if (!response?.ok) {
            button.disabled = false;
            button.textContent = response?.message ?? "Try again";
            return;
          }
          button.dataset.docbunkerRequest = response.requestId;
        },
      );
    });
    link.insertAdjacentElement("afterend", button);
  }
  addBatchButton();
}

// --- "Open all" batch button ---
function addBatchButton() {
  const existing = document.querySelector(`.${BATCH_BUTTON_CLASS}`);
  if (existing) existing.remove();

  const links = getAttachmentLinks();
  if (links.length < 2) return;

  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = BATCH_BUTTON_CLASS;
  btn.textContent = `Open all ${links.length} in DocBunker`;
  btn.title = "Download and open all attachments in the DocBunker sandbox";

  btn.addEventListener("click", () => {
    btn.disabled = true;
    btn.textContent = "Opening\u2026";
    const urls = getAttachmentLinks().map((a) => a.href);
    chrome.runtime.sendMessage(
      { type: "openAllAttachments", urls },
      (response) => {
        if (response?.ok) {
          btn.textContent = `Opened ${response.opened} in DocBunker`;
          if (response.failed > 0) {
            btn.textContent += ` (${response.failed} failed)`;
          }
        } else {
          btn.textContent = response?.message ?? "Failed";
          btn.disabled = false;
        }
      },
    );
  });

  const lastLink = links[links.length - 1];
  const lastButton = lastLink?.nextElementSibling;
  const anchor = lastButton?.classList?.contains(BUTTON_CLASS)
    ? lastButton
    : lastLink;
  if (anchor?.parentNode) {
    const wrapper = document.createElement("div");
    wrapper.style.cssText =
      "margin-top:6px; margin-inline-start:0; padding:2px 0;";
    wrapper.appendChild(btn);
    anchor.parentNode.insertBefore(wrapper, anchor.nextSibling);
  }
}

// --- styles ---
const style = document.createElement("style");
style.textContent = `
  .${BUTTON_CLASS} {
    margin-inline-start: 6px;
    padding: 3px 8px;
    color: #174a70;
    background: linear-gradient(#fff, #dbeaf5);
    border: 1px solid #7d9db5;
    border-radius: 3px;
    font: 12px "Segoe UI", sans-serif;
    cursor: pointer;
  }
  .${BUTTON_CLASS}:hover { background: linear-gradient(#fff, #c6e5f7); }
  .${BUTTON_CLASS}:disabled { color: #777; cursor: default; }
  .${BATCH_BUTTON_CLASS} {
    margin-inline-start: 6px;
    padding: 4px 10px;
    color: #fff;
    background: #174a70;
    border: 1px solid #0f3554;
    border-radius: 3px;
    font: 12px "Segoe UI", sans-serif;
    cursor: pointer;
  }
  .${BATCH_BUTTON_CLASS}:hover { background: #1a5a8a; }
  .${BATCH_BUTTON_CLASS}:disabled { color: #aaa; cursor: default; background: #555; }
`;
document.documentElement.appendChild(style);

// --- init ---
(async () => {
  await loadTypeFilter();
  addButtons();
  new MutationObserver((records) => {
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node instanceof Element) addButtons(node);
      }
    }
  }).observe(document.body, { childList: true, subtree: true });
})();

// --- listen for filter changes from options page ---
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.enabledTypes) {
    enabledTypes = new Set(changes.enabledTypes.newValue);
    // re-scan: remove buttons for disabled types, add for newly enabled
    document.querySelectorAll(`.${BUTTON_CLASS}, .${BATCH_BUTTON_CLASS}`).forEach((el) => el.remove());
    document.querySelectorAll("[data-docbunker-ready]").forEach((el) => delete el.dataset.docbunkerReady);
    addButtons();
  }
});

// --- keyboard shortcut: open focused attachment ---
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type === "getFocusedAttachment") {
    const focused = document.activeElement;
    let el = focused;
    while (el && el !== document.body) {
      if (el.tagName === "A" && el.dataset.docbunkerReady) {
        sendResponse({ url: el.href });
        return;
      }
      const btn = el.querySelector?.(`.${BUTTON_CLASS}`);
      if (btn) {
        const prevBtn = el.previousElementSibling;
        if (
          prevBtn?.classList?.contains(BUTTON_CLASS) ||
          prevBtn?.dataset?.docbunkerReady
        ) {
          sendResponse({ url: prevBtn.href || prevBtn.previousElementSibling?.href });
          return;
        }
      }
      el = el.parentElement;
    }
    const buttons = document.querySelectorAll(`.${BUTTON_CLASS}`);
    for (const btn of buttons) {
      const rect = btn.getBoundingClientRect();
      if (
        rect.top >= 0 &&
        rect.top <= window.innerHeight &&
        rect.left >= 0 &&
        rect.left <= window.innerWidth
      ) {
        const link = btn.previousElementSibling;
        if (link?.href) {
          sendResponse({ url: link.href });
          return;
        }
      }
    }
    sendResponse({ url: null });
  }

  if (
    message?.type === "docBunkerStatus" &&
    typeof message.requestId === "string"
  ) {
    for (const button of document.querySelectorAll(`.${BUTTON_CLASS}`)) {
      if (button.dataset.docbunkerRequest !== message.requestId) continue;
      button.textContent =
        message.status === "opened"
          ? "Opened in DocBunker"
          : "Open in DocBunker";
      button.title = message.message;
      button.disabled = false;
      delete button.dataset.docbunkerRequest;
    }
  }
});
