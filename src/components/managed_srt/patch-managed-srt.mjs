import assert from "node:assert/strict";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const relativeLinuxRuntime = join(
  "node_modules",
  "@anthropic-ai",
  "sandbox-runtime",
  "dist",
  "sandbox",
  "linux-sandbox-utils.js",
);

const relativeMacRuntime = join(
  "node_modules",
  "@anthropic-ai",
  "sandbox-runtime",
  "dist",
  "sandbox",
  "macos-sandbox-utils.js",
);

const denyOrderUpstream = `        const denyPaths = [
            ...(writeConfig.denyWithinAllow || []),
            ...(await linuxGetMandatoryDenyPaths(ripgrepConfig, mandatoryDenySearchDepth, allowGitConfig, abortSignal)),
        ];`;

const denyOrderPatched = `        const denyPaths = [
            // Mandatory child paths must be mounted before caller-supplied parent
            // denies. Otherwise a read-only parent prevents bwrap from creating
            // a mount point for a non-existent mandatory child.
            ...(await linuxGetMandatoryDenyPaths(ripgrepConfig, mandatoryDenySearchDepth, allowGitConfig, abortSignal)),
            ...(writeConfig.denyWithinAllow || []),
        ];`;

const seccompReadUpstream = `        const fsArgs = await generateFilesystemArgs(readConfig, writeConfig, maskedFileBinds, maskedFileStoreDir, ripgrepConfig, mandatoryDenySearchDepth, allowGitConfig, abortSignal);`;

const seccompReadPatched = `        // The outer sandbox can hide the user home before its inner seccomp
        // helper starts. Re-expose only the helper selected by this verified
        // runtime so Unix-socket filtering remains active inside that boundary.
        const seccompReadPath = !allowAllUnixSockets
            ? seccompConfig?.argv0
                ? seccompConfig.applyPath
                : getApplySeccompBinaryPath(seccompConfig?.applyPath)
            : undefined;
        const effectiveReadConfig = readConfig && seccompReadPath
            ? {
                ...readConfig,
                allowWithinDeny: [...(readConfig.allowWithinDeny || []), seccompReadPath],
            }
            : readConfig;
        const fsArgs = await generateFilesystemArgs(effectiveReadConfig, writeConfig, maskedFileBinds, maskedFileStoreDir, ripgrepConfig, mandatoryDenySearchDepth, allowGitConfig, abortSignal);`;

const missingAncestorUpstream = `                    const firstNonExistent = findFirstNonExistentComponent(normalizedPath);
                    // Fix 2: If firstNonExistent is an intermediate component (not the
                    // leaf deny path itself), mount a read-only empty directory instead
                    // of /dev/null. This prevents the component from appearing as a file
                    // which breaks tools that expect to traverse it as a directory.`;

const missingAncestorPatched = `                    const firstNonExistent = findFirstNonExistentComponent(normalizedPath);
                    // Multiple child and parent denies can converge on the same first
                    // missing component. The first read-only mount already protects the
                    // entire subtree; emitting another can conflict on file-vs-directory
                    // destination type and make bwrap refuse to start.
                    if (seenDenyWriteMounts.has(firstNonExistent)) {
                        continue;
                    }
                    seenDenyWriteMounts.add(firstNonExistent);
                    // Fix 2: If firstNonExistent is an intermediate component (not the
                    // leaf deny path itself), mount a read-only empty directory instead
                    // of /dev/null. This prevents the component from appearing as a file
                    // which breaks tools that expect to traverse it as a directory.`;

const mountSetUpstream = `        const seenDenyWrite = new Set();
        for (const pathPattern of denyPaths) {`;

const mountSetPatched = `        const seenDenyWrite = new Set();
        const seenDenyWriteMounts = new Set();
        for (const pathPattern of denyPaths) {`;

const linuxReplacements = [
  {
    name: "nested deny mount order",
    upstream: denyOrderUpstream,
    patched: denyOrderPatched,
  },
  {
    name: "seccomp helper read access",
    upstream: seccompReadUpstream,
    patched: seccompReadPatched,
  },
  {
    name: "missing ancestor mount deduplication",
    upstream: missingAncestorUpstream,
    patched: missingAncestorPatched,
  },
  {
    name: "missing ancestor mount tracking",
    upstream: mountSetUpstream,
    patched: mountSetPatched,
  },
];

const macImportsUpstream = `import { spawn } from 'child_process';
import * as path from 'path';`;

const macImportsPatched = `import { spawn } from 'child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'path';`;

const macProfileArgUpstream = `    // Use \`env\` command to set environment variables - each VAR=value is a separate
    // argument that quote() escapes properly, avoiding shell quoting issues
    const wrappedCommand = quote([
        'env',
        ...unsetEnvArgs,
        ...setEnvArgs,
        ...proxyEnvArgs,
        '/usr/bin/sandbox-exec',
        '-p',
        profile,
        shell,
        '-c',
        command,
    ]);`;

const macProfileArgPatched = `    // macOS applies a much smaller per-process argv limit than Linux. Persist
    // the generated Seatbelt profile in a private directory instead of passing
    // it through sandbox-exec -p, whose argv grows with every protected path.
    // A3S Core pins TMPDIR to its per-command scratch directory; the EXIT trap
    // also removes the profile after ordinary completion and startup failure.
    const profileDirectory = mkdtempSync(path.join(tmpdir(), 'a3s-srt-profile-'));
    const profilePath = path.join(profileDirectory, 'sandbox.sb');
    writeFileSync(profilePath, profile, {
        encoding: 'utf8',
        flag: 'wx',
        mode: 0o600,
    });
    const cleanupCommand = quote(['/bin/rm', '-rf', '--', profileDirectory]);
    const cleanupTrap = quote(['trap', cleanupCommand, 'EXIT']);
    const sandboxCommand = quote([
        'env',
        ...unsetEnvArgs,
        ...setEnvArgs,
        ...proxyEnvArgs,
        '/usr/bin/sandbox-exec',
        '-f',
        profilePath,
        shell,
        '-c',
        command,
    ]);
    const wrappedCommand = \`\${cleanupTrap}; \${sandboxCommand}\`;`;

const macReplacements = [
  {
    name: "profile file imports",
    upstream: macImportsUpstream,
    patched: macImportsPatched,
  },
  {
    name: "Seatbelt profile file transport",
    upstream: macProfileArgUpstream,
    patched: macProfileArgPatched,
  },
];

function occurrenceCount(source, needle) {
  return source.split(needle).length - 1;
}

function isDirectInvocation(argvPath, moduleUrl) {
  if (!argvPath) {
    return false;
  }
  try {
    return (
      realpathSync(argvPath) === realpathSync(fileURLToPath(moduleUrl))
    );
  } catch {
    return false;
  }
}

function prepareRuntimePatch(installRoot, relativeRuntime, platform, replacements) {
  const runtime = join(resolve(installRoot), relativeRuntime);
  let source = readFileSync(runtime, "utf8");
  let changed = false;

  for (const replacement of replacements) {
    const upstreamCount = occurrenceCount(source, replacement.upstream);
    const patchedCount = occurrenceCount(source, replacement.patched);
    if (upstreamCount === 0 && patchedCount === 1) {
      continue;
    }
    if (upstreamCount !== 1 || patchedCount !== 0) {
      throw new Error(
        `managed SRT ${platform} compatibility patch expected one ${replacement.name} ` +
          `upstream block in ${runtime}; found upstream=${upstreamCount}, ` +
          `patched=${patchedCount}`,
      );
    }
    source = source.replace(replacement.upstream, replacement.patched);
    changed = true;
  }

  return { runtime, source, changed };
}

export function patchManagedSrt(installRoot) {
  const plans = [
    prepareRuntimePatch(
      installRoot,
      relativeLinuxRuntime,
      "Linux",
      linuxReplacements,
    ),
    prepareRuntimePatch(
      installRoot,
      relativeMacRuntime,
      "macOS",
      macReplacements,
    ),
  ];

  for (const plan of plans) {
    if (plan.changed) {
      writeFileSync(plan.runtime, plan.source, "utf8");
    }
  }
  return plans.some((plan) => plan.changed) ? "patched" : "already-patched";
}

function selfTest() {
  const root = mkdtempSync(join(tmpdir(), "a3s-managed-srt-patch-"));
  const fixtures = [
    [relativeLinuxRuntime, linuxReplacements],
    [relativeMacRuntime, macReplacements],
  ];
  try {
    for (const [relativeRuntime, replacements] of fixtures) {
      const runtime = join(root, relativeRuntime);
      mkdirSync(dirname(runtime), { recursive: true });
      const fixture = replacements
        .map((replacement) => replacement.upstream)
        .join("\n");
      writeFileSync(runtime, `prefix\n${fixture}\nsuffix\n`, "utf8");
    }

    assert.equal(patchManagedSrt(root), "patched");
    for (const [relativeRuntime, replacements] of fixtures) {
      const patched = readFileSync(join(root, relativeRuntime), "utf8");
      for (const replacement of replacements) {
        assert.equal(occurrenceCount(patched, replacement.upstream), 0);
        assert.equal(occurrenceCount(patched, replacement.patched), 1);
      }
    }
    assert.equal(patchManagedSrt(root), "already-patched");

    writeFileSync(
      join(root, relativeMacRuntime),
      "unexpected upstream source\n",
      "utf8",
    );
    assert.throws(
      () => patchManagedSrt(root),
      /macOS compatibility patch expected one .* upstream block/,
    );

    const invocationTarget = join(root, "invocation-target.mjs");
    const invocationLink = join(root, "invocation-link.mjs");
    writeFileSync(invocationTarget, "export {};\n", "utf8");
    symlinkSync(invocationTarget, invocationLink);
    assert.equal(
      isDirectInvocation(invocationLink, pathToFileURL(invocationTarget).href),
      true,
    );
    assert.equal(
      isDirectInvocation(join(root, "missing.mjs"), import.meta.url),
      false,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const invokedDirectly = isDirectInvocation(process.argv[1], import.meta.url);

if (invokedDirectly) {
  if (process.argv[2] === "--self-test" && process.argv.length === 3) {
    selfTest();
  } else if (process.argv[2] && process.argv.length === 3) {
    process.stdout.write(`${patchManagedSrt(process.argv[2])}\n`);
  } else {
    throw new Error(
      "usage: node patch-managed-srt.mjs <install-root> | --self-test",
    );
  }
}
