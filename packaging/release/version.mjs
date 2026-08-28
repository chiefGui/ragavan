import { lstatSync, readFileSync } from "node:fs";
import path from "node:path";

export const RELEASE_VERSION_FILE = "VERSION";

const NUMERIC_IDENTIFIER = String.raw`(?:0|[1-9][0-9]*)`;
const NON_NUMERIC_IDENTIFIER = String.raw`(?:[0-9]*[A-Za-z-][0-9A-Za-z-]*)`;
const PRE_RELEASE_IDENTIFIER = String.raw`(?:${NUMERIC_IDENTIFIER}|${NON_NUMERIC_IDENTIFIER})`;
const BUILD_IDENTIFIER = String.raw`(?:[0-9A-Za-z-]+)`;
const VERSION_PATTERN = new RegExp(
  String.raw`^${NUMERIC_IDENTIFIER}\.${NUMERIC_IDENTIFIER}\.${NUMERIC_IDENTIFIER}(?:-${PRE_RELEASE_IDENTIFIER}(?:\.${PRE_RELEASE_IDENTIFIER})*)?(?:\+${BUILD_IDENTIFIER}(?:\.${BUILD_IDENTIFIER})*)?$`,
);

export function requireReleaseVersion(version, source = "release version") {
  if (typeof version !== "string" || !VERSION_PATTERN.test(version)) {
    throw new Error(`${source} does not contain a valid SemVer version`);
  }
  return version;
}

export function readReleaseVersion(root = ".") {
  const versionPath = path.join(root, RELEASE_VERSION_FILE);
  let version;
  try {
    if (!lstatSync(versionPath).isFile()) {
      throw new Error("path is not a regular file");
    }
    version = readFileSync(versionPath, "utf8").trim();
  } catch (error) {
    throw new Error(
      `could not read ${RELEASE_VERSION_FILE}: ${error.message}`,
      { cause: error },
    );
  }
  return requireReleaseVersion(version, RELEASE_VERSION_FILE);
}
