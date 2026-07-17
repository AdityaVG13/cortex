import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SRC = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const CSS_FILE = "styles/index.css";

export function readBundledStyles() {
  return fs.readFileSync(path.join(SRC, CSS_FILE), "utf8");
}
