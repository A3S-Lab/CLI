import { createHash } from "node:crypto";
import {
  lstatSync,
  readFileSync,
  readdirSync,
} from "node:fs";
import { join } from "node:path";

const [root, digestFile] = process.argv.slice(2);
if (!root || !digestFile) {
  throw new Error(
    "usage: node verify-managed-srt-tree.mjs <tree> <expected-digest-file>",
  );
}

const expected = readFileSync(digestFile, "utf8").trim().toLowerCase();
if (!/^[0-9a-f]{64}$/.test(expected)) {
  throw new Error(`invalid expected SHA-256 digest: ${expected}`);
}

const hash = createHash("sha256");
const maximumEntries = 2_000;
const maximumBytes = 32 * 1024 * 1024;
let entryCount = 0;
let byteCount = 0;

function hashField(value) {
  const bytes = Buffer.from(value);
  const size = Buffer.alloc(8);
  size.writeBigUInt64LE(BigInt(bytes.length));
  hash.update(size);
  hash.update(bytes);
}

function compareNames(left, right) {
  return Buffer.compare(Buffer.from(left.name), Buffer.from(right.name));
}

function hashDirectory(directory, relativeDirectory) {
  const entries = readdirSync(directory, { withFileTypes: true }).sort(
    compareNames,
  );
  for (const entry of entries) {
    entryCount += 1;
    if (entryCount > maximumEntries) {
      throw new Error(
        `managed SRT release payload exceeds ${maximumEntries} entries`,
      );
    }
    const relative = relativeDirectory
      ? `${relativeDirectory}/${entry.name}`
      : entry.name;
    const path = join(directory, entry.name);
    const metadata = lstatSync(path);

    if (metadata.isSymbolicLink()) {
      throw new Error(`managed SRT release payload contains a link: ${path}`);
    } else if (metadata.isDirectory()) {
      hash.update(Buffer.from("dir\0"));
      hashField(relative);
      hashDirectory(path, relative);
    } else if (metadata.isFile()) {
      byteCount += metadata.size;
      if (byteCount > maximumBytes) {
        throw new Error(
          `managed SRT release payload exceeds ${maximumBytes} bytes`,
        );
      }
      hash.update(Buffer.from("file\0"));
      hashField(relative);
      hash.update(readFileSync(path));
      hash.update(Buffer.from("\0"));
    } else {
      throw new Error(`unsupported file type in managed SRT tree: ${path}`);
    }
  }
}

hashDirectory(root, "");
const actual = hash.digest("hex");
if (actual !== expected) {
  throw new Error(
    `managed SRT tree digest mismatch: expected ${expected}, found ${actual}`,
  );
}

process.stdout.write(`${actual}\n`);
