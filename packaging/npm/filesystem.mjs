import { lstat, stat } from "node:fs/promises";

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

export async function requireFile(filePath, meaning) {
  const metadata = await pathType(filePath);
  if (!metadata || !metadata.isFile()) {
    throw new Error(`${meaning} was not found at ${filePath}`);
  }
}

export async function requireDirectory(directory, meaning) {
  const metadata = await pathType(directory);
  if (!metadata || !metadata.isDirectory()) {
    throw new Error(`${meaning} was not found at ${directory}`);
  }
}

export async function requireAbsent(filePath, meaning) {
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
