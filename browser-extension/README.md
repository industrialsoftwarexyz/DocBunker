# DocBunker browser extension

A browser extension that adds **Open in DocBunker** buttons to webmail
attachments. Works on Chrome, Edge, and Firefox.

Click a button, the file gets downloaded, validated, and opened in the
DocBunker sandbox — no manual download-then-open.

## Supported webmail

Gmail, Outlook (live.com / office.com / office365.com), Yahoo Mail,
Proton Mail, iCloud Mail, Zoho Mail, GMX, Web.de, Mail.com, AOL Mail.

## Install for testing

### Chrome / Edge

1. Run DocBunker once so it registers the native host.
2. Open `chrome://extensions` (or `edge://extensions`), enable Developer
   mode, click **Load unpacked**.
3. Select this `browser-extension` directory.

### Firefox

1. Run DocBunker once so it registers the native host.
2. Open `about:debugging#/runtime/this-firefox`.
3. Click **Load Temporary Add-on** and select `manifest.firefox.json`.

## How it works

- Scans webmail attachment links for supported file types (PDF, PNG, JPEG,
  WebP, DOCX, PPTX, XLSX)
- Inserts a button next to each one
- Downloads the file through the browser's normal download flow
- Sends only the file path to the DocBunker native host
- Waits for DocBunker to confirm ingestion before reporting success
- Optionally deletes the browser's copy after confirmation

## Files

| File | Purpose |
| --- | --- |
| `manifest.json` | Chrome/Edge manifest (Manifest V3) |
| `manifest.firefox.json` | Firefox manifest (Manifest V2) |
| `service-worker.js` | Chrome/Edge background (service worker) |
| `background.js` | Firefox background (identical logic) |
| `webmail.js` | Content script — attachment detection + buttons |
| `options.html` / `options.js` | Settings page |
| `icon-128.png` | Extension icon |

## Permissions

The extension requests: `contextMenus`, `downloads`, `nativeMessaging`,
`notifications`, `storage`, and host access to supported webmail sites only.

It does **not** use email APIs, cookies, or browsing history.

## Privacy

No data leaves the extension except the file path, sent to the local
DocBunker native host. No analytics, no external services, no persistent
transfer metadata. Full details in [PRIVACY.md](PRIVACY.md).
