import { lstatSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

import { isMain } from "../entrypoint.mjs";
import { readReleaseVersion, requireReleaseVersion } from "./version.mjs";

const ROOT_PACKAGE = "ragavan";
const PACKAGE_HEADER = /^\[\[package\]\][ \t]*\r?$/gm;

function readRegularFile(filePath, meaning) {
  if (!lstatSync(filePath).isFile()) {
    throw new Error(`${meaning} is not a regular file`);
  }
  return readFileSync(filePath, "utf8");
}

function workspacePackageNames(metadata) {
  if (
    !metadata ||
    typeof metadata !== "object" ||
    !Array.isArray(metadata.packages) ||
    !Array.isArray(metadata.workspace_members)
  ) {
    throw new Error("Cargo metadata does not describe workspace members");
  }

  const packagesById = new Map();
  for (const candidate of metadata.packages) {
    if (
      !candidate ||
      typeof candidate !== "object" ||
      typeof candidate.id !== "string" ||
      typeof candidate.name !== "string" ||
      packagesById.has(candidate.id)
    ) {
      throw new Error("Cargo metadata contains an invalid package");
    }
    packagesById.set(candidate.id, candidate.name);
  }

  const names = new Set();
  for (const id of metadata.workspace_members) {
    const name = packagesById.get(id);
    if (name === undefined) {
      throw new Error(`Cargo metadata is missing workspace member ${id}`);
    }
    if (names.has(name)) {
      throw new Error(`Cargo workspace contains duplicate package ${name}`);
    }
    names.add(name);
  }
  if (!names.has(ROOT_PACKAGE)) {
    throw new Error(`Cargo workspace does not contain ${ROOT_PACKAGE}`);
  }
  return names;
}

function workspaceVersion(manifest) {
  let section = "";
  let version;
  for (const line of manifest.split(/\r?\n/)) {
    const header = line.match(/^\s*\[([^\]\r\n]+)\]\s*(?:#.*)?$/);
    if (header) {
      section = header[1].trim();
      continue;
    }
    if (section !== "workspace.package") {
      continue;
    }

    const candidate = line.match(/^\s*version\s*=\s*"([^"\r\n]+)"\s*(?:#.*)?$/);
    if (!candidate) {
      continue;
    }
    if (version !== undefined) {
      throw new Error(
        "Cargo.toml contains multiple workspace package versions",
      );
    }
    version = candidate[1];
  }
  if (version === undefined) {
    throw new Error("Cargo.toml does not contain workspace.package.version");
  }
  return requireReleaseVersion(version, "Cargo.toml workspace package version");
}

function lockString(block, field) {
  const pattern = new RegExp(
    `^${field}[ \\t]*=[ \\t]*"([^"\\r\\n]+)"[ \\t]*\\r?$`,
    "gm",
  );
  const matches = [...block.matchAll(pattern)];
  if (matches.length !== 1) {
    throw new Error(`Cargo.lock package has an invalid ${field} field`);
  }
  return matches[0][1];
}

function synchronizeLockfile(lockfile, workspacePackages, releaseVersion) {
  const headers = [...lockfile.matchAll(PACKAGE_HEADER)];
  if (headers.length === 0) {
    throw new Error("Cargo.lock does not contain package entries");
  }

  const found = new Set();
  let cursor = 0;
  let synchronized = "";
  for (const [index, header] of headers.entries()) {
    const start = header.index;
    const end = headers[index + 1]?.index ?? lockfile.length;
    let block = lockfile.slice(start, end);
    const name = lockString(block, "name");

    synchronized += lockfile.slice(cursor, start);
    if (workspacePackages.has(name)) {
      if (/^source[ \t]*=/m.test(block)) {
        throw new Error(`Cargo.lock workspace package ${name} has a source`);
      }
      if (found.has(name)) {
        throw new Error(
          `Cargo.lock contains duplicate workspace package ${name}`,
        );
      }
      found.add(name);
      lockString(block, "version");
      block = block.replace(
        /^version([ \t]*=[ \t]*)"[^"\r\n]+"([ \t]*\r?)$/m,
        `version$1"${releaseVersion}"$2`,
      );
    }
    synchronized += block;
    cursor = end;
  }
  synchronized += lockfile.slice(cursor);

  const missing = [...workspacePackages].filter((name) => !found.has(name));
  if (missing.length > 0) {
    throw new Error(
      `Cargo.lock is missing workspace packages: ${missing.sort().join(", ")}`,
    );
  }
  return synchronized;
}

export function synchronizeCargoState({
  metadata,
  manifest,
  lockfile,
  releaseVersion,
}) {
  const version = requireReleaseVersion(releaseVersion);
  const manifestVersion = workspaceVersion(manifest);
  if (manifestVersion !== version) {
    throw new Error(
      `VERSION ${version} does not match Cargo.toml workspace package version ${manifestVersion}`,
    );
  }

  const packages = workspacePackageNames(metadata);
  return {
    lockfile: synchronizeLockfile(lockfile, packages, version),
    packageCount: packages.size,
    version,
  };
}

export function synchronizeCargoRoot(metadata, root) {
  const candidateRoot = path.resolve(root);
  const lockPath = path.join(candidateRoot, "Cargo.lock");
  const before = readRegularFile(lockPath, "Cargo.lock");
  const result = synchronizeCargoState({
    metadata,
    manifest: readRegularFile(
      path.join(candidateRoot, "Cargo.toml"),
      "Cargo.toml",
    ),
    lockfile: before,
    releaseVersion: readReleaseVersion(candidateRoot),
  });
  if (result.lockfile !== before) {
    writeFileSync(lockPath, result.lockfile, "utf8");
  }
  return { ...result, changed: result.lockfile !== before };
}

if (isMain(import.meta.url)) {
  const arguments_ = process.argv.slice(2);
  if (arguments_.length !== 1) {
    process.stderr.write("usage: cargo-lock.mjs <candidate-root>\n");
    process.exitCode = 2;
  } else {
    try {
      const metadata = JSON.parse(readFileSync(0, "utf8"));
      const { changed, packageCount, version } = synchronizeCargoRoot(
        metadata,
        arguments_[0],
      );
      process.stdout.write(
        `${JSON.stringify({ version, package_count: packageCount, changed })}\n`,
      );
    } catch (error) {
      process.stderr.write(
        `could not synchronize Cargo release state: ${error.message}\n`,
      );
      process.exitCode = 1;
    }
  }
}
