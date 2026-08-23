const checkbox = document.querySelector("#delete-after-open");

chrome.storage.local.get({ deleteAfterOpen: false }).then(({ deleteAfterOpen }) => {
  checkbox.checked = deleteAfterOpen;
});

checkbox.addEventListener("change", () => {
  void chrome.storage.local.set({ deleteAfterOpen: checkbox.checked });
});
