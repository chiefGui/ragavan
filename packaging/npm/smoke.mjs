import { spawnSync } from "node:child_process";
import { lstat, stat } from "node:fs/promises";
import path from "node:path";

import { npmPlatforms, readPlatforms } from "../platforms.mjs";
import { runNpm } from "./npm.mjs";

const VERSION_PATTERN = /^[0-9A-Za-z.+-]+$/;

async function pathType(filePath) {
  try {
    return await stat(filePath);
  } catch (error) {
    if (error && error.code === "ENOENT") {
      return undefined;
    }
    throw error;
  }
}

async function requireFile(filePath, meaning) {
  const metadata = await pathType(filePath);
  if (!metadata || !metadata.isFile()) {
    throw new Error(`${meaning} was not found at ${filePath}`);
  }
}

async function requireDirectory(directory, meaning) {
  const metadata = await pathType(directory);
  if (!metadata || !metadata.isDirectory()) {
    throw new Error(`${meaning} was not found at ${directory}`);
  }
}

async function requireAbsent(filePath, meaning) {
  try {
    await lstat(filePath);
  } catch (error) {
    if (error && error.code === "ENOENT") {
      return;
    }
    throw error;
  }
  throw new Error(`${meaning} already exists at ${filePath}`);
}

function runInstalledCommand(executable) {
  const options = {
    encoding: "utf8",
    windowsHide: true,
  };
  if (process.platform !== "win32") {
    return spawnSync(executable, ["--version"], options);
  }

  return spawnSync(
    "powershell.exe",
    [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "& $env:RAGAVAN_NPM_EXECUTABLE --version",
    ],
    {
      ...options,
      env: { ...process.env, RAGAVAN_NPM_EXECUTABLE: executable },
    },
  );
}

async function smoke(version, target, artifacts, prefix) {
  if (!VERSION_PATTERN.test(version)) {
    throw new Error(`invalid npm package version ${version}`);
  }
  if (artifacts.length === 0) {
    throw new Error("npm artifact directory cannot be empty");
  }
  if (prefix.length === 0) {
    throw new Error("npm smoke installation prefix cannot be empty");
  }

  const platform = npmPlatforms(await readPlatforms()).find(
    (candidate) => candidate.target === target,
  );
  if (!platform) {
    throw new Error(`unsupported release target ${target}`);
  }
  if (platform.os !== process.platform || platform.cpu !== process.arch) {
    throw new Error(
      `release target ${target} requires ${platform.os}-${platform.cpu}, but the runner is ${process.platform}-${process.arch}`,
    );
  }

  const artifactDirectory = path.resolve(artifacts);
  const installationPrefix = path.resolve(prefix);
  const rootTarball = path.join(artifactDirectory, `ragavan-${version}.tgz`);
  const platformTarball = path.join(
    artifactDirectory,
    `${platform.package}-${version}.tgz`,
  );
  await requireDirectory(artifactDirectory, "npm artifact directory");
  await requireFile(rootTarball, "root npm package");
  await requireFile(platformTarball, `npm package for ${target}`);
  await requireAbsent(installationPrefix, "npm smoke installation prefix");

  const environment = {
    ...process.env,
    npm_config_cache: path.join(installationPrefix, ".npm-cache"),
    npm_config_offline: "true",
    npm_config_registry: "http://127.0.0.1:9",
  };
  runNpm(
    [
      "install",
      "--global",
      "--prefix",
      installationPrefix,
      "--omit=optional",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      platformTarball,
      rootTarball,
    ],
    { env: environment },
  );

  const executable =
    process.platform === "win32"
      ? path.join(installationPrefix, "ragavan.cmd")
      : path.join(installationPrefix, "bin", "ragavan");

  const result = runInstalledCommand(executable);
  if (result.error) {
    throw new Error(
      `could not start installed Ragavan: ${result.error.message}`,
      {
        cause: result.error,
      },
    );
  }
  if (result.status !== 0) {
    const detail = result.stderr.trim() || result.stdout.trim();
    throw new Error(
      `installed Ragavan failed with exit code ${result.status}${detail ? `: ${detail}` : ""}`,
    );
  }

  const expected = `ragavan ${version}`;
  const actual = result.stdout.trim();
  if (actual !== expected) {
    throw new Error(`expected ${expected}, received ${actual || "no output"}`);
  }
  process.stdout.write(`${actual}\n`);
}

const arguments_ = process.argv.slice(2);
if (arguments_.length !== 4) {
  process.stderr.write(
    "usage: smoke.mjs <version> <target> <artifact-directory> <installation-prefix>\n",
  );
  process.exitCode = 2;
} else {
  try {
    await smoke(...arguments_);
  } catch (error) {
    process.stderr.write(`npm smoke test failed: ${error.message}\n`);
    process.exitCode = 1;
  }
}
