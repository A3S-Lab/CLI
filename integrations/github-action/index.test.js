"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  assertTrustedActor,
  fetchPullRequestFiles,
  main,
  publishPullRequestReview,
  resolveReviewTarget,
  runBoundedProcess,
  structuredFailure,
  verifyA3sAutomationSupport
} = require("./index");

function writeEvent(directory, payload) {
  const eventPath = path.join(directory, "event.json");
  fs.writeFileSync(eventPath, JSON.stringify(payload), "utf8");
  return eventPath;
}

test("trusted actor check accepts only repository write authority", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  let request;
  global.fetch = async (url, options) => {
    request = { url, options };
    return new Response(JSON.stringify({ permission: "write", role_name: "maintain" }), {
      status: 200,
      headers: { "content-type": "application/json" }
    });
  };
  await assertTrustedActor(
    { githubToken: "secret", allowedActors: [] },
    { GITHUB_ACTOR: "octocat", GITHUB_REPOSITORY: "A3S-Lab/CLI" }
  );
  assert.match(request.url, /A3S-Lab\/CLI\/collaborators\/octocat\/permission$/);
  assert.equal(request.options.redirect, "error");
  assert.equal(request.options.headers.Authorization, "Bearer secret");

  global.fetch = async () => new Response(JSON.stringify({ permission: "read" }), { status: 200 });
  await assert.rejects(
    assertTrustedActor(
      { githubToken: "secret", allowedActors: [] },
      { GITHUB_ACTOR: "reader", GITHUB_REPOSITORY: "A3S-Lab/CLI" }
    ),
    /does not have write/
  );
});

test("explicit actor allowlist avoids the permission request", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  global.fetch = async () => {
    throw new Error("fetch should not run");
  };
  await assertTrustedActor(
    { githubToken: "secret", allowedActors: ["dependabot[bot]"] },
    { GITHUB_ACTOR: "dependabot[bot]", GITHUB_REPOSITORY: "A3S-Lab/CLI" }
  );
});

test("review target accepts pull_request and mention-triggered issue comments", () => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "a3s-action-event-"));
  try {
    const common = {
      GITHUB_REPOSITORY: "A3S-Lab/CLI",
      GITHUB_EVENT_PATH: writeEvent(fixture, {
        repository: { full_name: "A3S-Lab/CLI" },
        pull_request: { number: 42 }
      }),
      GITHUB_EVENT_NAME: "pull_request"
    };
    assert.deepEqual(
      resolveReviewTarget({ publishReview: true, pullRequestNumber: null }, common),
      { owner: "A3S-Lab", repo: "CLI", fullName: "A3S-Lab/CLI", number: 42 }
    );
    common.GITHUB_EVENT_PATH = writeEvent(fixture, {
      repository: { full_name: "A3S-Lab/CLI" },
      issue: { number: 43, pull_request: { url: "https://example.test" } },
      comment: { body: "please @a3s review this" }
    });
    common.GITHUB_EVENT_NAME = "issue_comment";
    assert.equal(
      resolveReviewTarget({ publishReview: true, pullRequestNumber: null }, common).number,
      43
    );
  } finally {
    fs.rmSync(fixture, { recursive: true, force: true });
  }
});

test("pull request files and review publication use bounded GitHub APIs", async () => {
  const requests = [];
  const fakeFetch = async (url, options) => {
    requests.push({ url, options });
    if (options.method === "POST") {
      return new Response(JSON.stringify({ html_url: "https://github.com/A3S-Lab/CLI/pull/7#pullrequestreview-1" }), { status: 200 });
    }
    return new Response(JSON.stringify([{
      filename: "src/app.js",
      status: "modified",
      additions: 1,
      deletions: 0,
      patch: "@@ -1 +1 @@\n-old\n+new"
    }]), { status: 200 });
  };
  const inputs = { githubToken: "secret" };
  const target = { owner: "A3S-Lab", repo: "CLI", number: 7 };
  const files = await fetchPullRequestFiles(inputs, target, fakeFetch);
  assert.equal(files[0].filename, "src/app.js");
  const published = await publishPullRequestReview(inputs, target, {
    summary: "One issue",
    findings: [{
      priority: "P1",
      path: "src/app.js",
      line: 1,
      side: "RIGHT",
      title: "Regression",
      body: "The replacement breaks callers."
    }]
  }, fakeFetch);
  assert.equal(published.findings, 1);
  assert.match(published.url, /pullrequestreview-1/);
  const posted = JSON.parse(requests[1].options.body);
  assert.equal(posted.event, "COMMENT");
  assert.equal(posted.comments[0].path, "src/app.js");
  assert.equal(requests[1].options.headers.Authorization, "Bearer secret");
});

test("pull request file lookup probes past the cap and rejects incomplete reviews", async () => {
  let calls = 0;
  const fakeFetch = async () => {
    calls += 1;
    const count = calls <= 10 ? 100 : 1;
    return new Response(JSON.stringify(Array.from({ length: count }, (_, index) => ({
      filename: `src/page-${calls}-file-${index}.js`,
      status: "modified",
      additions: 1,
      deletions: 0,
      patch: "@@ -1 +1 @@\n-old\n+new"
    }))), { status: 200 });
  };
  await assert.rejects(
    fetchPullRequestFiles(
      { githubToken: "secret" },
      { owner: "A3S-Lab", repo: "CLI", number: 7 },
      fakeFetch
    ),
    /more than 1000 files/
  );
  assert.equal(calls, 11);
});

test("bounded process passes stdin without shell interpolation", async () => {
  const input = "$(echo should-not-run)\n::warning::still-data";
  const result = await runBoundedProcess(
    process.execPath,
    ["-e", "process.stdin.pipe(process.stdout)"],
    {
      cwd: process.cwd(),
      env: process.env,
      input,
      timeoutMs: 10_000,
      maxStdoutBytes: 1024,
      label: "fixture"
    }
  );
  assert.equal(result.code, 0);
  assert.equal(result.stdout.toString("utf8"), input);
});

test("bounded process terminates its process group at the deadline", async () => {
  await assert.rejects(
    runBoundedProcess(
      process.execPath,
      ["-e", "setInterval(() => {}, 1000)"],
      {
        cwd: process.cwd(),
        env: process.env,
        input: "",
        timeoutMs: 100,
        maxStdoutBytes: 1024,
        label: "deadline fixture"
      }
    ),
    /exceeded its deadline/
  );
});

test("structured failures expose only the bounded CLI error code", () => {
  const result = {
    code: 1,
    stdout: Buffer.from(JSON.stringify({
      schemaVersion: 1,
      ok: false,
      error: { code: "approval.required", message: "review needed" }
    })),
    stderr: "fallback"
  };
  assert.equal(
    structuredFailure(result),
    "A3S Code failed (approval.required); child output was not logged"
  );
});

test("compatibility check requires the closed tool-policy CLI contract", async () => {
  const runner = async () => ({
    code: 0,
    signal: null,
    stdout: Buffer.from("--tool-policy <TOOL_POLICY>\nworkspace-write"),
    stderr: ""
  });
  await verifyA3sAutomationSupport("a3s", process.cwd(), process.env, runner);
  await assert.rejects(
    verifyA3sAutomationSupport("old-a3s", process.cwd(), process.env, async () => ({
      code: 0,
      signal: null,
      stdout: Buffer.from("legacy help"),
      stderr: ""
    })),
    /predates closed automation profiles/
  );
});

test("action orchestration keeps prompt off argv and scrubs the model process", async () => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "a3s-action-index-"));
  try {
    const outputPath = path.join(fixture, "github-output.txt");
    fs.writeFileSync(outputPath, "", "utf8");
    let invocation;
    const environment = {
      "INPUT_GITHUB-TOKEN": "github-secret",
      INPUT_PROMPT: "Review without shell interpolation: $(whoami)",
      INPUT_PERMISSIONS: "workspace-write",
      INPUT_TIMEOUT_SECONDS: "60",
      GITHUB_WORKSPACE: fixture,
      GITHUB_OUTPUT: outputPath,
      GITHUB_ACTOR: "maintainer",
      GITHUB_REPOSITORY: "A3S-Lab/CLI",
      RUNNER_TEMP: fixture,
      OPENAI_API_KEY: "provider-secret",
      PATH: process.env.PATH
    };
    const result = Buffer.from(JSON.stringify({
      schemaVersion: 1,
      command: "code.exec",
      ok: true,
      data: {
        text: "review complete",
        sessionId: "session-1",
        usage: { totalTokens: 2 },
        toolPolicy: "workspace-write"
      }
    }));
    await main(environment, {
      command() {},
      info() {},
      async assertTrustedActor() {},
      async installA3s() {
        return "fixture-a3s";
      },
      async verifyA3sAutomationSupport() {},
      async runBoundedProcess(executable, args, options) {
        invocation = { executable, args, options };
        return { code: 0, signal: null, stdout: result, stderr: "" };
      }
    });

    assert.equal(invocation.executable, "fixture-a3s");
    assert.equal(invocation.options.input, environment.INPUT_PROMPT);
    assert.equal(invocation.args.includes(environment.INPUT_PROMPT), false);
    assert.deepEqual(
      invocation.args.slice(invocation.args.indexOf("code")),
      ["code", "exec", "--mode", "auto", "--tool-policy", "workspace-write"]
    );
    assert.equal(invocation.options.env.OPENAI_API_KEY, "provider-secret");
    assert.equal(invocation.options.env.GITHUB_OUTPUT, undefined);
    assert.equal(invocation.options.env["INPUT_GITHUB-TOKEN"], undefined);
    assert.equal(invocation.options.env.GITHUB_REPOSITORY, undefined);

    const outputs = fs.readFileSync(outputPath, "utf8");
    assert.match(outputs, /final-message<<[^\n]+\nreview complete\n/);
    assert.match(outputs, /final-message-truncated<<[^\n]+\nfalse\n/);
    assert.match(outputs, /session-id<<[^\n]+\nsession-1\n/);
    assert.equal(outputs.includes("github-secret"), false);
  } finally {
    fs.rmSync(fixture, { recursive: true, force: true });
  }
});

test("action publication validates model findings before calling GitHub", async () => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "a3s-action-review-"));
  try {
    const outputPath = path.join(fixture, "github-output.txt");
    fs.writeFileSync(outputPath, "", "utf8");
    const eventPath = writeEvent(fixture, {
      repository: { full_name: "A3S-Lab/CLI" },
      pull_request: { number: 9 }
    });
    const environment = {
      "INPUT_GITHUB-TOKEN": "github-secret",
      INPUT_PROMPT: "Review the change",
      INPUT_PERMISSIONS: "read-only",
      INPUT_PUBLISH_REVIEW: "true",
      INPUT_TIMEOUT_SECONDS: "60",
      GITHUB_WORKSPACE: fixture,
      GITHUB_OUTPUT: outputPath,
      GITHUB_ACTOR: "maintainer",
      GITHUB_REPOSITORY: "A3S-Lab/CLI",
      GITHUB_EVENT_NAME: "pull_request",
      GITHUB_EVENT_PATH: eventPath,
      RUNNER_TEMP: fixture,
      PATH: process.env.PATH
    };
    const files = [{
      filename: "src/app.js",
      status: "modified",
      additions: 1,
      deletions: 0,
      patch: "@@ -1 +1 @@\n-old\n+new"
    }];
    let childPrompt = "";
    let publishedReview;
    await main(environment, {
      command() {},
      info() {},
      async assertTrustedActor() {},
      async installA3s() { return "fixture-a3s"; },
      async verifyA3sAutomationSupport() {},
      async fetchPullRequestFiles() { return files; },
      async publishPullRequestReview(_inputs, target, review) {
        publishedReview = { target, review };
        return { url: "https://github.com/review/9", findings: review.findings.length };
      },
      async runBoundedProcess(_executable, _args, options) {
        childPrompt = options.input;
        return {
          code: 0,
          signal: null,
          stderr: "",
          stdout: Buffer.from(JSON.stringify({
            schemaVersion: 1,
            command: "code.exec",
            ok: true,
            data: {
              text: "```a3s-review\n{\"summary\":\"One issue\",\"findings\":[{\"priority\":\"P1\",\"path\":\"src/app.js\",\"line\":1,\"side\":\"RIGHT\",\"title\":\"Regression\",\"body\":\"The new line breaks callers.\"}]}\n```",
              sessionId: "session-review",
              usage: {},
              toolPolicy: "read-only"
            }
          }))
        };
      }
    });
    assert.match(childPrompt, /untrusted pull-request data/);
    assert.equal(publishedReview.target.number, 9);
    assert.equal(publishedReview.review.findings[0].priority, "P1");
    const outputs = fs.readFileSync(outputPath, "utf8");
    assert.match(outputs, /review-published<<[^\n]+\ntrue\n/);
    assert.match(outputs, /review-findings<<[^\n]+\n1\n/);
  } finally {
    fs.rmSync(fixture, { recursive: true, force: true });
  }
});
