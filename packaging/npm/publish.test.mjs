import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import { npmPlatforms, readPlatforms } from "../platforms.mjs";
import { distributionTag, releasePackages } from "./family.mjs";
import { publishPublication } from "./publish.mjs";

const platforms = npmPlatforms(await readPlatforms());

function publication(version = "1.2.3") {
  return {
    version,
    tag: distributionTag(version),
    packages: releasePackages(platforms, version).map(
      (releasePackage, index) => ({
        ...releasePackage,
        version,
        path: path.join("artifacts", releasePackage.tarball),
        integrity: `sha512-test-${index}`,
      }),
    ),
  };
}

function recordingRegistry(integrities = new Map()) {
  const published = [];
  return {
    published,
    async publishedIntegrity(releasePackage) {
      return integrities.get(releasePackage.name);
    },
    async publish(releasePackage, tag) {
      published.push({ name: releasePackage.name, tag });
      integrities.set(releasePackage.name, releasePackage.integrity);
    },
  };
}

test("publishes missing native packages before the launcher", async () => {
  const candidate = publication();
  const registry = recordingRegistry();

  const results = await publishPublication(candidate, registry);

  assert.deepEqual(
    registry.published,
    candidate.packages.map((releasePackage) => ({
      name: releasePackage.name,
      tag: "latest",
    })),
  );
  assert.deepEqual(
    results,
    candidate.packages.map((releasePackage) => ({
      name: releasePackage.name,
      status: "published",
    })),
  );
  assert.equal(candidate.packages.at(-1).kind, "launcher");
});

test("resumes an exact partial publication", async () => {
  const candidate = publication();
  const existing = new Map(
    candidate.packages
      .slice(0, 2)
      .map((releasePackage) => [releasePackage.name, releasePackage.integrity]),
  );
  const registry = recordingRegistry(existing);

  const results = await publishPublication(candidate, registry);

  assert.deepEqual(
    registry.published.map(({ name }) => name),
    candidate.packages.slice(2).map(({ name }) => name),
  );
  assert.deepEqual(
    results.map(({ status }) => status),
    ["unchanged", "unchanged", "published", "published", "published"],
  );
});

test("rejects conflicting registry contents before publishing", async () => {
  const candidate = publication();
  const conflict = candidate.packages[1];
  const registry = recordingRegistry(
    new Map([[conflict.name, "sha512-different"]]),
  );

  await assert.rejects(
    publishPublication(candidate, registry),
    new RegExp(
      `npm already contains different contents for ${conflict.name}@${candidate.version}`,
    ),
  );
  assert.deepEqual(registry.published, []);
});
