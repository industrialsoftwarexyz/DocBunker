# DocBunker for Gmail

The extension adds **Open in DocBunker** to supported Gmail attachments and a
matching link context-menu action. Chrome performs the authenticated download;
the extension sends only the completed local path to the DocBunker native
messaging host — a dedicated `docbunker-native-broker` binary with no WebView
and no document parsers. The broker re-validates the file (supported magic
signature, size, path inside the user's Downloads directory) before handing it
to the DocBunker app.

Pending transfers survive Manifest V3 service-worker suspension. DocBunker
acknowledges actual ingestion before the extension reports success, and the
options page can delete Chrome's downloaded copy after that acknowledgment.
Repeated requests are deduplicated, at most three transfers can run at once,
and subsequent files are forwarded to the existing DocBunker window.

## Install for testing

1. Install and run DocBunker once so it registers the per-user native host.
2. Open `chrome://extensions`, enable Developer mode and choose **Load unpacked**.
3. Select this `browser-extension` directory.

The fixed manifest key gives this unpacked build the extension ID
`lmmdckggliegiglepibblfnpaiaeojpf`, matching the native host allowlist.

Production distribution should publish this exact keyed extension in the
Chrome Web Store. It requests access only to Gmail pages, downloads, context
menus, notifications and the DocBunker native messaging host. It does not use
the Gmail API, cookies permission or browsing history.
