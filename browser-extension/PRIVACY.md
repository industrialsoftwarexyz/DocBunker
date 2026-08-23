# Privacy

DocBunker for Gmail does not collect, transmit, sell or retain personal data.

The extension acts only after the user chooses **Open in DocBunker**. Chrome
downloads that selected attachment through its normal authenticated download
flow. The extension sends the resulting local path to the allowlisted DocBunker
native host; attachment bytes, Gmail URLs, filenames and mailbox information
are never sent to an external service.

Transfer metadata is stored only in `chrome.storage.session` and disappears
when the browser session ends. The optional `deleteAfterOpen` preference is the
only persistent extension setting.
