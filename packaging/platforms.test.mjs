import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFile, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import { isMain } from "./entrypoint.mjs";
import { readPlatforms } from "./platforms.mjs";

const executable = fileURLToPath(new URL("./platforms.mjs", import.meta.url));
const entrypoint = fileURLToPath(new URL("./entrypoint.mjs", import.meta.url));

test(
  "recognizes an entry point through a filesystem alias",
  { skip: process.platform === "win32" },
  async (context) => {
    const directory = await mkdtemp(
      path.join(os.tmpdir(), "ragavan-entrypoint-"),
    );
    context.after(() => rm(directory, { recursive: true, force: true }));
    const target = path.join(directory, "target.mjs");
    const alias = path.join(directory, "alias.mjs");
    await writeFile(target, "", "utf8");
    await symlink(target, alias);

    assert.equal(isMain(pathToFileURL(target), alias), true);
  },
);

test("exports the release matrix from the platform catalog", async () => {
  const platforms = await readPlatforms();
  const result = spawnSync(process.execPath, [executable], {
    encoding: "utf8",
    windowsHide: true,
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr, "");
  assert.deepEqual(JSON.parse(result.stdout), {
    include: platforms.map(({ target, runner, binary }) => ({
      target,
      runner,
      binary,
    })),
  });
});

test("rejects ambiguous npm runtime mappings", async (context) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ragavan-platforms-"));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const copiedExecutable = path.join(directory, "platforms.mjs");
  await copyFile(executable, copiedExecutable);
  await copyFile(entrypoint, path.join(directory, "entrypoint.mjs"));
  await writeFile(
    path.join(directory, "platforms.json"),
    JSON.stringify([
      {
        target: "first-target",
        runner: "first-runner",
        binary: "ragavan",
        npm: {
          os: "linux",
          cpu: "x64",
          package: "@ragavan-cli/first",
        },
      },
      {
        target: "second-target",
        runner: "second-runner",
        binary: "ragavan",
        npm: {
          os: "linux",
          cpu: "x64",
          package: "@ragavan-cli/second",
        },
      },
    ]),
    "utf8",
  );

  const result = spawnSync(process.execPath, [copiedExecutable], {
    encoding: "utf8",
    windowsHide: true,
  });

  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /npm runtime linux-x64 is duplicated/);
});
