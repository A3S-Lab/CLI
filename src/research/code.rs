use a3s_code_core::AgentEvent;
use a3s_deep_research::engine::DeepResearchEvent;

#[path = "journal.rs"]
mod journal;
#[path = "runner.rs"]
mod runner;
#[path = "runtime.rs"]
mod runtime;
#[path = "workflow_source.rs"]
mod workflow_source;

pub(crate) use journal::{
    load_latest_code_deep_research_journal, read_code_deep_research_journal,
    settle_interrupted_code_deep_research_journal, CodeDeepResearchJournalSnapshot,
};
#[cfg(test)]
pub(crate) use runner::build_isolated_research_session_with_resolver;
pub(crate) use runner::{
    build_code_deep_research_request, CodeDeepResearchLaunch, CodeDeepResearchRunExit,
    CodeDeepResearchRunHandle, CodeDeepResearchRunner, CodeDeepResearchRunnerBudget,
};
#[cfg(test)]
pub(crate) use runtime::validate_dynamic_workflow_arguments;
pub(crate) use workflow_source::{
    apply_patched_retrieval_workflow_source, code_deep_research_workflow_arguments,
};
#[cfg(test)]
pub(crate) use workflow_source::patched_retrieval_workflow_source;

#[derive(Debug)]
pub(crate) enum CodeDeepResearchEvent {
    Engine(DeepResearchEvent),
    Agent(AgentEvent),
}
