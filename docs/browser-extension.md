# Browser extension

DocBunker ships with a Chrome extension ("DocBunker for Gmail", `browser-extension/`)
that adds an **Open in DocBunker** action to supported Gmail attachments and to
matching download links. It exists so the risky click — opening an attachment
you are not sure about — lands in the sandbox instead of your default viewer.

## How a transfer works

1. You pick **Open in DocBunker** on an attachment in Gmail.
2. Chrome performs its normal authenticated download. The extension never sees
   or transmits attachment bytes.
3. The extension hands the completed local path to the DocBunker native
   messaging host (`docbunker-native-broker`), a dedicated binary with no
   WebView and no document parsers.
4. The broker re-validates the file: supported magic signature, size, and path
   inside the user's Downloads directory (`DOCBUNKER_ALLOWED_OPEN_DIR`
   overrides it). Only then does it spawn the app.
5. The app acknowledges actual ingestion before the extension reports success;
   the options page can delete Chrome's downloaded copy afterwards
   (`deleteAfterOpen`, the only persistent setting).

Transfer metadata lives exclusively in `chrome.storage.session` and disappears
with the browser session. Pending transfers survive Manifest V3 service-worker
suspension; repeated requests are deduplicated and at most three run at once.

## Trust properties

- The browser can only reach `docbunker-native-broker`, and only for this one
  hand-off flow — see the trust boundary rules in [`SECURITY.md`](https://github.com/industrialsoftwarexyz/DocBunker/blob/main/SECURITY.md).
- The extension requests access to Gmail pages, downloads, context menus,
  notifications and the DocBunker native host. It does not use the Gmail API,
  cookies permission or browsing history.
- The production build is keyed so the unpacked extension ID is
  `lmmdckggliegiglepibblfnpaiaeojpf`, matching the native host allowlist.

Details: [README](https://github.com/industrialsoftwarexyz/DocBunker/blob/main/browser-extension/README.md)
and [privacy statement](https://github.com/industrialsoftwarexyz/DocBunker/blob/main/browser-extension/PRIVACY.md)
in `browser-extension/`.

## Install for testing

1. Build and run DocBunker once so it registers the per-user native host.
2. Open `chrome://extensions`, enable Developer mode, choose **Load unpacked**.
3. Select the repository's `browser-extension/` directory.
