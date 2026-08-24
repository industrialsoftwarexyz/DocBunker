# Browser extension

DocBunker ships with a Chrome extension that adds an **Open in DocBunker**
button next to Gmail attachments. Click it and the file opens in the sandbox
instead of your default viewer — no need to download first and open manually.

## What you see

When you open an email with attachments in Gmail, a small button appears next
to each one. Click it and DocBunker downloads the file, sends it to the
sandbox, and shows you the result. If you enable the option, Chrome's copy of
the file gets deleted once DocBunker confirms it opened.

The extension only activates on `mail.google.com`. It doesn't work on other
sites, and it doesn't read your email content — just watches for attachment
links.

## What happens behind the scenes

1. You click **Open in DocBunker** on an attachment.
2. Chrome downloads the file normally — the extension never sees the file
   bytes, just the download path.
3. The path goes to the DocBunker native broker (`docbunker-native-broker`),
   a small binary that validates the file: right magic bytes, right size, and
   it must be in your Downloads folder (or a custom path you set with
   `DOCBUNKER_ALLOWED_OPEN_DIR`).
4. Only after validation does the broker launch DocBunker with the file.
5. The extension waits for DocBunker to confirm it actually ingested the file
   before reporting success.

Pending transfers survive if the service worker goes to sleep (Manifest V3
limitation). You can open up to three files at once; extra requests wait. If
the same attachment is requested twice, the second request reuses the first.

## Options

Right-click the extension icon and choose **Options**, or find it on
`chrome://extensions`. There's one toggle:

- **Delete Chrome's downloaded copy after opening** — removes the file from
  your Downloads folder once DocBunker confirms ingestion. Off by default.

## Permissions

The extension requests:

| Permission | Why |
| --- | --- |
| `contextMenus` | "Open attachment in DocBunker" right-click menu |
| `downloads` | Trigger the download and clean up afterwards |
| `nativeMessaging` | Talk to the DocBunker broker |
| `notifications` | Show status when something fails |
| `storage` | Remember your delete preference and track in-flight transfers |
| `host_permissions` for `mail.google.com` | Only active on Gmail pages |

It does **not** request: Gmail API, cookies, browsing history, or access to
any site other than Gmail.

The production build uses a fixed extension key so the unpacked ID matches
the native host allowlist (`lmmdckggliegiglepibblfnpaiaeojpf`).

## Install for testing

1. Run DocBunker once so it registers the native host.
2. Open `chrome://extensions`, enable Developer mode, click **Load unpacked**.
3. Select the `browser-extension/` directory from the repo.

The extension icon appears in your toolbar. Open Gmail and you'll see the
buttons next to attachments.

## Troubleshooting

**No buttons appear in Gmail.**
Make sure you're on `mail.google.com` (not a cached or offline version). Try
reloading the tab after installing the extension.

**"Could not download attachment" error.**
Chrome might have blocked the download. Check that DocBunker has download
permissions in `chrome://extensions` and that your downloads folder is
accessible.

**"DocBunker rejected the attachment" error.**
The broker couldn't validate the file. It might be an unsupported type, too
large, or outside your Downloads folder. Check the broker logs.

**The download completes but nothing opens.**
Make sure DocBunker is running. The broker needs the app registered as a
native messaging host — run DocBunker at least once after installation.
