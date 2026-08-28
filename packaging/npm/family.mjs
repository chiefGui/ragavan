const VERSION_PATTERN = /^[0-9A-Za-z.+-]+$/;

export const ROOT_PACKAGE = "ragavan";
export const PUBLICATION_MANIFEST = "ragavan-npm-publication.json";
export const PUBLICATION_SCHEMA = 1;

export function requireVersion(version) {
  if (typeof version !== "string" || !VERSION_PATTERN.test(version)) {
    throw new Error(`invalid npm package version ${version}`);
  }
  return version;
}

export function tarballName(packageName, version) {
  return `${packageName}-${requireVersion(version)}.tgz`;
}

export function releasePackages(platforms, version) {
  requireVersion(version);
  return [
    ...platforms.map((platform) => ({
      kind: "native",
      name: platform.package,
      target: platform.target,
      tarball: tarballName(platform.package, version),
    })),
    {
      kind: "launcher",
      name: ROOT_PACKAGE,
      tarball: tarballName(ROOT_PACKAGE, version),
    },
  ];
}

export function distributionTag(version) {
  const packageVersion = requireVersion(version);
  const [withoutBuildMetadata] = packageVersion.split("+", 1);
  return withoutBuildMetadata.includes("-") ? "next" : "latest";
}
