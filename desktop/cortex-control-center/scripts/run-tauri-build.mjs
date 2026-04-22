import { spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import { access, readFile } from "node:fs/promises";
import { dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectDir = resolve(scriptDir, "..");

const defaultKeyFile = resolve(projectDir, ".secrets", "tauri", "updater-private.key");
const defaultKeyPasswordFile = resolve(projectDir, ".secrets", "tauri", "updater-private.key.password");

const keyPathEnvNames = ["TAURI_SIGNING_PRIVATE_KEY_FILE", "TAURI_SIGNING_PRIVATE_KEY_PATH"];
const passwordPathEnvNames = [
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD_FILE",
  "TAURI_SIGNING_PRIVATE_KEY_PASS_FILE",
];

function resolvePath(input) {
  if (!input || !input.trim()) return "";
  const value = input.trim();
  return isAbsolute(value) ? value : resolve(projectDir, value);
}

async function canRead(path) {
  if (!path) return false;
  try {
    await access(path, fsConstants.R_OK);
    return true;
  } catch {
    return false;
  }
}

async function resolveTauriCliPath() {
  const candidates = [
    resolve(projectDir, "node_modules", "@tauri-apps", "cli", "tauri.js"),
    resolve(projectDir, "node_modules", "@tauri-apps", "cli", "bin", "tauri.js"),
  ];

  for (const candidate of candidates) {
    if (await canRead(candidate)) {
      return candidate;
    }
  }

  return "";
}

async function loadKey() {
  const inlineValue = process.env.TAURI_SIGNING_PRIVATE_KEY || "";
  if (inlineValue.trim()) {
    const possiblePath = resolvePath(inlineValue);
    if (await canRead(possiblePath)) {
      const key = await readFile(possiblePath, "utf8");
      return { key, source: possiblePath };
    }
    return { key: inlineValue, source: "TAURI_SIGNING_PRIVATE_KEY (inline value)" };
  }

  const candidates = [
    ...keyPathEnvNames.map((name) => ({ name, path: resolvePath(process.env[name] || "") })),
    { name: "default", path: defaultKeyFile },
  ];

  for (const candidate of candidates) {
    if (await canRead(candidate.path)) {
      const key = await readFile(candidate.path, "utf8");
      return { key, source: candidate.path };
    }
  }

  return null;
}

async function loadPassword() {
  if (process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
    return process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;
  }

  const candidates = [
    ...passwordPathEnvNames.map((name) => resolvePath(process.env[name] || "")),
    defaultKeyPasswordFile,
  ];

  for (const candidate of candidates) {
    if (await canRead(candidate)) {
      return (await readFile(candidate, "utf8")).trim();
    }
  }

  return "";
}

function printMissingKeyError() {
  console.error("[desktop:build] Missing Tauri updater signing key.");
  console.error("[desktop:build] Checked:");
  console.error("  - TAURI_SIGNING_PRIVATE_KEY (inline value or file path)");
  console.error("  - TAURI_SIGNING_PRIVATE_KEY_FILE");
  console.error("  - TAURI_SIGNING_PRIVATE_KEY_PATH");
  console.error(`  - ${defaultKeyFile}`);
  console.error("[desktop:build] Configure one of those locations, then retry.");
}

async function main() {
  const tauriCli = await resolveTauriCliPath();
  if (!tauriCli) {
    console.error("[desktop:build] Missing Tauri CLI. Run npm ci first.");
    process.exit(1);
  }

  const loadedKey = await loadKey();
  if (!loadedKey || !loadedKey.key.trim()) {
    printMissingKeyError();
    process.exit(1);
  }

  const signingPassword = await loadPassword();
  const args = [tauriCli, "build", ...process.argv.slice(2)];
  const env = {
    ...process.env,
    TAURI_SIGNING_PRIVATE_KEY: loadedKey.key,
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD:
      signingPassword || process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || "",
  };

  console.log(`[desktop:build] Using updater signing key from ${loadedKey.source}`);

  const child = spawn(process.execPath, args, {
    cwd: projectDir,
    env,
    stdio: "inherit",
    windowsHide: true,
  });

  child.once("error", (error) => {
    console.error(`[desktop:build] Failed to start Tauri build: ${error.message}`);
    process.exit(1);
  });

  child.once("exit", (code, signal) => {
    if (signal) {
      console.error(`[desktop:build] Tauri build terminated by signal ${signal}`);
      process.exit(1);
    }
    process.exit(code ?? 1);
  });
}

main().catch((error) => {
  console.error(`[desktop:build] ${error.message}`);
  process.exit(1);
});
