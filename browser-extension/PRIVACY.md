# Privacy

DocBunker for Gmail does not collect, transmit, sell, or retain personal
data.

The extension only acts when you click **Open in DocBunker**. Chrome
downloads the file through its normal flow — the extension never reads
the file bytes. The only thing sent to the DocBunker native host is the
local file path. No Gmail URLs, filenames, mailbox content, or any other
data is transmitted anywhere.

Transfer metadata is stored only in `chrome.storage.session` and is
wiped when the browser session ends. The only persistent setting is the
optional `deleteAfterOpen` toggle, stored locally in
`chrome.storage.local`.

No analytics. No external services. No tracking.
