import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { npmPlatforms, readPlatforms } from "../platforms.mjs";
import { runNpm } from "./npm.mjs";
import { packageFamily } from "./package.mjs";

const platforms = npmPlatforms(await readPlatforms());
const smokeExecutable = fileURLToPath(new URL("./smoke.mjs", import.meta.url));
const version = "9.8.7-test.4";

async function temporaryDirectory(testContext) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ragavan-npm-test-"));
  testContext.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

async function prepareInputs(directory, omittedTarget) {
  await mkdir(directory);
  for (const platform of platforms) {
    if (platform.target === omittedTarget) {
      continue;
    }

    const destinationDirectory = path.join(
      directory,
      `binary-${platform.target}`,
    );
    await mkdir(destinationDirectory);
    await copyFile(
      process.execPath,
      path.join(destinationDirectory, platform.binary),
    );
  }
}

function npmEnvironment(cache) {
  return {
    ...process.env,
    npm_config_cache: cache,
    npm_config_offline: "true",
    npm_config_registry: "http://127.0.0.1:9",
  };
}

function hostPlatform() {
  const platform = platforms.find(
    (candidate) =>
      candidate.os === process.platform && candidate.cpu === process.arch,
  );
  assert.ok(
    platform,
    `the test host ${process.platform}-${process.arch} must be supported`,
  );
  return platform;
}

function installedLauncher(prefix, globalRoot) {
  return {
    source: path.join(globalRoot, "ragavan", "bin", "ragavan.cjs"),
    shim:
      process.platform === "win32"
        ? path.join(prefix, "ragavan.cmd")
        : path.join(prefix, "bin", "ragavan"),
  };
}

function executeLauncher(launcher, arguments_) {
  return spawnSync(process.execPath, [launcher, ...arguments_], {
    encoding: "utf8",
    windowsHide: true,
  });
}

function executeShim(shim) {
  if (process.platform === "win32") {
    return spawnSync(
      "powershell.exe",
      [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "& $env:RAGAVAN_TEST_SHIM --version",
      ],
      {
        encoding: "utf8",
        env: { ...process.env, RAGAVAN_TEST_SHIM: shim },
        windowsHide: true,
      },
    );
  }

  return spawnSync(shim, ["--version"], { encoding: "utf8" });
}

test("packages and globally installs the complete npm family", async (context) => {
  const temporary = await temporaryDirectory(context);
  const input = path.join(temporary, "input");
  const output = path.join(temporary, "output");
  const prefix = path.join(temporary, "prefix");
  const cache = path.join(temporary, "cache");
  await prepareInputs(input);

  const tarballs = await packageFamily(version, input, output);
  const expectedNames = [
    ...platforms.map((platform) => `${platform.package}-${version}.tgz`),
    `ragavan-${version}.tgz`,
  ].sort();
  assert.deepEqual(
    tarballs.map((tarball) => path.basename(tarball)).sort(),
    expectedNames,
  );
  assert.deepEqual((await readdir(output)).sort(), expectedNames);

  const currentPlatform = hostPlatform();

  const rootTarball = path.join(output, `ragavan-${version}.tgz`);
  const platformTarball = path.join(
    output,
    `${currentPlatform.package}-${version}.tgz`,
  );
  const environment = npmEnvironment(cache);
  runNpm(
    [
      "install",
      "--global",
      "--prefix",
      prefix,
      "--omit=optional",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      platformTarball,
      rootTarball,
    ],
    { env: environment },
  );

  const globalRoot = runNpm(["root", "--global", "--prefix", prefix], {
    env: environment,
  }).trim();
  const rootPackage = JSON.parse(
    await readFile(path.join(globalRoot, "ragavan", "package.json"), "utf8"),
  );
  assert.equal(rootPackage.version, version);
  assert.deepEqual(rootPackage.bin, { ragavan: "bin/ragavan.cjs" });
  assert.equal(Object.hasOwn(rootPackage, "scripts"), false);
  assert.deepEqual(
    rootPackage.optionalDependencies,
    Object.fromEntries(
      platforms.map((platform) => [platform.package, version]),
    ),
  );
  const runtimePlatforms = JSON.parse(
    await readFile(path.join(globalRoot, "ragavan", "platforms.json"), "utf8"),
  );
  assert.deepEqual(
    runtimePlatforms,
    platforms.map(({ os, cpu, package: packageName, binary }) => ({
      os,
      cpu,
      package: packageName,
      binary,
    })),
  );
  assert.deepEqual(
    (await readdir(globalRoot)).sort(),
    ["ragavan", currentPlatform.package].sort(),
  );

  const platformPackage = JSON.parse(
    await readFile(
      path.join(globalRoot, currentPlatform.package, "package.json"),
      "utf8",
    ),
  );
  assert.equal(platformPackage.version, version);
  assert.deepEqual(platformPackage.os, [currentPlatform.os]);
  assert.deepEqual(platformPackage.cpu, [currentPlatform.cpu]);
  assert.equal(Object.hasOwn(platformPackage, "scripts"), false);

  const launcher = installedLauncher(prefix, globalRoot);
  assert.equal(existsSync(launcher.shim), true);
  const shimResult = executeShim(launcher.shim);
  assert.equal(shimResult.status, 0, shimResult.stderr);
  assert.equal(shimResult.stdout.trim(), process.version);

  const nativeExecutable = path.join(
    globalRoot,
    currentPlatform.package,
    "bin",
    currentPlatform.binary,
  );
  const identityResult = executeLauncher(launcher.source, [
    "-e",
    "process.stdout.write(process.execPath)",
  ]);
  assert.equal(identityResult.status, 0, identityResult.stderr);
  assert.equal(
    path.normalize(identityResult.stdout),
    path.normalize(nativeExecutable),
  );

  const exitResult = executeLauncher(launcher.source, [
    "-e",
    "process.exit(37)",
  ]);
  assert.equal(exitResult.status, 37);

  await rm(path.join(globalRoot, currentPlatform.package), {
    recursive: true,
    force: true,
  });
  const missingResult = executeLauncher(launcher.source, ["--version"]);
  assert.equal(missingResult.status, 1);
  assert.match(
    missingResult.stderr,
    new RegExp(`native package ${currentPlatform.package} is missing`),
  );
});

test("rejects incomplete inputs without leaving partial output", async (context) => {
  const temporary = await temporaryDirectory(context);
  const input = path.join(temporary, "input");
  const output = path.join(temporary, "output");
  const omitted = platforms.at(-1);
  await prepareInputs(input, omitted.target);

  await assert.rejects(
    packageFamily(version, input, output),
    new RegExp(`release binary for ${omitted.target}`),
  );
  assert.equal(existsSync(output), false);
  assert.deepEqual(
    (await readdir(temporary)).filter((entry) =>
      entry.startsWith(".ragavan-npm-"),
    ),
    [],
  );
});

test("refuses to replace an existing output directory", async (context) => {
  const temporary = await temporaryDirectory(context);
  const input = path.join(temporary, "input");
  const output = path.join(temporary, "output");
  const sentinel = path.join(output, "sentinel");
  await prepareInputs(input);
  await mkdir(output);
  await writeFile(sentinel, "preserved\n", "utf8");

  await assert.rejects(packageFamily(version, input, output), /already exists/);
  assert.equal(await readFile(sentinel, "utf8"), "preserved\n");
  assert.equal((await stat(output)).isDirectory(), true);
});

test(
  "refuses to replace a dangling output symlink",
  { skip: process.platform === "win32" },
  async (context) => {
    const temporary = await temporaryDirectory(context);
    const input = path.join(temporary, "input");
    const output = path.join(temporary, "output");
    await prepareInputs(input);
    await symlink("missing-output", output);

    await assert.rejects(
      packageFamily(version, input, output),
      /already exists/,
    );
    assert.equal((await lstat(output)).isSymbolicLink(), true);
  },
);

test(
  "smoke install executes the npm command shim",
  { skip: process.platform === "win32" },
  async (context) => {
    const temporary = await temporaryDirectory(context);
    const input = path.join(temporary, "input");
    const artifacts = path.join(temporary, "artifacts");
    const prefix = path.join(temporary, "prefix");
    const platform = hostPlatform();
    await prepareInputs(input);
    const nativeExecutable = path.join(
      input,
      `binary-${platform.target}`,
      platform.binary,
    );
    await writeFile(
      nativeExecutable,
      `#!/bin/sh\nprintf '%s\\n' 'ragavan ${version}'\n`,
      "utf8",
    );
    await chmod(nativeExecutable, 0o755);
    await packageFamily(version, input, artifacts);

    const result = spawnSync(
      process.execPath,
      [smokeExecutable, version, platform.target, artifacts, prefix],
      { encoding: "utf8", windowsHide: true },
    );

    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stderr, "");
    assert.equal(result.stdout, `ragavan ${version}\n`);
  },
);

test("smoke install refuses an existing prefix", async (context) => {
  const temporary = await temporaryDirectory(context);
  const artifacts = path.join(temporary, "artifacts");
  const prefix = path.join(temporary, "prefix");
  const sentinel = path.join(prefix, "sentinel");
  const platform = hostPlatform();
  await mkdir(artifacts);
  await writeFile(path.join(artifacts, `ragavan-${version}.tgz`), "", "utf8");
  await writeFile(
    path.join(artifacts, `${platform.package}-${version}.tgz`),
    "",
    "utf8",
  );
  await mkdir(prefix);
  await writeFile(sentinel, "preserved\n", "utf8");

  const result = spawnSync(
    process.execPath,
    [smokeExecutable, version, platform.target, artifacts, prefix],
    { encoding: "utf8", windowsHide: true },
  );

  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /npm smoke installation prefix already exists/);
  assert.equal(await readFile(sentinel, "utf8"), "preserved\n");
});
