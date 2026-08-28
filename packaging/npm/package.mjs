import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { isMain } from "../entrypoint.mjs";
import { npmPlatforms, readPlatforms } from "../platforms.mjs";
import {
  PUBLICATION_MANIFEST,
  PUBLICATION_SCHEMA,
  ROOT_PACKAGE,
  requireVersion,
  tarballName,
} from "./family.mjs";
import { requireAbsent, requireDirectory, requireFile } from "./filesystem.mjs";
import { parseNpmReport, runNpm } from "./npm.mjs";

const sourceDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(sourceDirectory, "..", "..");
const launcher = path.join(sourceDirectory, "launcher.cjs");
const license = path.join(repositoryRoot, "LICENSE");
const readme = path.join(repositoryRoot, "README.md");

const repository = {
  type: "git",
  url: "git+https://github.com/chiefGui/ragavan.git",
};
const homepage = "https://github.com/chiefGui/ragavan#readme";
const bugs = { url: "https://github.com/chiefGui/ragavan/issues" };

async function writeJson(filePath, value) {
  await writeFile(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function commonMetadata(name, version, description) {
  return {
    name,
    version,
    description,
    license: "MIT",
    repository,
    homepage,
    bugs,
  };
}

function sameValues(left, right) {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

async function pack(directory, outputDirectory, expectedFiles, name, version) {
  const output = runNpm([
    "pack",
    "--json",
    "--ignore-scripts",
    "--loglevel=error",
    "--pack-destination",
    outputDirectory,
    directory,
  ]);

  const report = parseNpmReport(output, `pack for ${name}`);
  if (report.name !== name || report.version !== version) {
    throw new Error(`npm packed the wrong identity for ${name}`);
  }
  if (
    !Array.isArray(report.files) ||
    report.files.some(
      (file) =>
        !file || typeof file !== "object" || typeof file.path !== "string",
    )
  ) {
    throw new Error(`npm returned invalid file metadata for ${name}`);
  }

  const actualFiles = report.files
    .map((file) => file.path)
    .sort((left, right) => left.localeCompare(right));
  const requiredFiles = [...expectedFiles].sort((left, right) =>
    left.localeCompare(right),
  );
  if (!sameValues(actualFiles, requiredFiles)) {
    throw new Error(
      `npm packed unexpected files for ${name}: ${actualFiles.join(", ")}`,
    );
  }

  const expectedTarball = tarballName(name, version);
  if (report.filename !== expectedTarball) {
    throw new Error(
      `npm returned tarball name ${report.filename ?? "unknown"} for ${name}; expected ${expectedTarball}`,
    );
  }

  const tarball = path.join(outputDirectory, report.filename);
  await requireFile(tarball, `npm tarball for ${name}`);
  if (
    typeof report.integrity !== "string" ||
    !report.integrity.startsWith("sha512-")
  ) {
    throw new Error(`npm did not report a SHA-512 integrity for ${name}`);
  }
  return { path: tarball, integrity: report.integrity };
}

async function stagePlatform(
  platform,
  version,
  inputDirectory,
  stageDirectory,
  outputDirectory,
) {
  const directory = path.join(stageDirectory, platform.package);
  const binaryDirectory = path.join(directory, "bin");
  const source = path.join(
    inputDirectory,
    `binary-${platform.target}`,
    platform.binary,
  );
  const binary = path.join(binaryDirectory, platform.binary);

  await mkdir(binaryDirectory, { recursive: true });
  await copyFile(source, binary);
  if (platform.os !== "win32") {
    await chmod(binary, 0o755);
  }
  await copyFile(license, path.join(directory, "LICENSE"));
  await writeJson(path.join(directory, "package.json"), {
    ...commonMetadata(
      platform.package,
      version,
      `The ${platform.target} native binary for Ragavan.`,
    ),
    os: [platform.os],
    cpu: [platform.cpu],
    files: [`bin/${platform.binary}`, "LICENSE"],
  });

  const packed = await pack(
    directory,
    outputDirectory,
    ["LICENSE", `bin/${platform.binary}`, "package.json"],
    platform.package,
    version,
  );
  return {
    kind: "native",
    name: platform.package,
    target: platform.target,
    tarball: path.basename(packed.path),
    integrity: packed.integrity,
  };
}

async function stageLauncher(
  platforms,
  version,
  stageDirectory,
  outputDirectory,
) {
  const directory = path.join(stageDirectory, "ragavan");
  const binaryDirectory = path.join(directory, "bin");
  await mkdir(binaryDirectory, { recursive: true });

  const stagedLauncher = path.join(binaryDirectory, "ragavan.cjs");
  await copyFile(launcher, stagedLauncher);
  await chmod(stagedLauncher, 0o755);
  await writeJson(
    path.join(directory, "platforms.json"),
    platforms.map(({ os, cpu, package: packageName, binary }) => ({
      os,
      cpu,
      package: packageName,
      binary,
    })),
  );
  await copyFile(license, path.join(directory, "LICENSE"));
  await copyFile(readme, path.join(directory, "README.md"));

  await writeJson(path.join(directory, "package.json"), {
    ...commonMetadata(
      ROOT_PACKAGE,
      version,
      "Zero-config development isolation for concurrent Git worktrees.",
    ),
    keywords: ["git", "worktree", "development", "isolation", "cli"],
    bin: { ragavan: "bin/ragavan.cjs" },
    files: ["bin/ragavan.cjs", "platforms.json", "LICENSE", "README.md"],
    optionalDependencies: Object.fromEntries(
      platforms.map((platform) => [platform.package, version]),
    ),
  });

  const packed = await pack(
    directory,
    outputDirectory,
    [
      "LICENSE",
      "README.md",
      "bin/ragavan.cjs",
      "package.json",
      "platforms.json",
    ],
    ROOT_PACKAGE,
    version,
  );
  return {
    kind: "launcher",
    name: ROOT_PACKAGE,
    tarball: path.basename(packed.path),
    integrity: packed.integrity,
  };
}

export async function packageFamily(version, input, output) {
  requireVersion(version);
  if (typeof input !== "string" || input.length === 0) {
    throw new Error("npm package input directory cannot be empty");
  }
  if (typeof output !== "string" || output.length === 0) {
    throw new Error("npm package output directory cannot be empty");
  }

  const inputDirectory = path.resolve(input);
  const outputDirectory = path.resolve(output);
  const platforms = npmPlatforms(await readPlatforms());

  await requireDirectory(inputDirectory, "npm package input directory");
  await requireFile(launcher, "npm launcher");
  await requireFile(license, "repository license");
  await requireFile(readme, "repository readme");
  await requireAbsent(outputDirectory, "npm package output");

  for (const platform of platforms) {
    await requireFile(
      path.join(inputDirectory, `binary-${platform.target}`, platform.binary),
      `release binary for ${platform.target}`,
    );
  }

  const outputParent = path.dirname(outputDirectory);
  await mkdir(outputParent, { recursive: true });
  await requireAbsent(outputDirectory, "npm package output");

  const temporaryDirectory = await mkdtemp(
    path.join(outputParent, ".ragavan-npm-"),
  );
  const stageDirectory = path.join(temporaryDirectory, "stage");
  const artifactDirectory = path.join(temporaryDirectory, "artifacts");

  try {
    await mkdir(stageDirectory);
    await mkdir(artifactDirectory);

    const packages = [];
    for (const platform of platforms) {
      packages.push(
        await stagePlatform(
          platform,
          version,
          inputDirectory,
          stageDirectory,
          artifactDirectory,
        ),
      );
    }
    packages.push(
      await stageLauncher(
        platforms,
        version,
        stageDirectory,
        artifactDirectory,
      ),
    );
    await writeJson(path.join(artifactDirectory, PUBLICATION_MANIFEST), {
      schema: PUBLICATION_SCHEMA,
      version,
      packages,
    });

    await rename(artifactDirectory, outputDirectory);
    return packages.map((releasePackage) =>
      path.join(outputDirectory, releasePackage.tarball),
    );
  } finally {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
}

if (isMain(import.meta.url)) {
  const arguments_ = process.argv.slice(2);
  if (arguments_.length !== 3) {
    process.stderr.write(
      "usage: package.mjs <version> <input-directory> <output-directory>\n",
    );
    process.exitCode = 2;
  } else {
    try {
      const tarballs = await packageFamily(...arguments_);
      for (const tarball of tarballs) {
        process.stdout.write(`${tarball}\n`);
      }
    } catch (error) {
      process.stderr.write(
        `could not package Ragavan for npm: ${error.message}\n`,
      );
      process.exitCode = 1;
    }
  }
}
