let _check = null;
let _relaunch = null;

async function loadPlugins() {
  try {
    const updater = await import("@tauri-apps/plugin-updater");
    const process = await import("@tauri-apps/plugin-process");
    _check = updater.check;
    _relaunch = process.relaunch;
  } catch {
    // Running outside Tauri (web dev) -- plugins unavailable
  }
}

loadPlugins();

export async function checkForUpdates() {
  if (!_check) return null;
  try {
    const update = await _check();
    if (!update) return null;
    return {
      version: update.version,
      notes: update.body || "",
      date: update.date || "",
      _update: update,
    };
  } catch {
    return null;
  }
}

export async function installUpdate(updateInfo) {
  if (!updateInfo?._update) return;
  await updateInfo._update.downloadAndInstall();
  if (_relaunch) await _relaunch();
}
