import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";

let invocation;

export function parseNpmReport(output, meaning) {
  let value;
  try {
    value = JSON.parse(output);
  } catch (error) {
    throw new Error(`npm returned invalid ${meaning} metadata`, {
      cause: error,
    });
  }

  const reports = Array.isArray(value) ? value : [value];
  if (reports.length !== 1 || !reports[0] || typeof reports[0] !== "object") {
    throw new Error(`npm returned unexpected ${meaning} metadata`);
  }
  return reports[0];
}

function resolveInvocation() {
  if (invocation) {
    return invocation;
  }

  const configured = process.env.npm_execpath;
  if (configured && existsSync(configured)) {
    invocation = { command: process.execPath, arguments: [configured] };
    return invocation;
  }

  const searchPath = process.env.PATH ?? process.env.Path ?? "";
  const directories = searchPath
    .split(path.delimiter)
    .filter((directory) => directory.length > 0);

  if (process.platform === "win32") {
    for (const directory of directories) {
      const command = path.join(directory, "npm.cmd");
      const cli = path.join(
        directory,
        "node_modules",
        "npm",
        "bin",
        "npm-cli.js",
      );
      if (existsSync(command) && existsSync(cli)) {
        invocation = { command: process.execPath, arguments: [cli] };
        return invocation;
      }
    }
  } else {
    for (const directory of directories) {
      const command = path.join(directory, "npm");
      if (existsSync(command)) {
        invocation = { command, arguments: [] };
        return invocation;
      }
    }
  }

  throw new Error("could not locate npm");
}

export function runNpm(arguments_, options = {}) {
  const npm = resolveInvocation();
  const inherit = options.stdio === "inherit";
  const result = spawnSync(npm.command, [...npm.arguments, ...arguments_], {
    cwd: options.cwd,
    env: options.env ?? process.env,
    ...(inherit ? { stdio: "inherit" } : { encoding: "utf8" }),
    windowsHide: true,
  });

  if (result.error) {
    throw new Error(`could not start npm: ${result.error.message}`, {
      cause: result.error,
    });
  }
  if (result.status !== 0) {
    const stderr =
      typeof result.stderr === "string" ? result.stderr.trim() : "";
    const stdout =
      typeof result.stdout === "string" ? result.stdout.trim() : "";
    const detail = stderr || stdout;
    throw new Error(
      `npm ${arguments_[0] ?? "command"} failed with exit code ${result.status}${detail ? `: ${detail}` : ""}`,
    );
  }

  return typeof result.stdout === "string" ? result.stdout : "";
}
