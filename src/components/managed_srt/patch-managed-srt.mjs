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

function removeCoveredLiteralPaths(paths) {
  const retained = [];
  const retainedSet = new Set();
  const ordered = [...new Set(paths)].sort(
    (left, right) =>
      left.length - right.length || (left < right ? -1 : left > right ? 1 : 0),
  );
  for (const candidate of ordered) {
    let covered = retainedSet.has(candidate);
    let boundary = candidate.length;
    while (!covered && boundary > 0) {
      boundary = candidate.lastIndexOf("/", boundary - 1);
      if (boundary < 0) {
        break;
      }
      const ancestor = boundary === 0 ? "/" : candidate.slice(0, boundary);
      covered = retainedSet.has(ancestor);
    }
    if (!covered) {
      retained.push(candidate);
      retainedSet.add(candidate);
    }
  }
  return retained.sort();
}

function escapeRegexLiteral(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function renderLiteralPathTrie(node) {
  const groupedBranches = new Map();
  for (const [character, child] of [...node.children.entries()].sort(
    ([left], [right]) => (left < right ? -1 : left > right ? 1 : 0),
  )) {
    const suffix = renderLiteralPathTrie(child);
    const characters = groupedBranches.get(suffix) ?? [];
    characters.push(character);
    groupedBranches.set(suffix, characters);
  }
  const branches = [];
  for (const [suffix, characters] of groupedBranches) {
    if (
      characters.length > 1 &&
      characters.every((character) => /^[A-Za-z0-9_]$/.test(character))
    ) {
      branches.push(`[${characters.join("")}]${suffix}`);
    } else {
      for (const character of characters) {
        branches.push(escapeRegexLiteral(character) + suffix);
      }
    }
  }
  if (branches.length === 0) {
    return "";
  }
  const continuation =
    branches.length === 1 ? branches[0] : `(${branches.join("|")})`;
  return node.terminal ? `(${continuation})?` : continuation;
}

function buildLiteralSubpathRegex(paths) {
  const root = { terminal: false, children: new Map() };
  for (const literalPath of paths) {
    let node = root;
    for (const character of literalPath) {
      let child = node.children.get(character);
      if (!child) {
        child = { terminal: false, children: new Map() };
        node.children.set(character, child);
      }
      node = child;
    }
    node.terminal = true;
  }
  return `^${renderLiteralPathTrie(root)}(/.*)?$`;
}

function buildBoundedLiteralSubpathRegexes(paths) {
  const regexes = [];
  const appendBoundedRegex = (chunk) => {
    const regex = buildLiteralSubpathRegex(chunk);
    if (Buffer.byteLength(JSON.stringify(regex)) <= 1_000) {
      regexes.push(regex);
      return;
    }
    if (chunk.length === 1) {
      throw new Error("literal path exceeds the Seatbelt string limit");
    }
    const midpoint = Math.ceil(chunk.length / 2);
    appendBoundedRegex(chunk.slice(0, midpoint));
    appendBoundedRegex(chunk.slice(midpoint));
  };
  appendBoundedRegex(paths);
  return regexes;
}

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
${removeCoveredLiteralPaths.toString()}
${escapeRegexLiteral.toString()}
${renderLiteralPathTrie.toString()}
${buildLiteralSubpathRegex.toString()}
${buildBoundedLiteralSubpathRegexes.toString()}
/**
 * Consolidate large literal subpath sets into exact finite regex tries. The
 * regexes retain subpath semantics without widening access to sibling paths.
 * Equivalent suffixes share character classes, while adaptive splits keep
 * every encoded string below Seatbelt's parser limit.
 */
function compactPathFilters(normalizedPaths) {
    const filters = new Set();
    const literalPaths = [];
    for (const normalizedPath of normalizedPaths) {
        if (containsGlobChars(normalizedPath)) {
            filters.add(pathFilter(normalizedPath));
        }
        else {
            literalPaths.push(normalizedPath);
        }
    }
    const compacted = removeCoveredLiteralPaths(literalPaths);
    if (compacted.length < 32) {
        for (const literalPath of compacted) {
            filters.add(pathFilter(literalPath));
        }
        return filters;
    }
    for (let offset = 0; offset < compacted.length; offset += 512) {
        const chunk = compacted.slice(offset, offset + 512);
        for (const regex of buildBoundedLiteralSubpathRegexes(chunk)) {
            filters.add(\`(regex \${escapePath(regex)})\`);
        }
    }
    return filters;
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
    const protectedPaths = [];
    for (const pathPattern of pathPatterns) {`;

const macRegexMoveRulesUpstream = `            // Use regex matching for glob patterns
            const regexPattern = globToRegex(normalizedPath);
            // Block moving/renaming files matching this pattern
            for (const op of ops) {
                rules.push(\`(deny \${op}\`, \`  (regex \${escapePath(regexPattern)})\`, \`  (with message "\${logTag}"))\`);
            }`;

const macRegexMoveRulesPatched = `            // Block moving/renaming files matching this pattern.
            protectedPaths.push(normalizedPath);`;

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
            protectedPaths.push(normalizedPath);
            // Block moves of ancestor directories
            for (const ancestorDir of getAncestorDirectories(normalizedPath)) {
                filters.add(\`(literal \${escapePath(ancestorDir)})\`);
            }`;

const macMoveRuleReturnUpstream = `    return rules;
}
/**
 * Generate filesystem read rules for sandbox profile`;

const macMoveRuleReturnPatched = `    for (const filter of compactPathFilters(protectedPaths)) {
        filters.add(filter);
    }
    return renderRule(
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
    const denyPaths = [];
    for (const pathPattern of config.denyOnly || []) {
        const normalizedPath = normalizePathForSandbox(pathPattern);
        if (normalizedPath === '/')
            deniesRoot = true;
        denyPaths.push(normalizedPath);
    }
    const denyFilters = compactPathFilters(denyPaths);
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
    const allowPaths = [];
    for (const pathPattern of config.allowWithinDeny || []) {
        const normalizedPath = normalizePathForSandbox(pathPattern);
        if (!containsGlobChars(normalizedPath)) {
            allowedSubpaths.push(normalizedPath);
        }
        allowPaths.push(normalizedPath);
    }
    const allowFilters = compactPathFilters(allowPaths);
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

const macNestedReadDenyRulesPatched = `    const nestedDenyPaths = [];
    for (const denyPath of config.denyOnly || []) {
        if (containsGlobChars(denyPath))
            continue;
        const normalized = normalizePathForSandbox(denyPath);
        if (allowedSubpaths.some(a => normalized.startsWith(a + '/'))) {
            nestedDenyPaths.push(normalized);
        }
    }
    const nestedDenyFilters = compactPathFilters(nestedDenyPaths);
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

const macWriteAllowMoveRulesPatched = `    const writeAllowPathsNormalized = [];
    for (const pathPattern of writeAllowPaths || []) {
        writeAllowPathsNormalized.push(normalizePathForSandbox(pathPattern));
    }
    const writeAllowFilters = compactPathFilters(writeAllowPathsNormalized);
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
    const allowPaths = [];
    for (const pathPattern of config.allowOnly || []) {
        allowPaths.push(normalizePathForSandbox(pathPattern));
    }
    const allowFilters = compactPathFilters(allowPaths);
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

const macWriteDenyRulesPatched = `    const normalizedDenyPaths = [];
    for (const pathPattern of denyPaths) {
        normalizedDenyPaths.push(normalizePathForSandbox(pathPattern));
    }
    const denyFilters = compactPathFilters(normalizedDenyPaths);
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
    // A function replacer preserves literal `$&`, `$`` and `$'` sequences in
    // generated JavaScript instead of interpreting them as replace tokens.
    source = source.replace(replacement.upstream, () => replacement.patched);
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
    const literalPaths = [
      "/tmp/project/foo",
      "/tmp/project/foo/covered-child",
      "/tmp/project/foobar",
      "/tmp/project/a[1]+$.txt",
    ];
    const retained = removeCoveredLiteralPaths(literalPaths);
    assert.deepEqual(retained, [
      "/tmp/project/a[1]+$.txt",
      "/tmp/project/foo",
      "/tmp/project/foobar",
    ]);
    const literalRegex = new RegExp(buildLiteralSubpathRegex(retained));
    for (const allowed of retained) {
      assert.equal(literalRegex.test(allowed), true);
      assert.equal(literalRegex.test(`${allowed}/descendant`), true);
    }
    assert.equal(literalRegex.test("/tmp/project/foo/covered-child"), true);
    assert.equal(literalRegex.test("/tmp/project/foob"), false);
    assert.equal(literalRegex.test("/tmp/project/foo-sibling"), false);
    assert.equal(literalRegex.test("/tmp/project/a11.txt"), false);

    const largeLiteralSet = Array.from(
      { length: 4_097 },
      (_, index) =>
        "/tmp/project/outside-hardlink-profile-alias-" +
        String(index).padStart(4, "0") +
        ".txt",
    );
    const largeRegexSources = [];
    for (let offset = 0; offset < largeLiteralSet.length; offset += 512) {
      largeRegexSources.push(
        ...buildBoundedLiteralSubpathRegexes(
          largeLiteralSet.slice(offset, offset + 512),
        ),
      );
    }
    assert.equal(largeRegexSources.length < 20, true);
    for (const regex of largeRegexSources) {
      assert.equal(Buffer.byteLength(JSON.stringify(regex)) <= 1_000, true);
    }
    const largeRegexes = largeRegexSources.map((regex) => new RegExp(regex));
    for (const literalPath of largeLiteralSet) {
      assert.equal(largeRegexes.some((regex) => regex.test(literalPath)), true);
    }
    assert.equal(
      largeRegexes.some((regex) =>
        regex.test("/tmp/project/outside-hardlink-profile-alias-4097.txt"),
      ),
      false,
    );

    const irregularPaths = Array.from({ length: 512 }, (_, index) => {
      let state = index + 1;
      let tail = "";
      for (let offset = 0; offset < 32; offset += 1) {
        state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
        tail += state.toString(36).at(-1);
      }
      return `/tmp/project/irregular-${index.toString(36)}-${tail}`;
    });
    const irregularRegexSources =
      buildBoundedLiteralSubpathRegexes(irregularPaths);
    assert.equal(irregularRegexSources.length > 1, true);
    for (const regex of irregularRegexSources) {
      assert.equal(Buffer.byteLength(JSON.stringify(regex)) <= 1_000, true);
    }
    const irregularRegexes = irregularRegexSources.map(
      (regex) => new RegExp(regex),
    );
    for (const literalPath of irregularPaths) {
      assert.equal(
        irregularRegexes.some((regex) => regex.test(literalPath)),
        true,
      );
    }

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
        assert.equal(
          occurrenceCount(patched, replacement.upstream),
          0,
          replacement.name,
        );
        assert.equal(
          occurrenceCount(patched, replacement.patched),
          1,
          replacement.name,
        );
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
