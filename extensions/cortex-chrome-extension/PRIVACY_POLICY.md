# Cortex Chrome Extension Privacy Policy (Draft)

Last updated: 2026-04-16

## Scope

The Cortex Chrome Extension is a local-first extension that helps users store and recall memory from their own Cortex instance.

## Data We Process

- User-entered text in the popup ("Store Memory").
- User-selected page text when they explicitly use the context-menu action.
- Optional context metadata (page title and URL) only when the user enables "Include page title + URL when storing from context menu".
- Extension configuration values (Cortex URL, agent defaults, recall defaults, timeout, preference toggles).
- API key used to authenticate with Cortex:
  - Default: session-only (cleared when the browser session ends).
  - Optional: persisted across sessions only when the user enables "Remember API key across browser restarts".

## How Data Is Used

- To send user-requested store/recall operations to the configured local Cortex endpoint.
- To render local extension UI state and results.

No advertising, profiling, sale, or third-party analytics use is performed by this extension.

## Data Sharing

- Data is sent only to the user-configured Cortex endpoint.
- In this Web Store build, endpoints are restricted to local loopback (`localhost` / `127.0.0.1`).
- No data is sent to developer-controlled remote servers by this extension.

## Storage

- Non-sensitive settings are stored in `chrome.storage.local`.
- API key is session-only by default via `chrome.storage.session`.
- If users opt in to persistent API key storage, it is stored in `chrome.storage.local`.

## User Controls

- Users can clear or change all settings via extension options.
- Users can disable optional page metadata capture at any time.
- Users can disable API key persistence at any time.

## Contact

For policy/contact details, use the maintainer contact on the Chrome Web Store listing for this extension.
