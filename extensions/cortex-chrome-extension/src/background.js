// SPDX-License-Identifier: MIT

import {
  DEFAULT_CORTEX_URL,
  isLoopbackUrl,
  normalizeCortexUrl
} from "./core.js";
import { healthcheck, recall, storeDecision } from "./cortex-client.js";
import { loadSettings, saveSettings } from "./storage.js";

const CONTEXT_MENU_ID = "cortex-store-selection";

chrome.runtime.onInstalled.addListener(async () => {
  await createContextMenu();
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== CONTEXT_MENU_ID) {
    return;
  }
  const selectedText = String(info.selectionText ?? "").trim();
  if (!selectedText) {
    return;
  }

  try {
    const settings = await loadSettings();
    await ensureOriginPermission(settings.cortexUrl);
    const context = settings.includePageMetadata
      ? [tab?.title ? `Page: ${tab.title}` : "", info.pageUrl ? `URL: ${info.pageUrl}` : ""]
          .filter(Boolean)
          .join(" | ")
      : "";
    await storeDecision(settings, {
      decision: selectedText,
      context,
      entryType: "note",
      sourceAgent: "chrome-extension-context-menu"
    });
    await setBadge("OK", "#2e7d32");
  } catch (error) {
    console.error("[cortex-extension] context-menu store failed", error);
    await setBadge("ERR", "#b71c1c");
  }
});

chrome.runtime.onMessage.addListener((request, _sender, sendResponse) => {
  handleMessage(request)
    .then((result) => sendResponse({ ok: true, result }))
    .catch((error) => {
      sendResponse({
        ok: false,
        error: String(error?.message ?? error ?? "Unknown extension error")
      });
    });
  return true;
});

async function handleMessage(request) {
  const action = String(request?.action ?? "");
  switch (action) {
    case "settings:get": {
      const settings = await loadSettings();
      const hasPermission = await hasOriginPermission(settings.cortexUrl);
      return { ...settings, hasOriginPermission: hasPermission };
    }
    case "settings:save": {
      const next = await saveSettings(request?.payload ?? {});
      await ensureOriginPermission(next.cortexUrl);
      return next;
    }
    case "cortex:health": {
      const settings = await loadSettings();
      await ensureOriginPermission(settings.cortexUrl);
      return healthcheck(settings);
    }
    case "cortex:store": {
      const settings = await loadSettings();
      await ensureOriginPermission(settings.cortexUrl);
      return storeDecision(settings, request?.payload ?? {});
    }
    case "cortex:recall": {
      const settings = await loadSettings();
      await ensureOriginPermission(settings.cortexUrl);
      return recall(settings, request?.payload ?? {});
    }
    case "permissions:ensure": {
      const url = normalizeCortexUrl(request?.payload?.cortexUrl ?? DEFAULT_CORTEX_URL);
      await ensureOriginPermission(url);
      return { granted: true, originPattern: `${new URL(url).origin}/*` };
    }
    default:
      throw new Error(`Unsupported action: ${action}`);
  }
}

async function createContextMenu() {
  try {
    await chrome.contextMenus.remove(CONTEXT_MENU_ID);
  } catch (_error) {
    // no-op: item may not exist on first install.
  }
  chrome.contextMenus.create({
    id: CONTEXT_MENU_ID,
    title: "Store selection in Cortex",
    contexts: ["selection"]
  });
}

async function hasOriginPermission(cortexUrl) {
  const normalized = normalizeCortexUrl(cortexUrl);
  return isLoopbackUrl(normalized);
}

async function ensureOriginPermission(cortexUrl) {
  const normalized = normalizeCortexUrl(cortexUrl);
  if (isLoopbackUrl(normalized)) {
    return true;
  }
  throw new Error(
    "This Chrome Web Store build only supports local Cortex URLs (localhost or 127.0.0.1)."
  );
}

async function setBadge(text, color) {
  await chrome.action.setBadgeBackgroundColor({ color });
  await chrome.action.setBadgeText({ text });
  setTimeout(() => {
    chrome.action.setBadgeText({ text: "" });
  }, 1200);
}
