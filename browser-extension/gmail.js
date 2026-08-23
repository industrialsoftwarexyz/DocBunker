const BUTTON_CLASS = "docbunker-open-button";

function addButtons(root = document) {
  for (const link of root.querySelectorAll("a[href]")) {
    if (!(link instanceof HTMLAnchorElement) || link.dataset.docbunkerReady) continue;
    if (!link.href.startsWith("https://mail.google.com/")) continue;
    const text = `${link.textContent ?? ""} ${link.getAttribute("aria-label") ?? ""}`.toLowerCase();
    if (!link.hasAttribute("download") && !text.match(/\.pdf|\.png|\.jpe?g|\.webp|\.docx|\.pptx|\.xlsx/)) continue;

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
      button.textContent = "Downloading…";
      chrome.runtime.sendMessage({ type: "openAttachment", url: link.href }, (response) => {
        if (!response?.ok) {
          button.disabled = false;
          button.textContent = response?.message ?? "Try again";
          return;
        }
        button.dataset.docbunkerRequest = response.requestId;
      });
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
  if (message?.type !== "docBunkerStatus" || typeof message.requestId !== "string") return;
  for (const button of document.querySelectorAll(`.${BUTTON_CLASS}`)) {
    if (button.dataset.docbunkerRequest !== message.requestId) continue;
    button.textContent = message.status === "opened" ? "Opened in DocBunker" : "Open in DocBunker";
    button.title = message.message;
    button.disabled = false;
    delete button.dataset.docbunkerRequest;
  }
});
