// Options page logic — persists settings to chrome.storage.local.

const ALL_TYPES = ["pdf", "png", "jpg", "jpeg", "webp", "docx", "pptx", "xlsx", "gif", "tiff", "bmp", "epub", "rtf"];

const defaults = {
  deleteAfterOpen: false,
  autoOpen: false,
  enabledTypes: [...ALL_TYPES],
};

// --- load ---
async function load() {
  const stored = await chrome.storage.local.get(defaults);

  document.querySelector("#delete-after-open").checked = stored.deleteAfterOpen;
  document.querySelector("#auto-open").checked = stored.autoOpen;
  const container = document.querySelector("#type-toggles");
  for (const ext of ALL_TYPES) {
    const label = document.createElement("label");
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = stored.enabledTypes.includes(ext);
    cb.dataset.ext = ext;
    cb.addEventListener("change", saveTypes);
    label.appendChild(cb);
    label.appendChild(document.createTextNode(`.${ext}`));
    container.appendChild(label);
  }
}

// --- save ---
function saveTypes() {
  const checked = [...document.querySelectorAll("#type-toggles input:checked")]
    .map((cb) => cb.dataset.ext);
  chrome.storage.local.set({ enabledTypes: checked });
}

document.querySelector("#delete-after-open").addEventListener("change", (e) => {
  chrome.storage.local.set({ deleteAfterOpen: e.target.checked });
});

document.querySelector("#auto-open").addEventListener("change", (e) => {
  chrome.storage.local.set({ autoOpen: e.target.checked });
});

load();
