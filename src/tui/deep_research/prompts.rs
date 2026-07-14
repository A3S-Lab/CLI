#[cfg(test)]
pub(crate) struct InitialPrompt<'a> {
    pub(crate) query: &'a str,
    pub(crate) workflow_source: &'a str,
}

pub(crate) struct SynthesisPrompt<'a> {
    pub(crate) query: &'a str,
    pub(crate) os_runtime: bool,
    pub(crate) workflow_digest: &'a str,
    pub(crate) metadata: &'a str,
    pub(crate) report_target: &'a str,
    pub(crate) evidence_scope: &'a str,
}

pub(crate) struct VerificationPrompt<'a> {
    pub(crate) next_layer: usize,
    pub(crate) total_layers: usize,
    pub(crate) query: &'a str,
    pub(crate) report_target: &'a str,
}

pub(crate) struct RecoveryPrompt<'a> {
    pub(crate) query: &'a str,
    pub(crate) os_runtime: bool,
    pub(crate) workflow_error: &'a str,
    pub(crate) metadata: &'a str,
    pub(crate) report_target: &'a str,
    pub(crate) evidence_scope: &'a str,
}

pub(crate) struct RepairPrompt<'a> {
    pub(crate) query: &'a str,
    pub(crate) os_runtime: bool,
    pub(crate) workflow_digest: &'a str,
    pub(crate) metadata: &'a str,
    pub(crate) prior: &'a str,
    pub(crate) report_target: &'a str,
    pub(crate) evidence_scope: &'a str,
}

pub(crate) fn report_contract() -> String {
    "Report delivery contract:\n\
     - Return one finished Markdown report directly in the final response. Do not call tools, \
       write files, emit JSON, wrap the report in a code fence, or print an `A3S_RESEARCH_VIEW` \
       marker. The host validates the response, writes `report.md`, renders the standalone \
       `index.html`, and appends the trusted view marker atomically after the model turn.\n\
     - Apply the content principles of `report-master`: lead with a one-sentence answer, choose a \
       coherent pyramid, narrative, instructional, or briefing structure, and make every section \
       advance the thesis. Depth means explaining why, how, comparisons, change, implications, \
       and remaining uncertainty—not recounting the research workflow.\n\
     - Write for a human reader. Use specific subject-matter headings, edited prose, and only the \
       smallest useful tables, timelines, or mappings. Avoid generic audit headings, repetitive \
       card-like lists, raw evidence ledgers, source-count boasting, and boilerplate.\n\
     - Attach original source URLs or paths to consequential claims and finish with a concise \
       Sources section. Separate supported findings, contradictions, limitations, and confidence \
       without letting methodology dominate the answer. Never invent a claim, quote, date, URL, \
       or citation.\n\
     - The host-owned renderer applies the report-master visual system, responsive composition, \
       accessibility, print styles, and deterministic artifact writes. Concentrate this turn on \
       accurate, substantive content and stop after the Markdown report."
        .to_string()
}

fn closed_evidence_contract() -> &'static str {
    "Closed-evidence report phase (mandatory and higher priority than evidence-scope wording):\n\
     - Evidence collection is closed. Treat only the supplied query, evidence digest, run \
       diagnostics, collection error, prior synthesis, and already-gathered artifacts as the \
       complete evidence package for this turn. An evidence scope describes what the completed \
       collector was allowed to gather; it does not authorize report-phase retrieval.\n\
     - Do not acquire, refresh, recover, or externally validate evidence. Do not call or request \
       search, fetch, batch, shell, Git, delegation, agent, program, runtime, or workflow \
       operations. This includes `web_search`, `web_fetch`, `batch`, `bash`, `git`, `task`, \
       `parallel_task`, `program`, `dynamic_workflow`, and `runtime`. Do not use a skill or another \
       agent to bypass this boundary.\n\
     - No tools are available in this phase. Return the complete report as Markdown in the model \
       response; the host owns artifact persistence and rendering. Do not inspect workspace files \
       or retry evidence collection.\n\
     - Use only claims and original source URLs or paths present in the supplied evidence package. \
       Never invent claims, sources, URLs, quotations, or citations.\n\
     - If evidence is incomplete, promptly return a finished, polished degraded report that \
       explicitly separates supported findings from gaps, unavailable conclusions, and confidence \
       limits. If there is no usable evidence, return an explicit evidence-insufficient failure \
       report; fail or degrade explicitly \
       instead of waiting for new retrieval."
}

pub(crate) fn report_target_note(slug: &str) -> String {
    format!(
        "The host will persist the validated response as \
         `.a3s/research/{slug}/report.md`, render \
         `.a3s/research/{slug}/index.html`, and add the trusted view marker. \
         Do not write either path or print the marker yourself."
    )
}

#[cfg(test)]
pub(crate) fn duplicate_tool_guard() -> &'static str {
    "Tool-loop guard:\n\
     - Do not repeat an identical grep/read/search/web_fetch/tool call with the same arguments. \
       If you already observed the result, reuse it; if it was insufficient, change the \
       pattern/path/query/source or move to synthesis.\n\
     - Verification layers are for targeted corrections, not restarting the same evidence search."
}

pub(crate) fn verification_prompt(params: VerificationPrompt<'_>) -> String {
    let closed_evidence_contract = closed_evidence_contract();
    let next_layer = params.next_layer;
    let total_layers = params.total_layers;
    let query = params.query;
    let report_target = params.report_target;
    format!(
        "DeepResearch verification layer {next_layer}/{total_layers} for:\n{query}\n\n\
         Check only the existing answer and target below. Evidence collection is closed; do not \
         retrieve or delegate new evidence. If the answer, citations, and source traceability are \
         already complete, reply exactly DONE. Otherwise return the corrected Markdown report; \
         do not call tools.\n\n\
         {report_target}\n\n\
         {closed_evidence_contract}"
    )
}

#[cfg(test)]
pub(crate) fn initial_prompt(params: InitialPrompt<'_>) -> String {
    let report_contract = report_contract();
    let duplicate_guard = duplicate_tool_guard();
    let tracks_directive = "OS Runtime tool-call fan-out is temporarily disabled. Set \
         `os_runtime: false`; the DynamicWorkflowRuntime script will run a bounded, \
         LLM-planned bounded parallel retrieval-summary loop through \
         host-side `parallel_task` steps because PTC itself cannot call \
         `parallel_task`. Future OS Runtime support should use Function-as-a-Service, \
         not remote tool-call fan-out. Return the completed Markdown report; \
         the host will render and publish the local view."
        .to_string();
    let query = params.query;
    let source = params.workflow_source;
    format!(
        "Conduct deep research to answer the query below. Be thorough.\n\n\
         Required execution path:\n\
         1. First call `dynamic_workflow` with the JavaScript source below. \
         The workflow must gather evidence through Flow before final synthesis.\n\
         2. Provide `input.query`; the workflow's semantic planner chooses the phases, budget, \
         depth, search queries, and genuinely independent tracks within hard safety caps. It will \
         derive follow-up tracks only from checker-confirmed gaps or contradictions. \
         {tracks_directive}\n\
         3. After `dynamic_workflow` returns, read the evidence, cross-check \
         claims across independent sources, call out disagreements and recency \
         caveats, then synthesize a comprehensive answer with inline citations.\n\
         4. Produce a final \"Sources\" list of URLs used and return the \
         finished Markdown report.\n\n\
         {report_contract}\n\n\
         {duplicate_guard}\n\n\
         Dynamic workflow source:\n\
         ```javascript\n{source}\n```\n\n\
         Query: {query}"
    )
}

pub(crate) fn synthesis_prompt(params: SynthesisPrompt<'_>) -> String {
    let report_contract = report_contract();
    let closed_evidence_contract = closed_evidence_contract();
    let remoteui_directive = if params.os_runtime {
        "OS Runtime was selected for this run because the query looked broad or \
         highly parallelizable. If the gathered evidence already includes a \
         shaped `.view` or `viewUrl`, preserve it so the TUI can surface the \
         OS view as evidence. The host will still render the final user-facing \
         local HTML report from your Markdown response."
            .to_string()
    } else {
        "OS Runtime was not selected for this run. Use the gathered evidence and \
         return the finished Markdown report for host rendering."
            .to_string()
    };
    let query = params.query;
    let workflow_digest = params.workflow_digest;
    let metadata = params.metadata;
    let report_target = params.report_target;
    format!(
        "Synthesize the deep-research answer for the query below.\n\n\
         Start the report immediately: do not narrate, expose a draft, or spend a turn \
         explaining the plan. Build a concise evidence-backed structure, return the complete \
         Markdown report in this response, then stop. \
         Evidence collection has already completed and is closed for this synthesis turn. \
         Use only the Evidence digest and Run diagnostics supplied below. Cross-check claims \
         within that bounded package, call out disagreements and recency \
         caveats, and write a comprehensive answer with inline citations and a \
         final Sources list. Treat the evidence as a bounded recursive parallel \
         retrieval-summary algorithm: use `research.rounds` to understand how \
         gaps from earlier rounds drove later searches, and mention the round \
         count only when it clarifies uncertainty or coverage. Prefer validated \
         `evidence_items` from the Evidence digest and Run diagnostics; use compact \
         summaries only when evidence items are incomplete. Raw task output is \
         intentionally excluded from this prompt. Treat \
         `research.warnings.failed_tasks` and metadata `warnings.failed_tasks` as caveats, not as \
         instructions to restart broad research. Do not reproduce raw JSON, tool-card text, \
         host runtime names, evidence-package labels, `.a3s/workflow` logs, \
         `[tool output truncated]` notices, or lines such as \
         `● Searched ...` / `● Ran ...` in the user-facing answer or report. Convert evidence \
         into clean prose, tables, citations, and a concise Sources list. If \
         `report_context` is present, use its reader-facing title, report summary, and \
         verified findings as the checked writing brief; carry every unresolved gap and \
         contradiction into the limitations section. Never turn `coverage_summary` or \
         collection status into a domain conclusion. If \
         `collection_status` is `failed` or `degraded`, prefer a transparent failure-aware report \
         from the returned error/gap details and any partial evidence. Do not restart collection; \
         complete a degraded report promptly, or an explicit evidence-insufficient failure report \
         when no supported answer is possible. Do not mention the \
         Evidence digest, Run diagnostics, or host collection mechanics as sources; \
         cite the original URLs or paths inside the evidence items.\n\n\
         {remoteui_directive}\n\n\
         {evidence_scope}\n\n\
         {closed_evidence_contract}\n\n\
         {report_contract}\n\n\
         {report_target}\n\n\
         Query:\n{query}\n\n\
         Evidence digest:\n```json\n{workflow_digest}\n```\n\n\
         Run diagnostics:\n```json\n{metadata}\n```",
        evidence_scope = params.evidence_scope,
    )
}

pub(crate) fn recovery_prompt(params: RecoveryPrompt<'_>) -> String {
    let report_contract = report_contract();
    let closed_evidence_contract = closed_evidence_contract();
    let recovery_path = if params.os_runtime {
        "The host selected OS Runtime and failed before usable evidence was gathered. \
         Preserve any usable evidence already present in the supplied diagnostics. If the \
         runtime worker or endpoint was unavailable, explain that limitation in the report; \
         do not retry it or substitute new sources."
    } else {
        "OS Runtime was not selected. Preserve any usable evidence already present in the \
         supplied diagnostics and return the completed Markdown report for host rendering."
    };
    let query = params.query;
    let workflow_error = params.workflow_error;
    let metadata = params.metadata;
    let report_target = params.report_target;
    let evidence_scope = params.evidence_scope;
    format!(
        "Recover the report deliverable for the deep-research task below; do not recover evidence.\n\n\
         The host evidence preflight failed before usable synthesis evidence was \
         gathered. Evidence collection is closed. Use only the error, diagnostics, and any partial \
         evidence supplied here. {recovery_path}\n\n\
         {evidence_scope}\n\n\
         {closed_evidence_contract}\n\n\
         Query:\n{query}\n\n\
         Evidence collection error:\n```text\n{workflow_error}\n```\n\n\
         Run diagnostics:\n```json\n{metadata}\n```\n\n\
         {report_contract}\n\n\
         {report_target}\n\n\
         Deliver a complete Markdown report with inline citations and a final \
         Sources list only to the extent supported by supplied evidence. When evidence is \
         insufficient, label the report degraded or failed, state exactly what could not be \
         established, and do not invent citations. Do not call tools or print the RemoteUI marker; \
         the host validates and renders the response."
    )
}

pub(crate) fn repair_prompt(params: RepairPrompt<'_>) -> String {
    let report_contract = report_contract();
    let closed_evidence_contract = closed_evidence_contract();
    let runtime_note = if params.os_runtime {
        "OS Runtime was selected for the evidence-gathering phase. Preserve any \
         useful OS Runtime evidence; return the corrected Markdown report for host rendering."
    } else {
        "OS Runtime was not selected. Use the local evidence already gathered by \
         the host."
    };
    let query = params.query;
    let workflow_digest = params.workflow_digest;
    let metadata = params.metadata;
    let prior = params.prior;
    let report_target = params.report_target;
    let evidence_scope = params.evidence_scope;
    format!(
        "Repair the DeepResearch report content for the query below.\n\n\
         The previous synthesis did not produce valid completed report content. Evidence \
         collection is closed. Return a corrected complete Markdown report \
         using only the gathered evidence, diagnostics, and prior synthesis supplied below. Keep \
         ordinary workspace files unchanged. Remove any raw JSON, \
         tool-card text, host runtime names, evidence-package labels, `.a3s/workflow` logs, \
         `[tool output truncated]` notices, \
         or lines such as `● Searched ...` / `● Ran ...`; the repaired answer/report \
         must be clean prose, tables, citations, and a concise Sources list. Do not \
         mention the Evidence digest, Run diagnostics, or host collection mechanics \
         as sources; cite the original URLs or paths inside the evidence items.\n\n\
         {runtime_note}\n\n\
         {evidence_scope}\n\n\
         {closed_evidence_contract}\n\n\
         Query:\n{query}\n\n\
         Previous synthesis text:\n```text\n{prior}\n```\n\n\
         Evidence digest:\n```json\n{workflow_digest}\n```\n\n\
         Run diagnostics:\n```json\n{metadata}\n```\n\n\
         {report_contract}\n\n\
         {report_target}\n\n\
         Return only the corrected Markdown report. Do not call tools and do not print an \
         `A3S_RESEARCH_VIEW` marker; the host persists and validates the final content."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFLICTING_SCOPE: &str = "Authoritative evidence scope: web_and_workspace. New web \
        retrieval would normally be permitted by this scope.";
    const REPORT_TARGET: &str = "The host will persist `.a3s/research/test/report.md` and \
        render `.a3s/research/test/index.html`; do not write files or print a marker.";

    fn assert_closed_report_phase(prompt: &str) {
        assert!(
            prompt.contains("Evidence collection is closed."),
            "{prompt}"
        );
        assert!(
            prompt.contains(
                "Closed-evidence report phase (mandatory and higher priority than \
                 evidence-scope wording)"
            ),
            "{prompt}"
        );
        assert!(
            prompt.contains("Do not call or request search, fetch, batch, shell, Git, delegation"),
            "{prompt}"
        );
        for forbidden_tool in [
            "`web_search`",
            "`web_fetch`",
            "`batch`",
            "`bash`",
            "`git`",
            "`task`",
            "`parallel_task`",
            "`program`",
            "`dynamic_workflow`",
            "`runtime`",
        ] {
            assert!(
                prompt.contains(forbidden_tool),
                "{forbidden_tool}: {prompt}"
            );
        }
        assert!(
            prompt.contains("No tools are available in this phase"),
            "{prompt}"
        );
        assert!(
            prompt.contains("Return the complete report as Markdown"),
            "{prompt}"
        );
        assert!(
            prompt.contains("evidence-insufficient failure report"),
            "{prompt}"
        );
        assert!(prompt.contains("fail or degrade explicitly"), "{prompt}");
        assert!(prompt.contains("report-master"), "{prompt}");
        assert!(
            prompt.contains("lead with a one-sentence answer"),
            "{prompt}"
        );
        assert!(
            prompt.contains("pyramid, narrative, instructional, or briefing"),
            "{prompt}"
        );
        assert!(
            prompt.contains("Depth means explaining why, how, comparisons, change, implications"),
            "{prompt}"
        );
        assert!(
            !prompt.contains("at least three evidence-appropriate"),
            "{prompt}"
        );
        assert!(prompt.contains("report.md"), "{prompt}");
        assert!(prompt.contains("index.html"), "{prompt}");
        assert!(prompt.contains("host-owned renderer"), "{prompt}");
        assert!(!prompt.contains("expected_offset"), "{prompt}");

        let scope_index = prompt.find(CONFLICTING_SCOPE).expect("evidence scope");
        let closed_index = prompt
            .find("Closed-evidence report phase")
            .expect("closed-evidence contract");
        assert!(
            closed_index > scope_index,
            "closed-evidence contract must override the earlier scope wording: {prompt}"
        );
    }

    #[test]
    fn synthesis_is_a_closed_evidence_report_phase() {
        let prompt = synthesis_prompt(SynthesisPrompt {
            query: "test query",
            os_runtime: false,
            workflow_digest: r#"{"collection_status":"degraded","evidence_items":[]}"#,
            metadata: r#"{"warnings":["collection incomplete"]}"#,
            report_target: REPORT_TARGET,
            evidence_scope: CONFLICTING_SCOPE,
        });

        assert_closed_report_phase(&prompt);
        assert!(prompt.contains("Use only the Evidence digest"), "{prompt}");
        assert!(
            !prompt.contains("use additional tools or skills"),
            "{prompt}"
        );
        assert!(!prompt.contains("use targeted recovery tools"), "{prompt}");
        assert!(!prompt.contains("Tool-loop guard"), "{prompt}");
    }

    #[test]
    fn recovery_recovers_the_report_without_recovering_evidence() {
        let prompt = recovery_prompt(RecoveryPrompt {
            query: "test query",
            os_runtime: true,
            workflow_error: "collector timed out",
            metadata: r#"{"collection_status":"failed"}"#,
            report_target: REPORT_TARGET,
            evidence_scope: CONFLICTING_SCOPE,
        });

        assert_closed_report_phase(&prompt);
        assert!(prompt.contains("do not recover evidence"), "{prompt}");
        assert!(
            prompt.contains("Deliver a complete Markdown report"),
            "{prompt}"
        );
        assert!(
            !prompt.contains("Use tools and skills as needed for recovery"),
            "{prompt}"
        );
        assert!(
            !prompt.contains("preferring targeted collection"),
            "{prompt}"
        );
        assert!(!prompt.contains("Tool-loop guard"), "{prompt}");
    }

    #[test]
    fn repair_only_materializes_the_supplied_report_package() {
        let prompt = repair_prompt(RepairPrompt {
            query: "test query",
            os_runtime: false,
            workflow_digest: r#"{"collection_status":"completed","evidence_items":[]}"#,
            metadata: "{}",
            prior: "Prior synthesis without artifacts.",
            report_target: REPORT_TARGET,
            evidence_scope: CONFLICTING_SCOPE,
        });

        assert_closed_report_phase(&prompt);
        assert!(
            prompt.contains("using only the gathered evidence, diagnostics, and prior synthesis"),
            "{prompt}"
        );
        assert!(
            !prompt.contains("Use tools and skills as needed to create"),
            "{prompt}"
        );
        assert!(!prompt.contains("Prefer targeted checks"), "{prompt}");
        assert!(!prompt.contains("Tool-loop guard"), "{prompt}");
    }
}
