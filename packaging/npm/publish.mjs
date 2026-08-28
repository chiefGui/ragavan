import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { npmPlatforms, readPlatforms } from "../platforms.mjs";
import {
  distributionTag,
  PUBLICATION_MANIFEST,
  PUBLICATION_SCHEMA,
  releasePackages,
  requireVersion,
} from "./family.mjs";
import { requireDirectory, requireFile } from "./filesystem.mjs";
import { runNpm } from "./npm.mjs";

const NPM_REGISTRY = "https://registry.npmjs.org/";

async function sha512Integrity(filePath) {
  const hash = createHash("sha512");
  for await (const chunk of createReadStream(filePath)) {
    hash.update(chunk);
  }
  return `sha512-${hash.digest("base64")}`;
}

function describeArtifactDifference(expected, actual) {
  const missing = expected.filter((name) => !actual.has(name));
  const unexpected = [...actual].filter((name) => !expected.includes(name));
  const details = [];
  if (missing.length > 0) {
    details.push(`missing ${missing.join(", ")}`);
  }
  if (unexpected.length > 0) {
    details.push(`unexpected ${unexpected.join(", ")}`);
  }
  return details.join("; ");
}

export async function loadPublication(version, artifacts) {
  requireVersion(version);
  if (typeof artifacts !== "string" || artifacts.length === 0) {
    throw new Error("npm artifact directory cannot be empty");
  }

  const artifactDirectory = path.resolve(artifacts);
  await requireDirectory(artifactDirectory, "npm artifact directory");

  const expectedPackages = releasePackages(
    npmPlatforms(await readPlatforms()),
    version,
  );
  const expectedTarballs = expectedPackages.map(
    (releasePackage) => releasePackage.tarball,
  );
  const actualTarballs = new Set(
    (await readdir(artifactDirectory, { withFileTypes: true }))
      .filter((entry) => entry.name.endsWith(".tgz"))
      .map((entry) => entry.name),
  );
  if (
    expectedTarballs.length !== actualTarballs.size ||
    expectedTarballs.some((name) => !actualTarballs.has(name))
  ) {
    throw new Error(
      `npm artifact set is invalid: ${describeArtifactDifference(expectedTarballs, actualTarballs)}`,
    );
  }

  const manifestPath = path.join(artifactDirectory, PUBLICATION_MANIFEST);
  await requireFile(manifestPath, "npm publication manifest");
  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  } catch (error) {
    throw new Error(`npm publication manifest is invalid: ${error.message}`, {
      cause: error,
    });
  }
  if (
    !manifest ||
    typeof manifest !== "object" ||
    Array.isArray(manifest) ||
    manifest.schema !== PUBLICATION_SCHEMA ||
    manifest.version !== version ||
    !Array.isArray(manifest.packages) ||
    manifest.packages.length !== expectedPackages.length
  ) {
    throw new Error("npm publication manifest does not describe this release");
  }

  const packages = [];
  for (const [index, expected] of expectedPackages.entries()) {
    const entry = manifest.packages[index];
    if (
      !entry ||
      typeof entry !== "object" ||
      Array.isArray(entry) ||
      entry.kind !== expected.kind ||
      entry.name !== expected.name ||
      entry.target !== expected.target ||
      entry.tarball !== expected.tarball ||
      typeof entry.integrity !== "string" ||
      !entry.integrity.startsWith("sha512-")
    ) {
      throw new Error(
        `npm publication manifest has an invalid package at index ${index}`,
      );
    }

    const releasePackage = {
      ...expected,
      version,
      path: path.join(artifactDirectory, expected.tarball),
      integrity: entry.integrity,
    };
    await requireFile(
      releasePackage.path,
      `npm tarball for ${releasePackage.name}`,
    );
    const actualIntegrity = await sha512Integrity(releasePackage.path);
    if (actualIntegrity !== releasePackage.integrity) {
      throw new Error(
        `npm tarball integrity does not match for ${releasePackage.name}@${version}`,
      );
    }
    packages.push(releasePackage);
  }

  const tag = distributionTag(version);
  return { version, tag, packages };
}

async function publishedIntegrity(releasePackage) {
  const packageName = encodeURIComponent(releasePackage.name);
  const version = encodeURIComponent(releasePackage.version);
  const url = new URL(`${packageName}/${version}`, NPM_REGISTRY);

  let response;
  try {
    response = await fetch(url, {
      cache: "no-store",
      headers: { accept: "application/json" },
      redirect: "error",
    });
  } catch (error) {
    throw new Error(
      `could not inspect ${releasePackage.name}@${releasePackage.version} on npm: ${error.message}`,
      { cause: error },
    );
  }
  if (response.status === 404) {
    return undefined;
  }
  if (!response.ok) {
    throw new Error(
      `could not inspect ${releasePackage.name}@${releasePackage.version} on npm: registry returned HTTP ${response.status}`,
    );
  }

  let manifest;
  try {
    manifest = await response.json();
  } catch (error) {
    throw new Error(
      `npm returned invalid metadata for ${releasePackage.name}@${releasePackage.version}`,
      { cause: error },
    );
  }
  const integrity = manifest?.dist?.integrity;
  if (typeof integrity !== "string") {
    throw new Error(
      `npm did not report an integrity for ${releasePackage.name}@${releasePackage.version}`,
    );
  }
  return integrity;
}

async function publishTarball(releasePackage, tag) {
  runNpm(
    [
      "publish",
      releasePackage.path,
      "--ignore-scripts",
      "--access=public",
      `--tag=${tag}`,
      `--registry=${NPM_REGISTRY}`,
    ],
    { stdio: "inherit" },
  );
}

const npmRegistry = { publishedIntegrity, publish: publishTarball };

export async function publishPublication(publication, registry = npmRegistry) {
  const existing = await Promise.all(
    publication.packages.map((releasePackage) =>
      registry.publishedIntegrity(releasePackage),
    ),
  );

  for (const [index, integrity] of existing.entries()) {
    if (
      integrity !== undefined &&
      integrity !== publication.packages[index].integrity
    ) {
      const releasePackage = publication.packages[index];
      throw new Error(
        `npm already contains different contents for ${releasePackage.name}@${releasePackage.version}`,
      );
    }
  }

  const results = [];
  for (const [index, releasePackage] of publication.packages.entries()) {
    if (existing[index] === releasePackage.integrity) {
      results.push({ name: releasePackage.name, status: "unchanged" });
      continue;
    }

    await registry.publish(releasePackage, publication.tag);
    results.push({ name: releasePackage.name, status: "published" });
  }
  return results;
}

export async function publishFamily(version, artifacts) {
  const publication = await loadPublication(version, artifacts);
  return publishPublication(publication);
}

const invokedPath = process.argv[1] && path.resolve(process.argv[1]);
if (invokedPath === fileURLToPath(import.meta.url)) {
  const arguments_ = process.argv.slice(2);
  if (arguments_.length !== 2) {
    process.stderr.write("usage: publish.mjs <version> <artifact-directory>\n");
    process.exitCode = 2;
  } else {
    try {
      const results = await publishFamily(...arguments_);
      for (const result of results) {
        process.stdout.write(
          `${result.status} ${result.name}@${arguments_[0]}\n`,
        );
      }
    } catch (error) {
      process.stderr.write(
        `could not publish Ragavan to npm: ${error.message}\n`,
      );
      process.exitCode = 1;
    }
  }
}
