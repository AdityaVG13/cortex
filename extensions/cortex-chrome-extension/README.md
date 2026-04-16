# Cortex Chrome Extension (MV3)

`cortex-chrome-extension` is a local-first Manifest V3 companion for storing and recalling memory from browser AI workflows.

## What It Does

- Saves selected text via context-menu (`Store selection in Cortex`)
- Stores manual notes from popup UI
- Recalls memory snippets from popup UI
- Keeps all API calls behind extension background service worker

## Security Posture

- No remotely hosted executable code
- No content-script scraping or DOM injection
- Local loopback host access only (`localhost`/`127.0.0.1`)
- No broad or wildcard host permissions
- API key defaults to session-only storage (optional persistence toggle)
- Context-menu store captures selected text only by default (page metadata is opt-in)

## Load Unpacked

1. Open `chrome://extensions`
2. Enable `Developer mode`
3. Click `Load unpacked`
4. Select this folder (`extensions/cortex-chrome-extension`)
5. Open extension options and configure:
   - Cortex URL
   - API key
   - Optional agent/budget defaults

## Local Tests

```bash
node --test tests/core.test.mjs
```
