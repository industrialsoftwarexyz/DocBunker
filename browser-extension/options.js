// Options page logic — persists settings to chrome.storage.local.

const ALL_TYPES = ["pdf", "png", "jpg", "jpeg", "webp", "docx", "pptx", "xlsx"];

const defaults = {
  deleteAfterOpen: false,
  enabledTypes: [...ALL_TYPES],
  downloadDir: "",
};

// --- load ---
async function load() {
  const stored = await chrome.storage.local.get(defaults);

  document.querySelector("#delete-after-open").checked = stored.deleteAfterOpen;
  document.querySelector("#download-dir").value = stored.downloadDir;

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

document.querySelector("#download-dir").addEventListener("change", (e) => {
  chrome.storage.local.set({ downloadDir: e.target.value.trim() });
});

document.querySelector("#reset-dir").addEventListener("click", () => {
  document.querySelector("#download-dir").value = "";
  chrome.storage.local.set({ downloadDir: "" });
});

load();
