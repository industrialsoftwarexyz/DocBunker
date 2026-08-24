# DocBunker for Gmail

A Chrome extension that adds **Open in DocBunker** buttons to Gmail
attachments. Click a button, the file gets downloaded, validated, and opened
in the DocBunker sandbox — no manual download-then-open.

## What it does

- Scans Gmail attachment links for supported file types (PDF, PNG, JPEG,
  WebP, DOCX, PPTX, XLSX)
- Inserts a button next to each one
- Downloads the file through Chrome's normal download flow
- Sends only the file path to the DocBunker native host
- Waits for DocBunker to confirm ingestion before reporting success
- Optionally deletes Chrome's copy after confirmation

## Install for testing

1. Run DocBunker once so it registers the native host.
2. Open `chrome://extensions`, enable Developer mode, click **Load unpacked**.
3. Select this `browser-extension` directory.

The fixed manifest key gives this unpacked build the extension ID
`lmmdckggliegiglepibblfnpaiaeojpf`, matching the native host allowlist.

## Permissions

The extension requests: `contextMenus`, `downloads`, `nativeMessaging`,
`notifications`, `storage`, and host access to `mail.google.com` only.

It does **not** use the Gmail API, cookies, or browsing history.

## Options

One toggle: delete Chrome's downloaded copy after DocBunker confirms
ingestion. Off by default. Stored in `chrome.storage.local`.

## Privacy

No data leaves the extension except the file path, sent to the local
DocBunker native host. No analytics, no external services, no persistent
transfer metadata. Full details in [PRIVACY.md](PRIVACY.md).

## Production distribution

Publish in the Chrome Web Store with the same manifest key. The native host
allowlist on the user's machine must match the extension ID.
