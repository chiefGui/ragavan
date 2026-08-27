#!/usr/bin/env node
"use strict";

const { spawnSync } = require("node:child_process");
const platforms = require("../platforms.json");

function fail(message) {
  process.stderr.write(`ragavan: ${message}\n`);
  process.exit(1);
}

const selected = platforms.find(
  (platform) =>
    platform.os === process.platform && platform.cpu === process.arch,
);

if (!selected) {
  fail(`unsupported platform ${process.platform}-${process.arch}`);
}

let executable;
try {
  executable = require.resolve(`${selected.package}/bin/${selected.binary}`);
} catch (error) {
  if (error && error.code === "MODULE_NOT_FOUND") {
    fail(
      `native package ${selected.package} is missing; reinstall ragavan without omitting optional dependencies`,
    );
  }
  throw error;
}

const child = spawnSync(executable, process.argv.slice(2), {
  stdio: "inherit",
});

if (child.error) {
  fail(`could not start the native executable: ${child.error.message}`);
}
if (child.signal) {
  try {
    process.kill(process.pid, child.signal);
  } catch {
    process.exit(1);
  }
}

process.exit(child.status ?? 1);
