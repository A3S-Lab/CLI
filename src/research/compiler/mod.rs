mod api;
mod catalog;
mod contract;
mod coverage;
mod document;
mod ledger;
mod render;

pub use api::{
    compile_evidence_report, evidence_source_content_digest, evidence_spec_digest,
    validate_evidence_catalog, validate_evidence_contract, CompiledEvidenceReport,
    CompilerCoverage, CompilerRejection, EvidenceCompilerError, EvidenceCompilerOutcome,
};

use catalog::*;
use contract::*;
use coverage::*;
use document::*;
use ledger::*;
use render::*;

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod frozen_fixture;
#[cfg(test)]
mod frozen_replay_tests;
#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod outcome_tests;
#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
