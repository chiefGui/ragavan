import assert from "node:assert/strict";
import test from "node:test";

import { requireReleaseVersion } from "./version.mjs";

test("accepts stable and prerelease SemVer versions", () => {
  for (const version of ["0.3.0", "1.0.0-alpha.1", "1.2.3-rc.1+build.42"]) {
    assert.equal(requireReleaseVersion(version), version);
  }
});

test("rejects malformed SemVer versions", () => {
  for (const version of ["1.2", "01.2.3", "1.2.3-01", "1.2.3+", "latest"]) {
    assert.throws(
      () => requireReleaseVersion(version),
      /does not contain a valid SemVer version/,
    );
  }
});
