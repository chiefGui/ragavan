import { spawnSync } from "node:child_process";

import { isMain } from "../entrypoint.mjs";
import {
  readReleaseVersion,
  RELEASE_VERSION_FILE,
  requireReleaseVersion,
} from "./version.mjs";

const RELEASE_PACKAGE = "ragavan";

export function resolveReleaseIdentity(metadata, releaseVersion, requestedTag) {
  if (
    !metadata ||
    typeof metadata !== "object" ||
    !Array.isArray(metadata.packages)
  ) {
    throw new Error("Cargo metadata does not contain a package list");
  }
  requireReleaseVersion(releaseVersion, RELEASE_VERSION_FILE);
  if (typeof requestedTag !== "string" || requestedTag.length === 0) {
    throw new Error("release tag cannot be empty");
  }

  const packages = metadata.packages.filter(
    (candidate) => candidate?.name === RELEASE_PACKAGE,
  );
  if (packages.length !== 1) {
    throw new Error(
      `expected exactly one Cargo package named ${RELEASE_PACKAGE}`,
    );
  }

  const { version } = packages[0];
  requireReleaseVersion(version, `${RELEASE_PACKAGE} Cargo version`);
  if (releaseVersion !== version) {
    throw new Error(
      `${RELEASE_VERSION_FILE} version ${releaseVersion} does not match Cargo version ${version}`,
    );
  }

  const tag = `v${releaseVersion}`;
  if (requestedTag !== tag) {
    throw new Error(
      `release tag ${requestedTag} does not match Cargo version ${tag}`,
    );
  }
  return { version, tag };
}

export function readReleaseIdentity(requestedTag) {
  const releaseVersion = readReleaseVersion();
  const result = spawnSync(
    "cargo",
    ["metadata", "--locked", "--no-deps", "--format-version", "1"],
    { encoding: "utf8", windowsHide: true },
  );
  if (result.error) {
    throw new Error(`could not run Cargo metadata: ${result.error.message}`, {
      cause: result.error,
    });
  }
  if (result.status !== 0) {
    const detail = result.stderr.trim();
    throw new Error(
      `Cargo metadata failed${detail.length > 0 ? `: ${detail}` : ""}`,
    );
  }

  let metadata;
  try {
    metadata = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`Cargo returned invalid metadata: ${error.message}`, {
      cause: error,
    });
  }
  return resolveReleaseIdentity(metadata, releaseVersion, requestedTag);
}

if (isMain(import.meta.url)) {
  const arguments_ = process.argv.slice(2);
  if (arguments_.length !== 1) {
    process.stderr.write("usage: identity.mjs <release-tag>\n");
    process.exitCode = 2;
  } else {
    try {
      process.stdout.write(
        `${JSON.stringify(readReleaseIdentity(arguments_[0]))}\n`,
      );
    } catch (error) {
      process.stderr.write(
        `could not resolve release identity: ${error.message}\n`,
      );
      process.exitCode = 1;
    }
  }
}
