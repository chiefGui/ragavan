import assert from "node:assert/strict";
import test from "node:test";

import { resolveReleaseIdentity } from "./identity.mjs";

function metadata(packages) {
  return { packages };
}

test("resolves the single Ragavan package as the release identity", () => {
  assert.deepEqual(
    resolveReleaseIdentity(
      metadata([
        { name: "ragavan-core", version: "0.2.1" },
        { name: "ragavan", version: "0.2.1" },
      ]),
      "0.2.1",
      "v0.2.1",
    ),
    { version: "0.2.1", tag: "v0.2.1" },
  );
});

test("rejects a tag that disagrees with Cargo", () => {
  assert.throws(
    () =>
      resolveReleaseIdentity(
        metadata([{ name: "ragavan", version: "0.2.1" }]),
        "0.2.1",
        "v0.3.0",
      ),
    /release tag v0\.3\.0 does not match Cargo version v0\.2\.1/,
  );
});

test("rejects release state that disagrees with Cargo", () => {
  assert.throws(
    () =>
      resolveReleaseIdentity(
        metadata([{ name: "ragavan", version: "0.2.1" }]),
        "0.3.0",
        "v0.3.0",
      ),
    /VERSION version 0\.3\.0 does not match Cargo version 0\.2\.1/,
  );
});

test("requires one unambiguous Ragavan package", () => {
  assert.throws(
    () => resolveReleaseIdentity(metadata([]), "0.2.1", "v0.2.1"),
    /expected exactly one Cargo package named ragavan/,
  );
  assert.throws(
    () =>
      resolveReleaseIdentity(
        metadata([
          { name: "ragavan", version: "0.2.1" },
          { name: "ragavan", version: "0.2.1" },
        ]),
        "0.2.1",
        "v0.2.1",
      ),
    /expected exactly one Cargo package named ragavan/,
  );
});

test("rejects malformed Cargo metadata", () => {
  assert.throws(
    () => resolveReleaseIdentity({}, "0.2.1", "v0.2.1"),
    /Cargo metadata does not contain a package list/,
  );
  assert.throws(
    () =>
      resolveReleaseIdentity(
        metadata([{ name: "ragavan", version: "not a version" }]),
        "0.2.1",
        "vnot a version",
      ),
    /ragavan Cargo version does not contain a valid SemVer version/,
  );
});

test("rejects malformed release state", () => {
  assert.throws(
    () =>
      resolveReleaseIdentity(
        metadata([{ name: "ragavan", version: "0.2.1" }]),
        "",
        "v0.2.1",
      ),
    /VERSION does not contain a valid SemVer version/,
  );
});
