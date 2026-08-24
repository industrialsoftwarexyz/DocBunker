// Generic webmail attachment scanner.
// Replaces the old Gmail-only gmail.js. Each provider defines how to find
// attachment links and verify the URL belongs to that provider.

const BUTTON_CLASS = "docbunker-open-button";

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

function addButtons(root = document) {
  for (const link of root.querySelectorAll("a[href]")) {
    if (!(link instanceof HTMLAnchorElement) || link.dataset.docbunkerReady)
      continue;

    const provider = PROVIDERS.find((p) => p.test(link));
    if (!provider) continue;

    const text = `${link.textContent ?? ""} ${link.getAttribute("aria-label") ?? ""}`.toLowerCase();
    if (!provider.isAttachment(link, text)) continue;

    link.dataset.docbunkerReady = "true";
    const button = document.createElement("button");
    button.type = "button";
    button.className = BUTTON_CLASS;
    button.textContent = "Open in DocBunker";
    button.title = "Download and open this attachment in the isolated DocBunker viewer";
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
}

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
`;
document.documentElement.appendChild(style);

addButtons();
new MutationObserver((records) => {
  for (const record of records) {
    for (const node of record.addedNodes) {
      if (node instanceof Element) addButtons(node);
    }
  }
}).observe(document.body, { childList: true, subtree: true });

chrome.runtime.onMessage.addListener((message) => {
  if (
    message?.type !== "docBunkerStatus" ||
    typeof message.requestId !== "string"
  )
    return;
  for (const button of document.querySelectorAll(`.${BUTTON_CLASS}`)) {
    if (button.dataset.docbunkerRequest !== message.requestId) continue;
    button.textContent =
      message.status === "opened" ? "Opened in DocBunker" : "Open in DocBunker";
    button.title = message.message;
    button.disabled = false;
    delete button.dataset.docbunkerRequest;
  }
});
