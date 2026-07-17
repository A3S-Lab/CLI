//! Reducer and replay failures.

use std::fmt;

use super::{InquiryPhase, QuestionStatus};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InquiryError {
    InvalidTransition {
        phase: InquiryPhase,
        event: &'static str,
    },
    HardLimitExceeded {
        resource: &'static str,
        limit: usize,
        actual: usize,
    },
    EmptyBatch {
        resource: &'static str,
    },
    EmptyValue {
        resource: &'static str,
    },
    DuplicateId {
        resource: &'static str,
        id: String,
    },
    ConflictingEvidence {
        id: String,
    },
    UnknownId {
        resource: &'static str,
        id: String,
    },
    InvalidQuestionState {
        id: String,
        status: QuestionStatus,
    },
    PerspectiveRequired {
        question_id: String,
    },
    UnresolvedQuestions {
        count: usize,
    },
    IncompleteSections {
        count: usize,
    },
    MissingSourceCitation {
        section_id: String,
    },
    InvalidResearchPlan {
        reason: String,
    },
    InvalidOutline {
        reason: String,
    },
}

impl fmt::Display for InquiryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { phase, event } => {
                write!(formatter, "event `{event}` is not valid during {phase:?}")
            }
            Self::HardLimitExceeded {
                resource,
                limit,
                actual,
            } => write!(
                formatter,
                "{resource} hard limit exceeded: {actual} > {limit}"
            ),
            Self::EmptyBatch { resource } => write!(formatter, "{resource} cannot be empty"),
            Self::EmptyValue { resource } => write!(formatter, "{resource} cannot be blank"),
            Self::DuplicateId { resource, id } => {
                write!(formatter, "duplicate {resource} id `{id}`")
            }
            Self::ConflictingEvidence { id } => {
                write!(
                    formatter,
                    "evidence id `{id}` conflicts with the accepted catalog"
                )
            }
            Self::UnknownId { resource, id } => write!(formatter, "unknown {resource} id `{id}`"),
            Self::InvalidQuestionState { id, status } => {
                write!(
                    formatter,
                    "question `{id}` cannot transition from {status:?}"
                )
            }
            Self::PerspectiveRequired { question_id } => {
                write!(
                    formatter,
                    "question `{question_id}` requires a committed perspective"
                )
            }
            Self::UnresolvedQuestions { count } => {
                write!(formatter, "{count} queued question(s) remain unresolved")
            }
            Self::IncompleteSections { count } => {
                write!(formatter, "{count} outline section(s) remain undrafted")
            }
            Self::MissingSourceCitation { section_id } => {
                write!(
                    formatter,
                    "section `{section_id}` must cite at least one source"
                )
            }
            Self::InvalidResearchPlan { reason } => {
                write!(formatter, "invalid research plan: {reason}")
            }
            Self::InvalidOutline { reason } => {
                write!(formatter, "invalid research outline: {reason}")
            }
        }
    }
}

impl std::error::Error for InquiryError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InquiryReplayError {
    pub event_index: usize,
    pub error: InquiryError,
}

impl fmt::Display for InquiryReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "event {} failed: {}",
            self.event_index, self.error
        )
    }
}

impl std::error::Error for InquiryReplayError {}
