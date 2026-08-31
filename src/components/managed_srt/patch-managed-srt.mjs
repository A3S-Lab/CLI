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

const macRuleHelpersUpstream = `function generateLogTag(command) {
    const encodedCommand = encodeSandboxedCommand(command);
    return \`CMD64_\${encodedCommand}_END_\${sessionSuffix}\`;
}
/**
 * Get all ancestor directories for a path, up to (but not including) root
 * Example: /private/tmp/test/file.txt -> ["/private/tmp/test", "/private/tmp", "/private"]
 */`;

const macRuleHelpersPatched = `function generateLogTag(command) {
    const encodedCommand = encodeSandboxedCommand(command);
    return \`CMD64_\${encodedCommand}_END_\${sessionSuffix}\`;
}
/**
 * Render the Seatbelt matcher used for one normalized path.
 */
function pathFilter(normalizedPath) {
    return containsGlobChars(normalizedPath)
        ? \`(regex \${escapePath(globToRegex(normalizedPath))})\`
        : \`(subpath \${escapePath(normalizedPath)})\`;
}
/**
 * Render one Seatbelt rule whose matchers are alternatives. Building the
 * result iteratively also avoids V8 argument-count limits for very large
 * protected-path sets.
 */
function renderRule(action, operations, filters, logTag) {
    const rules = [];
    if (filters.size === 0) {
        return rules;
    }
    rules.push(\`(\${action} \${operations.join(' ')}\`);
    for (const filter of filters) {
        rules.push(\`  \${filter}\`);
    }
    rules.push(\`  (with message "\${logTag}"))\`);
    return rules;
}
/**
 * Get all ancestor directories for a path, up to (but not including) root
 * Example: /private/tmp/test/file.txt -> ["/private/tmp/test", "/private/tmp", "/private"]
 */`;

const macMoveRuleCollectorUpstream = `    const rules = [];
    const ops = ['file-write-unlink', 'file-write-create'];
    for (const pathPattern of pathPatterns) {`;

const macMoveRuleCollectorPatched = `    // Seatbelt treats matchers in one rule as alternatives. Consolidating
    // them avoids compiling one complete rule per protected path and op.
    const filters = new Set();
    for (const pathPattern of pathPatterns) {`;

const macRegexMoveRulesUpstream = `            // Use regex matching for glob patterns
            const regexPattern = globToRegex(normalizedPath);
            // Block moving/renaming files matching this pattern
            for (const op of ops) {
                rules.push(\`(deny \${op}\`, \`  (regex \${escapePath(regexPattern)})\`, \`  (with message "\${logTag}"))\`);
            }`;

const macRegexMoveRulesPatched = `            // Block moving/renaming files matching this pattern.
            filters.add(pathFilter(normalizedPath));`;

const macGlobAncestorMoveRulesUpstream = `                // Block moves of the base directory itself
                for (const op of ops) {
                    rules.push(\`(deny \${op}\`, \`  (literal \${escapePath(baseDir)})\`, \`  (with message "\${logTag}"))\`);
                }
                // Block moves of ancestor directories
                for (const ancestorDir of getAncestorDirectories(baseDir)) {
                    for (const op of ops) {
                        rules.push(\`(deny \${op}\`, \`  (literal \${escapePath(ancestorDir)})\`, \`  (with message "\${logTag}"))\`);
                    }
                }`;

const macGlobAncestorMoveRulesPatched = `                // Block moves of the base directory itself
                filters.add(\`(literal \${escapePath(baseDir)})\`);
                // Block moves of ancestor directories
                for (const ancestorDir of getAncestorDirectories(baseDir)) {
                    filters.add(\`(literal \${escapePath(ancestorDir)})\`);
                }`;

const macLiteralMoveRulesUpstream = `            // Use subpath matching for literal paths
            // Block moving/renaming the denied path itself
            for (const op of ops) {
                rules.push(\`(deny \${op}\`, \`  (subpath \${escapePath(normalizedPath)})\`, \`  (with message "\${logTag}"))\`);
            }
            // Block moves of ancestor directories
            for (const ancestorDir of getAncestorDirectories(normalizedPath)) {
                for (const op of ops) {
                    rules.push(\`(deny \${op}\`, \`  (literal \${escapePath(ancestorDir)})\`, \`  (with message "\${logTag}"))\`);
                }
            }`;

const macLiteralMoveRulesPatched = `            // Use subpath matching for literal paths
            // Block moving/renaming the denied path itself
            filters.add(pathFilter(normalizedPath));
            // Block moves of ancestor directories
            for (const ancestorDir of getAncestorDirectories(normalizedPath)) {
                filters.add(\`(literal \${escapePath(ancestorDir)})\`);
            }`;

const macMoveRuleReturnUpstream = `    return rules;
}
/**
 * Generate filesystem read rules for sandbox profile`;

const macMoveRuleReturnPatched = `    return renderRule(
        'deny',
        ['file-write-unlink', 'file-write-create'],
        filters,
        logTag,
    );
}
/**
 * Generate filesystem read rules for sandbox profile`;

const macReadDenyRulesUpstream = `    // Then deny specific paths
    for (const pathPattern of config.denyOnly || []) {
        const normalizedPath = normalizePathForSandbox(pathPattern);
        if (normalizedPath === '/')
            deniesRoot = true;
        if (containsGlobChars(normalizedPath)) {
            // Use regex matching for glob patterns
            const regexPattern = globToRegex(normalizedPath);
            rules.push(\`(deny file-read*\`, \`  (regex \${escapePath(regexPattern)})\`, \`  (with message "\${logTag}"))\`);
        }
        else {
            // Use subpath matching for literal paths
            rules.push(\`(deny file-read*\`, \`  (subpath \${escapePath(normalizedPath)})\`, \`  (with message "\${logTag}"))\`);
        }
    }`;

const macReadDenyRulesPatched = `    // Then deny specific paths in one rule. Each matcher is an alternative.
    const denyFilters = new Set();
    for (const pathPattern of config.denyOnly || []) {
        const normalizedPath = normalizePathForSandbox(pathPattern);
        if (normalizedPath === '/')
            deniesRoot = true;
        denyFilters.add(pathFilter(normalizedPath));
    }
    for (const rule of renderRule('deny', ['file-read*'], denyFilters, logTag)) {
        rules.push(rule);
    }`;

const macReadAllowRulesUpstream = `    // Re-allow specific paths within denied regions (allowWithinDeny takes precedence)
    const allowedSubpaths = [];
    for (const pathPattern of config.allowWithinDeny || []) {
        const normalizedPath = normalizePathForSandbox(pathPattern);
        if (containsGlobChars(normalizedPath)) {
            const regexPattern = globToRegex(normalizedPath);
            rules.push(\`(allow file-read*\`, \`  (regex \${escapePath(regexPattern)})\`, \`  (with message "\${logTag}"))\`);
        }
        else {
            allowedSubpaths.push(normalizedPath);
            rules.push(\`(allow file-read*\`, \`  (subpath \${escapePath(normalizedPath)})\`, \`  (with message "\${logTag}"))\`);
        }
    }`;

const macReadAllowRulesPatched = `    // Re-allow specific paths within denied regions (allowWithinDeny takes precedence)
    const allowedSubpaths = [];
    const allowFilters = new Set();
    for (const pathPattern of config.allowWithinDeny || []) {
        const normalizedPath = normalizePathForSandbox(pathPattern);
        if (!containsGlobChars(normalizedPath)) {
            allowedSubpaths.push(normalizedPath);
        }
        allowFilters.add(pathFilter(normalizedPath));
    }
    for (const rule of renderRule('allow', ['file-read*'], allowFilters, logTag)) {
        rules.push(rule);
    }`;

const macNestedReadDenyRulesUpstream = `    for (const denyPath of config.denyOnly || []) {
        if (containsGlobChars(denyPath))
            continue;
        const normalized = normalizePathForSandbox(denyPath);
        if (allowedSubpaths.some(a => normalized.startsWith(a + '/'))) {
            rules.push(\`(deny file-read*\`, \`  (subpath \${escapePath(normalized)})\`, \`  (with message "\${logTag}"))\`);
        }
    }`;

const macNestedReadDenyRulesPatched = `    const nestedDenyFilters = new Set();
    for (const denyPath of config.denyOnly || []) {
        if (containsGlobChars(denyPath))
            continue;
        const normalized = normalizePathForSandbox(denyPath);
        if (allowedSubpaths.some(a => normalized.startsWith(a + '/'))) {
            nestedDenyFilters.add(pathFilter(normalized));
        }
    }
    for (const rule of renderRule('deny', ['file-read*'], nestedDenyFilters, logTag)) {
        rules.push(rule);
    }`;

const macWriteAllowMoveRulesUpstream = `    if (writeAllowPaths && writeAllowPaths.length > 0) {
        for (const pathPattern of writeAllowPaths) {
            const normalizedPath = normalizePathForSandbox(pathPattern);
            for (const op of ['file-write-unlink', 'file-write-create']) {
                if (containsGlobChars(normalizedPath)) {
                    const regexPattern = globToRegex(normalizedPath);
                    rules.push(\`(allow \${op}\`, \`  (regex \${escapePath(regexPattern)})\`, \`  (with message "\${logTag}"))\`);
                }
                else {
                    rules.push(\`(allow \${op}\`, \`  (subpath \${escapePath(normalizedPath)})\`, \`  (with message "\${logTag}"))\`);
                }
            }
        }
    }`;

const macWriteAllowMoveRulesPatched = `    const writeAllowFilters = new Set();
    for (const pathPattern of writeAllowPaths || []) {
        writeAllowFilters.add(pathFilter(normalizePathForSandbox(pathPattern)));
    }
    for (const rule of renderRule(
        'allow',
        ['file-write-unlink', 'file-write-create'],
        writeAllowFilters,
        logTag,
    )) {
        rules.push(rule);
    }`;

const macWriteAllowRulesUpstream = `    // Generate allow rules
    for (const pathPattern of config.allowOnly || []) {
        const normalizedPath = normalizePathForSandbox(pathPattern);
        if (containsGlobChars(normalizedPath)) {
            // Use regex matching for glob patterns
            const regexPattern = globToRegex(normalizedPath);
            rules.push(\`(allow file-write*\`, \`  (regex \${escapePath(regexPattern)})\`, \`  (with message "\${logTag}"))\`);
        }
        else {
            // Use subpath matching for literal paths
            rules.push(\`(allow file-write*\`, \`  (subpath \${escapePath(normalizedPath)})\`, \`  (with message "\${logTag}"))\`);
        }
    }`;

const macWriteAllowRulesPatched = `    // Generate one allow rule whose matchers are alternatives.
    const allowFilters = new Set();
    for (const pathPattern of config.allowOnly || []) {
        allowFilters.add(pathFilter(normalizePathForSandbox(pathPattern)));
    }
    for (const rule of renderRule('allow', ['file-write*'], allowFilters, logTag)) {
        rules.push(rule);
    }`;

const macWriteDenyRulesUpstream = `    for (const pathPattern of denyPaths) {
        const normalizedPath = normalizePathForSandbox(pathPattern);
        if (containsGlobChars(normalizedPath)) {
            // Use regex matching for glob patterns
            const regexPattern = globToRegex(normalizedPath);
            rules.push(\`(deny file-write*\`, \`  (regex \${escapePath(regexPattern)})\`, \`  (with message "\${logTag}"))\`);
        }
        else {
            // Use subpath matching for literal paths
            rules.push(\`(deny file-write*\`, \`  (subpath \${escapePath(normalizedPath)})\`, \`  (with message "\${logTag}"))\`);
        }
    }`;

const macWriteDenyRulesPatched = `    const denyFilters = new Set();
    for (const pathPattern of denyPaths) {
        denyFilters.add(pathFilter(normalizePathForSandbox(pathPattern)));
    }
    for (const rule of renderRule('deny', ['file-write*'], denyFilters, logTag)) {
        rules.push(rule);
    }`;

const macReadMoveAppendUpstream = `    rules.push(...generateMoveBlockingRules(config.denyOnly || [], logTag));`;

const macReadMoveAppendPatched = `    for (const rule of generateMoveBlockingRules(config.denyOnly || [], logTag)) {
        rules.push(rule);
    }`;

const macWriteMoveAppendUpstream = `    rules.push(...generateMoveBlockingRules(denyPaths, logTag));`;

const macWriteMoveAppendPatched = `    for (const rule of generateMoveBlockingRules(denyPaths, logTag)) {
        rules.push(rule);
    }`;

const macProfileRuleAppendUpstream = `    profile.push('; File read');
    profile.push(...generateReadRules(readConfig, logTag, writeAllowPaths));
    profile.push('');
    // Write rules
    profile.push('; File write');
    profile.push(...generateWriteRules(writeConfig, logTag, allowGitConfig));`;

const macProfileRuleAppendPatched = `    profile.push('; File read');
    for (const rule of generateReadRules(readConfig, logTag, writeAllowPaths)) {
        profile.push(rule);
    }
    profile.push('');
    // Write rules
    profile.push('; File write');
    for (const rule of generateWriteRules(writeConfig, logTag, allowGitConfig)) {
        profile.push(rule);
    }`;

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
    name: "consolidated rule helpers",
    upstream: macRuleHelpersUpstream,
    patched: macRuleHelpersPatched,
  },
  {
    name: "move rule collection",
    upstream: macMoveRuleCollectorUpstream,
    patched: macMoveRuleCollectorPatched,
  },
  {
    name: "regex move rule collection",
    upstream: macRegexMoveRulesUpstream,
    patched: macRegexMoveRulesPatched,
  },
  {
    name: "glob ancestor move rule collection",
    upstream: macGlobAncestorMoveRulesUpstream,
    patched: macGlobAncestorMoveRulesPatched,
  },
  {
    name: "literal move rule collection",
    upstream: macLiteralMoveRulesUpstream,
    patched: macLiteralMoveRulesPatched,
  },
  {
    name: "move rule rendering",
    upstream: macMoveRuleReturnUpstream,
    patched: macMoveRuleReturnPatched,
  },
  {
    name: "read deny consolidation",
    upstream: macReadDenyRulesUpstream,
    patched: macReadDenyRulesPatched,
  },
  {
    name: "read allow consolidation",
    upstream: macReadAllowRulesUpstream,
    patched: macReadAllowRulesPatched,
  },
  {
    name: "nested read deny consolidation",
    upstream: macNestedReadDenyRulesUpstream,
    patched: macNestedReadDenyRulesPatched,
  },
  {
    name: "read move rule iteration",
    upstream: macReadMoveAppendUpstream,
    patched: macReadMoveAppendPatched,
  },
  {
    name: "write-root move allow consolidation",
    upstream: macWriteAllowMoveRulesUpstream,
    patched: macWriteAllowMoveRulesPatched,
  },
  {
    name: "write allow consolidation",
    upstream: macWriteAllowRulesUpstream,
    patched: macWriteAllowRulesPatched,
  },
  {
    name: "write deny consolidation",
    upstream: macWriteDenyRulesUpstream,
    patched: macWriteDenyRulesPatched,
  },
  {
    name: "write move rule iteration",
    upstream: macWriteMoveAppendUpstream,
    patched: macWriteMoveAppendPatched,
  },
  {
    name: "profile rule iteration",
    upstream: macProfileRuleAppendUpstream,
    patched: macProfileRuleAppendPatched,
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
    let invocationLinkCreated = false;
    try {
      symlinkSync(invocationTarget, invocationLink);
      invocationLinkCreated = true;
    } catch (error) {
      if (!(process.platform === "win32" && error?.code === "EPERM")) {
        throw error;
      }
      // Non-developer Windows accounts cannot create symlinks. Unix CI still
      // exercises the canonicalized direct-invocation path on every change.
    }
    if (invocationLinkCreated) {
      assert.equal(
        isDirectInvocation(invocationLink, pathToFileURL(invocationTarget).href),
        true,
      );
    }
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
