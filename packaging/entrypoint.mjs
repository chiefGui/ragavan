import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

export function isMain(moduleUrl, invokedPath = process.argv[1]) {
  if (!invokedPath) {
    return false;
  }

  return realpathSync(invokedPath) === realpathSync(fileURLToPath(moduleUrl));
}
