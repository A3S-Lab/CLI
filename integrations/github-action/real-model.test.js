"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const { buildReviewPrompt, parseReviewProtocol } = require("./core");

const enabled = process.env.A3S_REAL_LLM_ACTION === "1";

test(
  "real configured model emits a publishable bounded PR review protocol",
  { skip: !enabled, timeout: 360_000 },
  () => {
    const executable = process.env.A3S_REAL_LLM_BIN;
    const config = process.env.A3S_REAL_LLM_CONFIG;
    const model = process.env.A3S_REAL_LLM_MODEL || "deepseek/deepseek-v4-flash";
    assert.ok(executable && fs.statSync(executable).isFile(), "A3S_REAL_LLM_BIN must be a file");
    assert.ok(config && fs.statSync(config).isFile(), "A3S_REAL_LLM_CONFIG must be a file");

    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "a3s-action-real-model-"));
    try {
      const files = [{
        filename: "src/auth.js",
        status: "modified",
        additions: 1,
        deletions: 1,
        patch: "@@ -1,3 +1,3 @@\n function isAdmin(user) {\n-  return user.role === \"admin\";\n+  return true;\n }"
      }];
      const prompt = buildReviewPrompt(
        "Review the supplied diff without tools. The added allow-all return is a high-priority authorization bypass; report that concrete P1 finding on the added line.",
        files
      );
      const run = spawnSync(executable, [
        "--output", "json",
        "--non-interactive",
        "--no-progress",
        "--config", config,
        "--directory", workspace,
        "code", "exec",
        "--mode", "default",
        "--tool-policy", "read-only",
        "--model", model
      ], {
        cwd: workspace,
        env: {
          ...process.env,
          HOME: workspace,
          A3S_STATE_HOME: path.join(workspace, "state")
        },
        input: prompt,
        encoding: "utf8",
        maxBuffer: 32 * 1024 * 1024,
        timeout: 300_000,
        windowsHide: true
      });
      assert.equal(
        run.status,
        0,
        `real A3S review probe failed: ${String(run.stderr || "").slice(0, 4096)}`
      );
      const result = JSON.parse(run.stdout);
      assert.equal(result.ok, true);
      assert.equal(result.data.toolPolicy, "read-only");
      const review = parseReviewProtocol(result.data.text, files);
      assert.equal(review.findings.length, 1);
      assert.equal(review.findings[0].priority, "P1");
      assert.equal(review.findings[0].path, "src/auth.js");
      assert.equal(review.findings[0].line, 2);
      assert.equal(review.findings[0].side, "RIGHT");
    } finally {
      fs.rmSync(workspace, { recursive: true, force: true });
    }
  }
);
