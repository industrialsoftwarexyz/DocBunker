# Browser extension

DocBunker ships with a browser extension that adds an **Open in DocBunker**
button next to webmail attachments. Click it and the file opens in the sandbox
instead of your default viewer — no need to download first and open manually.

Works on Chrome, Edge, and Firefox. Supported webmail providers: Gmail,
Outlook (live.com / office.com / office365.com), Yahoo Mail, Proton Mail,
iCloud Mail, Zoho Mail, GMX, Web.de, Mail.com, and AOL Mail.

## What you see

When you open an email with attachments, a small button appears next to each
one. Click it and DocBunker downloads the file, sends it to the sandbox, and
shows you the result. If you enable the option, the browser's copy of the
file gets deleted once DocBunker confirms it opened.

The extension only activates on webmail sites — it doesn't work on other
sites, and it doesn't read your email content.

## What happens behind the scenes

1. You click **Open in DocBunker** on an attachment.
2. The browser downloads the file normally — the extension never sees the
   file bytes, just the download path.
3. The path goes to the DocBunker native broker (`docbunker-native-broker`),
   a small binary that validates the file: right magic bytes, right size, and
   it must be in your Downloads folder (or a custom path you set with
   `DOCBUNKER_ALLOWED_OPEN_DIR`).
4. Only after validation does the broker launch DocBunker with the file.
5. The extension waits for DocBunker to confirm it actually ingested the file
   before reporting success.

Pending transfers survive if the background script goes to sleep (Manifest V3
limitation on Chrome/Edge). You can open up to three files at once; extra
requests wait. If the same attachment is requested twice, the second request
reuses the first.

## Browser differences

| Feature | Chrome / Edge | Firefox |
| --- | --- | --- |
| Manifest version | V3 (service worker) | V2 (background script) |
| Extension ID | Fixed via `key` in manifest | Auto-generated; check `about:debugging` |
| Native host registration | Automatic on app start | Automatic on app start |
| Storage for pending transfers | `chrome.storage.session` | `chrome.storage.local` (session not available) |

The extension code is cross-browser. `service-worker.js` is used by Chrome
and Edge (Manifest V3). `background.js` (identical logic) is used by Firefox
(Manifest V2).

## Options

Right-click the extension icon and choose **Options**, or find it on the
extensions page. There's one toggle:

- **Delete the downloaded copy after opening** — removes the file from your
  Downloads folder once DocBunker confirms ingestion. Off by default.

## Permissions

The extension requests:

| Permission | Why |
| --- | --- |
| `contextMenus` | "Open attachment in DocBunker" right-click menu |
| `downloads` | Trigger the download and clean up afterwards |
| `nativeMessaging` | Talk to the DocBunker broker |
| `notifications` | Show status when something fails |
| `storage` | Remember your delete preference and track in-flight transfers |
| Host access for webmail sites | Only active on supported webmail pages |

It does **not** request: email API access, cookies, browsing history, or
access to any site other than the supported webmail providers.

The production Chrome/Edge build uses a fixed extension key so the unpacked
ID matches the native host allowlist.

## Install for testing

### Chrome / Edge

1. Run DocBunker once so it registers the native host.
2. Open `chrome://extensions` (or `edge://extensions`), enable Developer
   mode, click **Load unpacked**.
3. Select the `browser-extension/` directory from the repo.

### Firefox

1. Run DocBunker once so it registers the native host.
2. Open `about:debugging#/runtime/this-firefox`.
3. Click **Load Temporary Add-on** and select any file inside
   `browser-extension/` (e.g. `manifest.firefox.json`).

Note: Firefox temporary add-ons are removed when the browser restarts. For
persistent installs, package the extension and sign it via AMO.

## Supported webmail providers

| Provider | Domains |
| --- | --- |
| Gmail | `mail.google.com` |
| Outlook | `outlook.live.com`, `outlook.office.com`, `outlook.office365.com` |
| Yahoo Mail | `mail.yahoo.com` |
| Proton Mail | `mail.proton.me`, `mail.protonmail.com` |
| iCloud Mail | `icloud.com` |
| Zoho Mail | `mail.zoho.com` |
| GMX | `gmx.com`, `gmx.net` |
| Web.de | `web.de` |
| Mail.com | `mail.com` |
| AOL Mail | `mail.aol.com` |

## Troubleshooting

**No buttons appear.**
Make sure you're on a supported webmail site. Try reloading the tab after
installing the extension.

**"Could not download attachment" error.**
The browser might have blocked the download. Check that DocBunker has
download permissions in the extensions page and that your downloads folder
is accessible.

**"DocBunker rejected the attachment" error.**
The broker couldn't validate the file. It might be an unsupported type, too
large, or outside your Downloads folder. Check the broker logs.

**The download completes but nothing opens.**
Make sure DocBunker is running. The broker needs the app registered as a
native messaging host — run DocBunker at least once after installation.
