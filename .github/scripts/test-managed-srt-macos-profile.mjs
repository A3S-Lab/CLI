import assert from "node:assert/strict";
import {
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [installRoot] = process.argv.slice(2);
if (!installRoot) {
  throw new Error(
    "usage: node test-managed-srt-macos-profile.mjs <managed-srt-root>",
  );
}

const runtime = resolve(
  installRoot,
  "node_modules",
  "@anthropic-ai",
  "sandbox-runtime",
  "dist",
  "sandbox",
  "macos-sandbox-utils.js",
);
const scratch = mkdtempSync(join(tmpdir(), "a3s-macos-profile-test-"));
const previousTemp = {
  TMPDIR: process.env.TMPDIR,
  TMP: process.env.TMP,
  TEMP: process.env.TEMP,
};

function restoreEnvironment(name, value) {
  if (value === undefined) {
    delete process.env[name];
  } else {
    process.env[name] = value;
  }
}

try {
  process.env.TMPDIR = scratch;
  process.env.TMP = scratch;
  process.env.TEMP = scratch;

  const { wrapCommandWithSandboxMacOS } = await import(
    pathToFileURL(runtime).href
  );
  const aliases = Array.from(
    { length: 4_097 },
    (_, index) =>
      "/private/tmp/a3s-large-profile/workspace/" +
      "outside-hardlink-profile-alias-with-a-deliberately-long-name-" +
      String(index).padStart(4, "0") +
      ".txt",
  );
  const wrapped = wrapCommandWithSandboxMacOS({
    command: "printf a3s-managed-srt-large-profile-ready",
    needsNetworkRestriction: false,
    readConfig: {
      denyOnly: aliases,
      allowWithinDeny: ["/private/tmp/a3s-large-profile/workspace"],
    },
    writeConfig: {
      allowOnly: ["/private/tmp/a3s-large-profile/workspace"],
      denyWithinAllow: aliases,
    },
    unsetEnvVars: [],
    setEnvVars: {},
    maskedFileBinds: [],
    allowPty: false,
    allowGitConfig: false,
    gitSafeDirectories: [],
    enableWeakerNetworkIsolation: false,
    allowAppleEvents: false,
    binShell: "/bin/bash",
  });

  const profileDirectories = readdirSync(scratch, { withFileTypes: true })
    .filter(
      (entry) => entry.isDirectory() && entry.name.startsWith("a3s-srt-profile-"),
    )
    .map((entry) => join(scratch, entry.name));
  assert.equal(profileDirectories.length, 1);
  const profilePath = join(profileDirectories[0], "sandbox.sb");
  const profile = readFileSync(profilePath, "utf8");
  const firstAlias = aliases[0];
  const lastAlias = aliases.at(-1);

  assert.ok(wrapped.includes("/usr/bin/sandbox-exec"));
  assert.ok(wrapped.includes(" -f "));
  assert.ok(wrapped.includes(profilePath));
  assert.ok(profile.includes(firstAlias));
  assert.ok(profile.includes(lastAlias));
  assert.ok(
    Buffer.byteLength(profile) < 8 * 1024 * 1024,
    "common ancestor rules were not deduplicated",
  );

  const commonAncestor = '  (literal "/private/tmp/a3s-large-profile/workspace")';
  const ancestorRuleCount = profile.split(commonAncestor).length - 1;
  assert.ok(
    ancestorRuleCount <= 4,
    `common ancestor move rule repeated ${ancestorRuleCount} times`,
  );

  process.stdout.write(
    `${JSON.stringify({ aliases: aliases.length, profileBytes: Buffer.byteLength(profile) })}\n`,
  );
} finally {
  restoreEnvironment("TMPDIR", previousTemp.TMPDIR);
  restoreEnvironment("TMP", previousTemp.TMP);
  restoreEnvironment("TEMP", previousTemp.TEMP);
  rmSync(scratch, { recursive: true, force: true });
}
