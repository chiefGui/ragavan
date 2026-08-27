import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const sourceDirectory = path.dirname(fileURLToPath(import.meta.url));
const platformCatalog = path.join(sourceDirectory, "platforms.json");

const TARGET_PATTERN = /^[0-9A-Za-z._-]+$/;
const RUNNER_PATTERN = /^[0-9A-Za-z._-]+$/;
const RUNTIME_PATTERN = /^[a-z0-9_]+$/;
const PACKAGE_PATTERN = /^ragavan(?:-[a-z0-9]+)+$/;

function requireObject(value, meaning) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${meaning} must be an object`);
  }
  return value;
}

function requireString(value, field, index) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`platform ${index} has an invalid ${field}`);
  }
  return value;
}

function parsePlatforms(value) {
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error("the platform catalog must contain at least one platform");
  }

  const platforms = [];
  const targets = new Set();
  const runtimes = new Set();
  const packages = new Set();

  for (const [index, entry] of value.entries()) {
    const platform = requireObject(entry, `platform ${index}`);
    const npm = requireObject(platform.npm, `platform ${index} npm metadata`);
    const target = requireString(platform.target, "target", index);
    const runner = requireString(platform.runner, "runner", index);
    const binary = requireString(platform.binary, "binary", index);
    const os = requireString(npm.os, "npm os", index);
    const cpu = requireString(npm.cpu, "npm cpu", index);
    const packageName = requireString(npm.package, "npm package", index);

    if (!TARGET_PATTERN.test(target)) {
      throw new Error(`platform ${index} has an unsafe target ${target}`);
    }
    if (!RUNNER_PATTERN.test(runner)) {
      throw new Error(`platform ${index} has an unsafe runner ${runner}`);
    }
    if (!RUNTIME_PATTERN.test(os) || !RUNTIME_PATTERN.test(cpu)) {
      throw new Error(
        `platform ${index} has an unsafe npm runtime ${os}-${cpu}`,
      );
    }
    if (!PACKAGE_PATTERN.test(packageName)) {
      throw new Error(
        `platform ${index} has an unsafe npm package ${packageName}`,
      );
    }
    if (
      (os === "win32" && binary !== "ragavan.exe") ||
      (os !== "win32" && binary !== "ragavan")
    ) {
      throw new Error(`platform ${index} has an invalid binary ${binary}`);
    }

    const runtime = `${os}-${cpu}`;
    if (targets.has(target)) {
      throw new Error(`platform target ${target} is duplicated`);
    }
    if (runtimes.has(runtime)) {
      throw new Error(`npm runtime ${runtime} is duplicated`);
    }
    if (packages.has(packageName)) {
      throw new Error(`npm package ${packageName} is duplicated`);
    }

    targets.add(target);
    runtimes.add(runtime);
    packages.add(packageName);
    platforms.push({
      target,
      runner,
      binary,
      npm: { os, cpu, package: packageName },
    });
  }

  return platforms;
}

export async function readPlatforms() {
  let source;
  try {
    source = await readFile(platformCatalog, "utf8");
  } catch (error) {
    throw new Error(
      `could not read platform catalog at ${platformCatalog}: ${error.message}`,
      { cause: error },
    );
  }

  try {
    return parsePlatforms(JSON.parse(source));
  } catch (error) {
    throw new Error(
      `invalid platform catalog at ${platformCatalog}: ${error.message}`,
      { cause: error },
    );
  }
}

export function npmPlatforms(platforms) {
  return platforms.map(({ target, binary, npm }) => ({
    target,
    binary,
    ...npm,
  }));
}

function releaseMatrix(platforms) {
  return {
    include: platforms.map(({ target, runner, binary }) => ({
      target,
      runner,
      binary,
    })),
  };
}

const invokedPath = process.argv[1] && path.resolve(process.argv[1]);
if (invokedPath === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) {
    process.stderr.write("usage: platforms.mjs\n");
    process.exitCode = 2;
  } else {
    try {
      process.stdout.write(
        JSON.stringify(releaseMatrix(await readPlatforms())),
      );
    } catch (error) {
      process.stderr.write(
        `could not resolve release platforms: ${error.message}\n`,
      );
      process.exitCode = 1;
    }
  }
}
