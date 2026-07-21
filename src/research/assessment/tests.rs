#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::{
        research_contract_outcome, CompletionCriterionAssessment, EvidenceDiagnosticKind,
        EvidenceQualityRequirements, EvidenceRef, InquiryLimits, InquiryPhase, Question,
        ResearchContractOutcome, ResearchMethod, SourceCoverageBinding, SourceEvidenceRole,
        StopConditionAssessment,
    };

    fn assessed_state() -> InquiryState {
        let limits = InquiryLimits::default();
        let obligation = ResearchObligation::new(
            "obligation:core",
            "Core finding",
            "Resolve the core finding",
            true,
            vec!["The finding is supported by traceable evidence".to_string()],
        );
        let mut state = InquiryState::default();
        for event in [
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::ResearchObligationsCommitted {
                obligations: vec![obligation],
                stop_conditions: vec!["The core finding is traceable".to_string()],
            },
        ] {
            state.apply(&event, &limits).expect("contract prefix");
        }
        let mut question = Question::queued("question:core", None, "What is supported?");
        question.obligation_ids = vec!["obligation:core".to_string()];
        state
            .apply(
                &InquiryEvent::QuestionsQueued {
                    questions: vec![question],
                },
                &limits,
            )
            .expect("question");
        state
            .apply(
                &InquiryEvent::EvidenceAccepted {
                    evidence: EvidenceRef::new(
                        "evidence:core",
                        vec!["claim:core".to_string()],
                        vec!["source:core".to_string()],
                    )
                    .with_diagnostics(vec![EvidenceDiagnostic::new(
                        "diagnostic:gap",
                        EvidenceDiagnosticKind::Gap,
                        "Independent corroboration remains unavailable",
                    )]),
                },
                &limits,
            )
            .expect("evidence");
        state
            .apply(
                &InquiryEvent::EvidenceAccepted {
                    evidence: EvidenceRef::new(
                        "evidence:resolution",
                        vec!["claim:resolution".to_string()],
                        vec!["source:resolution".to_string()],
                    ),
                },
                &limits,
            )
            .expect("resolution evidence");
        state
            .apply(
                &InquiryEvent::EvidenceAccepted {
                    evidence: EvidenceRef::new(
                        "evidence:unrelated",
                        vec!["claim:unrelated".to_string()],
                        vec!["source:unrelated".to_string()],
                    ),
                },
                &limits,
            )
            .expect("unrelated evidence");
        state
            .apply(
                &InquiryEvent::QuestionAnswered {
                    question_id: "question:core".to_string(),
                    answer: "The accepted evidence supports the core finding.".to_string(),
                    evidence_ids: vec![
                        "evidence:core".to_string(),
                        "evidence:resolution".to_string(),
                    ],
                },
                &limits,
            )
            .expect("answer");
        state
    }

    fn assessment(
        disposition: DiagnosticDisposition,
        diagnostic_evidence_ids: &[&str],
    ) -> ResearchContractAssessment {
        ResearchContractAssessment {
            obligations: vec![ResearchObligationAssessment {
                obligation_id: "obligation:core".to_string(),
                criteria: vec![CompletionCriterionAssessment {
                    criterion_index: 0,
                    status: ContractAssessmentStatus::Satisfied,
                    rationale: "The accepted claim and source support the criterion.".to_string(),
                    evidence_ids: vec!["evidence:core".to_string()],
                }],
                primary_source: None,
                independent_corroboration: None,
            }],
            stop_conditions: vec![StopConditionAssessment {
                condition_index: 0,
                status: ContractAssessmentStatus::Satisfied,
                rationale: "The finding is traceable.".to_string(),
                evidence_ids: vec!["evidence:core".to_string()],
            }],
            diagnostics: vec![EvidenceDiagnosticAssessment {
                diagnostic_id: "diagnostic:gap".to_string(),
                disposition,
                obligation_ids: vec!["obligation:core".to_string()],
                rationale: "The retained evidence explicitly bounds the gap.".to_string(),
                evidence_ids: diagnostic_evidence_ids
                    .iter()
                    .map(|id| (*id).to_string())
                    .collect(),
            }],
        }
    }

    fn quality_state() -> InquiryState {
        let limits = InquiryLimits::default();
        let obligation = ResearchObligation::new(
            "obligation:quality",
            "Evidence quality",
            "Establish the finding under its declared source-quality contract",
            true,
            vec!["The finding is directly supported".to_string()],
        )
        .with_evidence_requirements(EvidenceQualityRequirements {
            primary_source_required: true,
            independent_corroboration_required: true,
        });
        let mut state = InquiryState::default();
        for event in [
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::ResearchObligationsCommitted {
                obligations: vec![obligation],
                stop_conditions: vec!["The evidence contract is closed".to_string()],
            },
        ] {
            state.apply(&event, &limits).expect("quality contract");
        }
        let mut question = Question::queued(
            "question:quality",
            None,
            "Which retained evidence closes the contract?",
        );
        question.obligation_ids = vec!["obligation:quality".to_string()];
        state
            .apply(
                &InquiryEvent::QuestionsQueued {
                    questions: vec![question],
                },
                &limits,
            )
            .expect("quality question");
        for evidence in [
            EvidenceRef::new(
                "evidence:primary",
                vec!["claim:primary".to_string()],
                vec!["source:primary".to_string()],
            )
            .with_source_coverage(vec![SourceCoverageBinding::new(
                "source:primary",
                "obligation:quality",
                vec![0],
                vec![
                    SourceEvidenceRole::Supporting,
                    SourceEvidenceRole::Primary,
                    SourceEvidenceRole::Independent,
                ],
            )]),
            EvidenceRef::new(
                "evidence:corroborating",
                vec!["claim:corroborating".to_string()],
                vec!["source:corroborating".to_string()],
            )
            .with_source_coverage(vec![SourceCoverageBinding::new(
                "source:corroborating",
                "obligation:quality",
                vec![0],
                vec![
                    SourceEvidenceRole::Supporting,
                    SourceEvidenceRole::Independent,
                ],
            )]),
        ] {
            state
                .apply(&InquiryEvent::EvidenceAccepted { evidence }, &limits)
                .expect("quality evidence");
        }
        state
            .apply(
                &InquiryEvent::QuestionAnswered {
                    question_id: "question:quality".to_string(),
                    answer: "The primary record and separately attributable corroboration support the finding."
                        .to_string(),
                    evidence_ids: vec![
                        "evidence:primary".to_string(),
                        "evidence:corroborating".to_string(),
                    ],
                },
                &limits,
            )
            .expect("quality answer");
        state
    }

    fn quality_assessment() -> ResearchContractAssessment {
        ResearchContractAssessment {
            obligations: vec![ResearchObligationAssessment {
                obligation_id: "obligation:quality".to_string(),
                criteria: vec![CompletionCriterionAssessment {
                    criterion_index: 0,
                    status: ContractAssessmentStatus::Satisfied,
                    rationale: "The retained evidence directly supports the finding.".to_string(),
                    evidence_ids: vec!["evidence:primary".to_string()],
                }],
                primary_source: Some(EvidenceRequirementAssessment {
                    status: ContractAssessmentStatus::Satisfied,
                    rationale: "The cited source is the direct original record.".to_string(),
                    evidence_ids: vec!["evidence:primary".to_string()],
                    source_ids: vec!["source:primary".to_string()],
                }),
                independent_corroboration: Some(EvidenceRequirementAssessment {
                    status: ContractAssessmentStatus::Satisfied,
                    rationale: "Two separately attributable sources corroborate the finding."
                        .to_string(),
                    evidence_ids: vec![
                        "evidence:primary".to_string(),
                        "evidence:corroborating".to_string(),
                    ],
                    source_ids: vec![
                        "source:primary".to_string(),
                        "source:corroborating".to_string(),
                    ],
                }),
            }],
            stop_conditions: vec![StopConditionAssessment {
                condition_index: 0,
                status: ContractAssessmentStatus::Satisfied,
                rationale: "The declared evidence contract is closed.".to_string(),
                evidence_ids: vec![
                    "evidence:primary".to_string(),
                    "evidence:corroborating".to_string(),
                ],
            }],
            diagnostics: Vec::new(),
        }
    }

    fn generation_wire(assessment: &ResearchContractAssessment) -> serde_json::Value {
        let obligations = assessment
            .obligations
            .iter()
            .map(|obligation| {
                let mut value = serde_json::to_value(obligation).expect("obligation value");
                let criteria = obligation
                    .criteria
                    .iter()
                    .map(|criterion| {
                        (
                            criterion.criterion_index.to_string(),
                            serde_json::to_value(criterion).expect("criterion value"),
                        )
                    })
                    .collect::<serde_json::Map<_, _>>();
                value["criteria"] = serde_json::Value::Object(criteria);
                (obligation.obligation_id.clone(), value)
            })
            .collect::<serde_json::Map<_, _>>();
        let stop_conditions = assessment
            .stop_conditions
            .iter()
            .map(|condition| {
                (
                    condition.condition_index.to_string(),
                    serde_json::to_value(condition).expect("stop-condition value"),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let diagnostics = assessment
            .diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.diagnostic_id.clone(),
                    serde_json::to_value(diagnostic).expect("diagnostic value"),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        serde_json::json!({
            "obligations": obligations,
            "stop_conditions": stop_conditions,
            "diagnostics": diagnostics,
        })
    }

    fn large_assessment_state(
        evidence_count: usize,
        diagnostics_per_evidence: usize,
    ) -> InquiryState {
        let limits = InquiryLimits {
            max_events: evidence_count
                .saturating_mul(diagnostics_per_evidence)
                .saturating_add(32),
            max_evidence_ids_per_answer: evidence_count,
            max_citation_ids_per_section: diagnostics_per_evidence.max(evidence_count),
            ..InquiryLimits::default()
        };
        let obligation = ResearchObligation::new(
            "obligation:large",
            "Large contract",
            "Assess a large closed evidence graph",
            true,
            vec!["The retained evidence supports the finding".to_string()],
        );
        let mut state = InquiryState::default();
        for event in [
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::ResearchObligationsCommitted {
                obligations: vec![obligation],
                stop_conditions: vec!["Every retained finding is traceable".to_string()],
            },
        ] {
            state.apply(&event, &limits).expect("large contract prefix");
        }
        let mut question = Question::queued("question:large", None, "What does the evidence show?");
        question.obligation_ids = vec!["obligation:large".to_string()];
        state
            .apply(
                &InquiryEvent::QuestionsQueued {
                    questions: vec![question],
                },
                &limits,
            )
            .expect("large question");
        let mut evidence_ids = Vec::new();
        for evidence_index in 0..evidence_count {
            let evidence_id = format!("evidence:{evidence_index}");
            let diagnostics = (0..diagnostics_per_evidence)
                .map(|diagnostic_index| {
                    EvidenceDiagnostic::new(
                        format!("diagnostic:{evidence_index}:{diagnostic_index}"),
                        EvidenceDiagnosticKind::Gap,
                        format!("Bounded evidence gap {diagnostic_index} on item {evidence_index}"),
                    )
                })
                .collect();
            state
                .apply(
                    &InquiryEvent::EvidenceAccepted {
                        evidence: EvidenceRef::new(
                            &evidence_id,
                            vec![format!("claim:{evidence_index}")],
                            vec![format!("source:{evidence_index}")],
                        )
                        .with_diagnostics(diagnostics),
                    },
                    &limits,
                )
                .expect("large evidence");
            evidence_ids.push(evidence_id);
        }
        state
            .apply(
                &InquiryEvent::QuestionAnswered {
                    question_id: "question:large".to_string(),
                    answer: "The retained evidence closes the large question path.".to_string(),
                    evidence_ids,
                },
                &limits,
            )
            .expect("large answer");
        state
    }

    fn large_assessment(state: &InquiryState) -> ResearchContractAssessment {
        ResearchContractAssessment {
            obligations: vec![ResearchObligationAssessment {
                obligation_id: "obligation:large".to_string(),
                criteria: vec![CompletionCriterionAssessment {
                    criterion_index: 0,
                    status: ContractAssessmentStatus::Satisfied,
                    rationale: "The retained evidence supports the finding.".to_string(),
                    evidence_ids: vec!["evidence:0".to_string()],
                }],
                primary_source: None,
                independent_corroboration: None,
            }],
            stop_conditions: vec![StopConditionAssessment {
                condition_index: 0,
                status: ContractAssessmentStatus::Satisfied,
                rationale: "The finding is traceable.".to_string(),
                evidence_ids: vec!["evidence:0".to_string()],
            }],
            diagnostics: evidence_diagnostic_catalog(state)
                .into_iter()
                .map(
                    |(diagnostic, parent_evidence_id)| EvidenceDiagnosticAssessment {
                        diagnostic_id: diagnostic.id.clone(),
                        disposition: DiagnosticDisposition::Bounded,
                        obligation_ids: vec!["obligation:large".to_string()],
                        rationale: "The diagnostic remains explicitly bounded.".to_string(),
                        evidence_ids: vec![parent_evidence_id.clone()],
                    },
                )
                .collect(),
        }
    }

    fn indexed_generation_wire_for_chunk(
        state: &InquiryState,
        chunk: &ResearchContractAssessmentGenerationChunk,
    ) -> serde_json::Value {
        let diagnostic_catalog = evidence_diagnostic_catalog(state);
        let mut obligations = serde_json::Map::new();
        let mut stop_conditions = serde_json::Map::new();
        let mut diagnostics = serde_json::Map::new();
        for unit in &chunk.scope.units {
            match *unit {
                AssessmentUnit::Obligation(_) => {
                    let references = chunk
                        .reference_catalog
                        .obligations
                        .get("obligation:large")
                        .expect("large obligation references");
                    let evidence_index = references
                        .evidence_ids
                        .iter()
                        .position(|id| id == "evidence:0")
                        .expect("large evidence reference");
                    obligations.insert(
                        "obligation:large".to_string(),
                        serde_json::json!({
                            "criteria": {
                                "0": {
                                    "status": "satisfied",
                                    "rationale": "The retained evidence supports the finding.",
                                    "evidence_ids": [evidence_index]
                                }
                            }
                        }),
                    );
                }
                AssessmentUnit::StopCondition(index) => {
                    let evidence_index = chunk
                        .reference_catalog
                        .stop_condition_evidence_ids
                        .iter()
                        .position(|id| id == "evidence:0")
                        .expect("large stop evidence reference");
                    stop_conditions.insert(
                        index.to_string(),
                        serde_json::json!({
                            "status": "satisfied",
                            "rationale": "The finding is traceable.",
                            "evidence_ids": [evidence_index]
                        }),
                    );
                }
                AssessmentUnit::Diagnostic(index) => {
                    let diagnostic_id = diagnostic_catalog[index].0.id.clone();
                    diagnostics.insert(
                        diagnostic_id,
                        serde_json::json!({
                            "disposition": "bounded",
                            "obligation_ids": [0],
                            "rationale": "The diagnostic remains explicitly bounded.",
                            "evidence_ids": [0]
                        }),
                    );
                }
            }
        }
        serde_json::json!({
            "obligations": obligations,
            "stop_conditions": stop_conditions,
            "diagnostics": diagnostics,
        })
    }

    #[test]
    fn keyed_generation_wire_round_trips_empty_and_multiple_diagnostics() {
        let empty = quality_assessment();
        assert_eq!(
            decode_research_contract_assessment(generation_wire(&empty)).expect("empty wire"),
            empty
        );

        let mut multiple = assessment(DiagnosticDisposition::Bounded, &["evidence:core"]);
        multiple.diagnostics.push(EvidenceDiagnosticAssessment {
            diagnostic_id: "diagnostic:second".to_string(),
            disposition: DiagnosticDisposition::Irrelevant,
            obligation_ids: Vec::new(),
            rationale: "The second diagnostic is outside the contract.".to_string(),
            evidence_ids: Vec::new(),
        });
        assert_eq!(
            decode_research_contract_assessment(generation_wire(&multiple))
                .expect("multiple diagnostic wire"),
            multiple
        );
    }

    #[test]
    fn keyed_generation_wire_rejects_every_key_discriminator_mismatch() {
        let value = generation_wire(&assessment(
            DiagnosticDisposition::Bounded,
            &["evidence:core"],
        ));
        for (field, old_key, new_key, expected) in [
            (
                "obligations",
                "obligation:core",
                "obligation:wrong",
                "obligation_id",
            ),
            ("stop_conditions", "0", "1", "condition_index"),
            (
                "diagnostics",
                "diagnostic:gap",
                "diagnostic:wrong",
                "diagnostic_id",
            ),
        ] {
            let mut mismatched = value.clone();
            let entries = mismatched[field].as_object_mut().expect("keyed entries");
            let entry = entries.remove(old_key).expect("original keyed entry");
            entries.insert(new_key.to_string(), entry);
            let error = decode_research_contract_assessment(mismatched)
                .expect_err("key/discriminator mismatch must fail");
            assert!(error.message().contains(expected), "{error}");
        }

        let mut criterion = value;
        let criteria = criterion["obligations"]["obligation:core"]["criteria"]
            .as_object_mut()
            .expect("keyed criteria");
        let entry = criteria.remove("0").expect("criterion zero");
        criteria.insert("1".to_string(), entry);
        let error = decode_research_contract_assessment(criterion)
            .expect_err("criterion key/index mismatch must fail");
        assert!(error.message().contains("criterion_index"));
    }

    #[test]
    fn small_contract_keeps_the_exact_single_chunk_schema() {
        let state = assessed_state();
        let chunks = research_contract_assessment_generation_chunks(
            "Assess the core finding",
            &state,
            "closed evidence packet",
            30_000,
        )
        .expect("small assessment chunks");
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].reference_encoding,
            AssessmentReferenceEncoding::ExactIds
        );
        assert_eq!(
            chunks[0].params.schema,
            research_contract_assessment_json_schema(&state).expect("exact schema")
        );
        assert!(
            serialized_schema_size(&chunks[0].params.schema).expect("schema size")
                <= RESEARCH_CONTRACT_ASSESSMENT_SCHEMA_BUDGET_BYTES
        );
    }

    #[test]
    fn maximum_diagnostic_contract_uses_bounded_exact_chunks_and_aggregates() {
        let state = large_assessment_state(64, 32);
        let expected = large_assessment(&state);
        let chunks = research_contract_assessment_generation_chunks(
            "Assess the large finding",
            &state,
            "closed evidence packet",
            30_000,
        )
        .expect("large assessment chunks");
        assert!(
            chunks.len() > 1,
            "the oversized exact schema must be chunked"
        );
        assert!(chunks.iter().all(|chunk| {
            chunk.reference_encoding == AssessmentReferenceEncoding::Indexed
                && serialized_schema_size(&chunk.params.schema)
                    .is_ok_and(|size| size <= RESEARCH_CONTRACT_ASSESSMENT_SCHEMA_BUDGET_BYTES)
        }));

        let mut parts = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let wire = indexed_generation_wire_for_chunk(&state, chunk);
            parts.push(
                decode_research_contract_assessment_chunk(&state, chunk, wire)
                    .expect("indexed assessment chunk"),
            );
        }
        let actual = aggregate_research_contract_assessments(&state, parts)
            .expect("complete aggregate assessment");
        assert_eq!(actual, expected);
    }

    #[test]
    fn indexed_references_remain_scoped_to_each_traceable_path() {
        let state = assessed_state();
        let scope = full_assessment_scope(&state);
        let references = assessment_reference_catalog(&state, &scope);
        assert_eq!(
            references.obligations["obligation:core"].evidence_ids,
            ["evidence:core", "evidence:resolution"],
            "unrelated evidence must not enter an obligation reference table"
        );
        assert_eq!(
            references.diagnostics["diagnostic:gap"].resolved_evidence_ids,
            ["evidence:resolution"],
            "only distinct evidence on the parent obligation path may resolve a diagnostic"
        );
        assert_eq!(
            references.diagnostics["diagnostic:gap"].bounded_evidence_ids,
            ["evidence:core"],
            "bounded disposition must use only the parent evidence"
        );

        let schema = indexed_assessment_schema(&state, &scope, &references)
            .expect("path-scoped indexed schema");
        let obligation_status = &schema["$defs"]["obligation_0_status"]["oneOf"];
        let satisfied = obligation_status
            .as_array()
            .and_then(|variants| {
                variants
                    .iter()
                    .find(|variant| variant["properties"]["status"]["enum"][0] == "satisfied")
            })
            .expect("satisfied obligation status");
        assert_eq!(
            satisfied["properties"]["evidence_ids"]["items"]["maximum"],
            1
        );
    }

    #[test]
    fn aggregate_rejects_missing_duplicate_and_unknown_assessment_identities() {
        let state = assessed_state();
        let valid = assessment(DiagnosticDisposition::Bounded, &["evidence:core"]);

        let mut missing = valid.clone();
        missing.diagnostics.clear();
        assert!(aggregate_research_contract_assessments(&state, vec![missing]).is_err());

        let mut duplicate = valid.clone();
        duplicate.diagnostics.push(duplicate.diagnostics[0].clone());
        assert!(aggregate_research_contract_assessments(&state, vec![duplicate]).is_err());

        let mut unknown = valid;
        unknown.diagnostics[0].diagnostic_id = "diagnostic:unknown".to_string();
        let error = aggregate_research_contract_assessments(&state, vec![unknown])
            .expect_err("unknown diagnostic identity must fail");
        assert!(error.message().contains("unknown evidence diagnostic"));
    }

    #[test]
    fn schema_is_closed_over_contract_evidence_and_diagnostics() {
        let state = assessed_state();
        let schema = research_contract_assessment_json_schema(&state).expect("schema");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["obligations"]["type"], "object");
        assert_eq!(
            schema["properties"]["obligations"]["required"],
            serde_json::json!(["obligation:core"])
        );
        assert_eq!(
            schema["properties"]["stop_conditions"]["required"],
            serde_json::json!(["0"])
        );
        assert_eq!(
            schema["properties"]["diagnostics"]["required"],
            serde_json::json!(["diagnostic:gap"])
        );
        assert!(schema.to_string().contains("obligation:core"));
        assert!(schema.to_string().contains("evidence:core"));
        assert!(schema.to_string().contains("diagnostic:gap"));
    }

    #[test]
    fn schema_requires_only_planner_declared_evidence_quality_assessments() {
        let state = quality_state();
        let params = research_contract_assessment_generation_params(
            "Assess the finding",
            &state,
            "closed evidence packet",
            30_000,
        )
        .expect("quality assessment params");
        let obligation =
            &params.schema["properties"]["obligations"]["properties"]["obligation:quality"];
        let required = obligation["required"].as_array().expect("required fields");
        assert!(required.contains(&serde_json::json!("primary_source")));
        assert!(required.contains(&serde_json::json!("independent_corroboration")));
        assert_eq!(
            obligation["properties"]["independent_corroboration"]["oneOf"][0]["properties"]
                ["source_ids"]["minItems"],
            2
        );
        assert!(params.prompt.contains("separately attributable source IDs"));

        let legacy = assessed_state();
        let schema = research_contract_assessment_json_schema(&legacy).expect("legacy schema");
        let obligation = &schema["properties"]["obligations"]["properties"]["obligation:core"];
        assert!(obligation["properties"].get("primary_source").is_none());
        assert!(obligation["properties"]
            .get("independent_corroboration")
            .is_none());
    }

    #[test]
    fn declared_evidence_quality_closes_only_with_traceable_source_roles() {
        let mut state = quality_state();
        let value = quality_assessment();
        validate_research_contract_assessment(&state, &value).expect("valid quality contract");
        state.contract_assessment = Some(value);
        assert_eq!(
            research_contract_outcome(&state),
            Some(ResearchContractOutcome::Satisfied)
        );
    }

    #[test]
    fn one_source_cannot_fake_independent_corroboration() {
        let state = quality_state();
        let mut value = quality_assessment();
        value.obligations[0]
            .independent_corroboration
            .as_mut()
            .expect("corroboration")
            .source_ids = vec!["source:primary".to_string()];
        let error = validate_research_contract_assessment(&state, &value)
            .expect_err("one source must not satisfy independent corroboration");
        assert!(error.message().contains("at least 2 distinct"));
    }

    #[test]
    fn host_requires_every_planner_declared_quality_assessment() {
        let state = quality_state();
        let mut value = quality_assessment();
        value.obligations[0].primary_source = None;
        let error = validate_research_contract_assessment(&state, &value)
            .expect_err("declared primary-source requirement cannot disappear");
        assert!(error
            .message()
            .contains("omitted its declared primary source"));

        let legacy_state = assessed_state();
        let mut legacy_assessment = assessment(DiagnosticDisposition::Bounded, &["evidence:core"]);
        legacy_assessment.obligations[0].primary_source = Some(EvidenceRequirementAssessment {
            status: ContractAssessmentStatus::Satisfied,
            rationale: "An undeclared requirement must not be injected.".to_string(),
            evidence_ids: vec!["evidence:core".to_string()],
            source_ids: vec!["source:core".to_string()],
        });
        let error = validate_research_contract_assessment(&legacy_state, &legacy_assessment)
            .expect_err("assessment cannot add an undeclared quality gate");
        assert!(error
            .message()
            .contains("assessed undeclared primary source"));
    }

    #[test]
    fn evidence_requirement_source_must_belong_to_its_cited_evidence() {
        let state = quality_state();
        let mut value = quality_assessment();
        let primary = value.obligations[0]
            .primary_source
            .as_mut()
            .expect("primary source");
        primary.evidence_ids = vec!["evidence:corroborating".to_string()];
        let error = validate_research_contract_assessment(&state, &value)
            .expect_err("source/evidence relationship must stay closed");
        assert!(error
            .message()
            .contains("does not belong to its cited evidence"));
    }

    #[test]
    fn bounded_declared_quality_prevents_material_false_completion() {
        let mut state = quality_state();
        let mut value = quality_assessment();
        value.obligations[0]
            .independent_corroboration
            .as_mut()
            .expect("corroboration")
            .status = ContractAssessmentStatus::Bounded;
        value.obligations[0]
            .independent_corroboration
            .as_mut()
            .expect("corroboration")
            .source_ids = vec!["source:primary".to_string()];
        validate_research_contract_assessment(&state, &value).expect("bounded quality assessment");
        state.contract_assessment = Some(value);
        assert_eq!(
            research_contract_outcome(&state),
            Some(ResearchContractOutcome::Qualified)
        );
    }

    #[test]
    fn legacy_contract_json_defaults_to_no_extra_evidence_requirement() {
        let obligation_value = serde_json::json!({
            "id": "obligation:legacy",
            "title": "Legacy obligation",
            "focus": "Replay an old journal",
            "material": true,
            "completion_criteria": ["The old criterion is supported"]
        });
        let obligation: ResearchObligation =
            serde_json::from_value(obligation_value.clone()).expect("legacy obligation");
        assert_eq!(
            obligation.evidence_requirements,
            EvidenceQualityRequirements::default()
        );
        assert!(serde_json::to_value(&obligation)
            .expect("serialize legacy obligation")
            .get("evidence_requirements")
            .is_none());
        let legacy_event_value = serde_json::json!({
            "type": "research_obligations_committed",
            "obligations": [obligation_value],
            "stop_conditions": ["The old criterion is supported"]
        });
        let legacy_event: InquiryEvent =
            serde_json::from_value(legacy_event_value.clone()).expect("legacy event");
        assert_eq!(
            serde_json::to_value(legacy_event).expect("re-encode legacy event"),
            legacy_event_value,
            "default evidence requirements must not change a legacy event digest"
        );

        let assessment: ResearchObligationAssessment = serde_json::from_value(serde_json::json!({
            "obligation_id": "obligation:legacy",
            "criteria": [{
                "criterion_index": 0,
                "status": "satisfied",
                "rationale": "Legacy evidence is traceable.",
                "evidence_ids": ["evidence:legacy"]
            }]
        }))
        .expect("legacy assessment");
        assert!(assessment.primary_source.is_none());
        assert!(assessment.independent_corroboration.is_none());
    }

    #[test]
    fn schema_requires_irrelevant_diagnostic_links_to_be_empty() {
        let state = assessed_state();
        let schema = research_contract_assessment_json_schema(&state).expect("schema");
        let dispositions = schema["properties"]["diagnostics"]["properties"]["diagnostic:gap"]
            ["oneOf"]
            .as_array()
            .expect("disposition variants");
        let irrelevant = dispositions
            .iter()
            .find(|variant| variant["properties"]["disposition"]["enum"][0] == "irrelevant")
            .expect("irrelevant disposition schema");
        assert_eq!(irrelevant["properties"]["obligation_ids"]["maxItems"], 0);
        assert_eq!(irrelevant["properties"]["evidence_ids"]["maxItems"], 0);
    }

    #[test]
    fn diagnostic_schema_is_closed_over_its_parent_obligation_path() {
        let state = assessed_state();
        let schema = research_contract_assessment_json_schema(&state).expect("schema");
        let dispositions = schema["properties"]["diagnostics"]["properties"]["diagnostic:gap"]
            ["oneOf"]
            .as_array()
            .expect("disposition variants");
        let resolved = dispositions
            .iter()
            .find(|variant| variant["properties"]["disposition"]["enum"][0] == "resolved")
            .expect("resolved disposition schema");
        assert_eq!(
            resolved["properties"]["obligation_ids"]["items"]["enum"],
            serde_json::json!(["obligation:core"])
        );
        assert_eq!(resolved["properties"]["obligation_ids"]["minItems"], 1);
        assert_eq!(resolved["properties"]["obligation_ids"]["maxItems"], 1);
        assert_eq!(
            resolved["properties"]["evidence_ids"]["items"]["enum"],
            serde_json::json!(["evidence:resolution"]),
            "the parent and unrelated evidence must not be offered as resolvers"
        );

        let bounded = dispositions
            .iter()
            .find(|variant| variant["properties"]["disposition"]["enum"][0] == "bounded")
            .expect("bounded disposition schema");
        assert_eq!(
            bounded["properties"]["evidence_ids"]["items"]["enum"],
            serde_json::json!(["evidence:core"])
        );
        assert_eq!(bounded["properties"]["evidence_ids"]["minItems"], 1);
        assert_eq!(bounded["properties"]["evidence_ids"]["maxItems"], 1);
    }

    #[test]
    fn diagnostic_schema_omits_resolved_without_a_distinct_traceable_resolver() {
        let mut state = assessed_state();
        state.questions[0].evidence_ids = vec!["evidence:core".to_string()];
        let schema = research_contract_assessment_json_schema(&state).expect("schema");
        let dispositions = schema["properties"]["diagnostics"]["properties"]["diagnostic:gap"]
            ["oneOf"]
            .as_array()
            .expect("disposition variants");
        assert!(dispositions
            .iter()
            .all(|variant| { variant["properties"]["disposition"]["enum"][0] != "resolved" }));
    }

    #[test]
    fn bounded_material_diagnostic_prevents_false_convergence() {
        let mut state = assessed_state();
        let value = assessment(DiagnosticDisposition::Bounded, &["evidence:core"]);
        validate_research_contract_assessment(&state, &value).expect("valid assessment");
        state.contract_assessment = Some(value);
        assert_eq!(
            research_contract_outcome(&state),
            Some(ResearchContractOutcome::Qualified)
        );
    }

    #[test]
    fn malformed_irrelevant_links_are_conservatively_bounded_by_the_host() {
        let mut state = assessed_state();
        let value = assessment(DiagnosticDisposition::Irrelevant, &["evidence:core"]);
        validate_research_contract_assessment(&state, &value)
            .expect_err("the strict validator must reject contradictory irrelevant links");

        let event = research_contract_assessment_event(&state, value)
            .expect("the event boundary should repair the known model shape");
        let InquiryEvent::ResearchContractAssessed { assessment } = &event else {
            panic!("expected a research contract assessment event");
        };
        let diagnostic = &assessment.diagnostics[0];
        assert_eq!(diagnostic.disposition, DiagnosticDisposition::Bounded);
        assert_eq!(diagnostic.obligation_ids, ["obligation:core"]);
        assert_eq!(diagnostic.evidence_ids, ["evidence:core"]);

        state
            .apply(&event, &InquiryLimits::default())
            .expect("event");
        assert_eq!(
            research_contract_outcome(&state),
            Some(ResearchContractOutcome::Qualified)
        );
    }

    #[test]
    fn parent_evidence_cannot_resolve_its_own_diagnostic() {
        let state = assessed_state();
        let value = assessment(DiagnosticDisposition::Resolved, &["evidence:core"]);
        let error = validate_research_contract_assessment(&state, &value)
            .expect_err("parent evidence must not resolve its own diagnostic");
        assert!(error.message().contains("different traceable evidence"));
    }

    #[test]
    fn unrelated_evidence_cannot_resolve_a_diagnostic() {
        let state = assessed_state();
        let value = assessment(DiagnosticDisposition::Resolved, &["evidence:unrelated"]);
        let error = validate_research_contract_assessment(&state, &value)
            .expect_err("unrelated evidence must not resolve a diagnostic");
        assert!(error.message().contains("linked obligation path"));
    }

    #[test]
    fn distinct_traceable_evidence_allows_resolved_diagnostic() {
        let mut state = assessed_state();
        let value = assessment(DiagnosticDisposition::Resolved, &["evidence:resolution"]);
        validate_research_contract_assessment(&state, &value).expect("valid assessment");
        state.contract_assessment = Some(value);
        assert_eq!(
            research_contract_outcome(&state),
            Some(ResearchContractOutcome::Satisfied)
        );
    }

    #[test]
    fn host_derivation_preserves_source_quality_as_bounded_without_typed_roles() {
        let mut state = quality_state();
        for evidence in state.evidence_catalog.values_mut() {
            evidence.source_coverage.clear();
        }
        let assessment = derive_research_contract_assessment(&state).expect("derived assessment");
        let obligation = &assessment.obligations[0];
        assert_eq!(
            obligation.criteria[0].status,
            ContractAssessmentStatus::Satisfied
        );
        assert_eq!(
            obligation.primary_source.as_ref().unwrap().status,
            ContractAssessmentStatus::Bounded
        );
        assert_eq!(
            obligation
                .independent_corroboration
                .as_ref()
                .unwrap()
                .status,
            ContractAssessmentStatus::Bounded
        );
        assert!(obligation
            .primary_source
            .as_ref()
            .unwrap()
            .rationale
            .contains("will not infer"));

        let event =
            research_contract_assessment_event(&state, assessment).expect("assessment event");
        state
            .apply(&event, &InquiryLimits::default())
            .expect("apply assessment");
        assert_eq!(
            research_contract_outcome(&state),
            Some(ResearchContractOutcome::Qualified)
        );
    }

    #[test]
    fn host_derivation_satisfies_quality_from_typed_source_coverage() {
        let mut state = quality_state();
        let assessment = derive_research_contract_assessment(&state).expect("derived assessment");
        let obligation = &assessment.obligations[0];
        assert_eq!(
            obligation.primary_source.as_ref().unwrap().status,
            ContractAssessmentStatus::Satisfied
        );
        assert_eq!(
            obligation
                .independent_corroboration
                .as_ref()
                .unwrap()
                .status,
            ContractAssessmentStatus::Satisfied
        );
        assert_eq!(
            obligation
                .independent_corroboration
                .as_ref()
                .unwrap()
                .source_ids,
            ["source:corroborating", "source:primary"]
        );

        let event =
            research_contract_assessment_event(&state, assessment).expect("assessment event");
        state
            .apply(&event, &InquiryLimits::default())
            .expect("apply assessment");
        assert_eq!(
            research_contract_outcome(&state),
            Some(ResearchContractOutcome::Satisfied)
        );
    }

    #[test]
    fn one_typed_independent_source_remains_bounded() {
        let mut state = quality_state();
        state
            .evidence_catalog
            .get_mut("evidence:corroborating")
            .unwrap()
            .source_coverage
            .clear();

        let assessment = derive_research_contract_assessment(&state).expect("derived assessment");
        assert_eq!(
            assessment.obligations[0]
                .independent_corroboration
                .as_ref()
                .unwrap()
                .status,
            ContractAssessmentStatus::Bounded
        );
    }

    #[test]
    fn host_derivation_maps_each_question_to_its_typed_criterion_edge() {
        let limits = InquiryLimits::default();
        let obligation = ResearchObligation::new(
            "obligation:mapped",
            "Mapped coverage",
            "Exercise structural question-to-criterion coverage",
            true,
            vec![
                "The first criterion has direct evidence".to_string(),
                "The second criterion is explicitly bounded".to_string(),
            ],
        );
        let mut state = InquiryState::default();
        for event in [
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::ResearchObligationsCommitted {
                obligations: vec![obligation],
                stop_conditions: vec!["Every material edge is terminal".to_string()],
            },
        ] {
            state.apply(&event, &limits).expect("contract prefix");
        }
        let mut first = Question::queued("question:first", None, "Resolve criterion zero");
        first.obligation_ids = vec!["obligation:mapped".to_string()];
        first.completion_criterion_indexes = vec![0];
        let mut second = Question::queued("question:second", None, "Resolve criterion one");
        second.obligation_ids = vec!["obligation:mapped".to_string()];
        second.completion_criterion_indexes = vec![1];
        state
            .apply(
                &InquiryEvent::QuestionsQueued {
                    questions: vec![first, second],
                },
                &limits,
            )
            .expect("mapped questions");
        state
            .apply(
                &InquiryEvent::EvidenceAccepted {
                    evidence: EvidenceRef::new(
                        "evidence:mapped",
                        vec!["claim:mapped".to_string()],
                        vec!["source:mapped".to_string()],
                    ),
                },
                &limits,
            )
            .expect("mapped evidence");
        state
            .apply(
                &InquiryEvent::QuestionAnswered {
                    question_id: "question:first".to_string(),
                    answer: "The accepted evidence resolves criterion zero.".to_string(),
                    evidence_ids: vec!["evidence:mapped".to_string()],
                },
                &limits,
            )
            .expect("mapped answer");
        state
            .apply(
                &InquiryEvent::QuestionBounded {
                    question_id: "question:second".to_string(),
                    reason: "The closed packet does not resolve criterion one.".to_string(),
                },
                &limits,
            )
            .expect("mapped bound");

        let assessment = derive_research_contract_assessment(&state).expect("derived assessment");
        assert_eq!(
            assessment.obligations[0].criteria[0].status,
            ContractAssessmentStatus::Satisfied
        );
        assert_eq!(
            assessment.obligations[0].criteria[1].status,
            ContractAssessmentStatus::Bounded
        );
        assert_eq!(
            assessment.obligations[0].criteria[1].evidence_ids,
            ["evidence:mapped"]
        );
        assert_eq!(
            assessment.stop_conditions[0].status,
            ContractAssessmentStatus::Bounded
        );
    }

    #[test]
    fn partial_answer_retains_material_evidence_and_derives_qualified_contract() {
        let limits = InquiryLimits::default();
        let obligation = ResearchObligation::new(
            "obligation:partial",
            "Partially supported material finding",
            "Retain supported facts while bounding the missing comparison edge",
            true,
            vec!["The available comparison is traceable and its gap is explicit".to_string()],
        );
        let mut state = InquiryState::default();
        for event in [
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::ResearchObligationsCommitted {
                obligations: vec![obligation],
                stop_conditions: vec![
                    "The material finding is traceable or explicitly qualified".to_string()
                ],
            },
        ] {
            state.apply(&event, &limits).expect("partial prefix");
        }
        let mut question = Question::queued(
            "question:partial",
            None,
            "Which comparison facts are supported and what remains unknown?",
        );
        question.obligation_ids = vec!["obligation:partial".to_string()];
        state
            .apply(
                &InquiryEvent::QuestionsQueued {
                    questions: vec![question],
                },
                &limits,
            )
            .expect("partial question");
        state
            .apply(
                &InquiryEvent::EvidenceAccepted {
                    evidence: EvidenceRef::new(
                        "evidence:partial",
                        vec!["claim:partial".to_string()],
                        vec!["source:partial".to_string()],
                    ),
                },
                &limits,
            )
            .expect("partial evidence");
        state
            .apply(
                &InquiryEvent::QuestionPartiallyAnswered {
                    question_id: "question:partial".to_string(),
                    answer: "The retained evidence establishes the dominant supported path."
                        .to_string(),
                    limitation:
                        "The packet does not establish the remaining named compatibility cases."
                            .to_string(),
                    evidence_ids: vec!["evidence:partial".to_string()],
                },
                &limits,
            )
            .expect("partial answer");

        assert_eq!(state.phase, InquiryPhase::Outlining);
        assert_eq!(state.questions[0].status, QuestionStatus::Answered);
        assert_eq!(
            state.questions[0].bound_reason.as_deref(),
            Some("The packet does not establish the remaining named compatibility cases.")
        );
        assert_eq!(state.questions[0].evidence_ids, ["evidence:partial"]);
        assert!(material_evidence_floor(&state));

        let assessment = derive_research_contract_assessment(&state).expect("partial assessment");
        assert_eq!(
            assessment.obligations[0].criteria[0].status,
            ContractAssessmentStatus::Bounded
        );
        assert_eq!(
            assessment.obligations[0].criteria[0].evidence_ids,
            ["evidence:partial"]
        );
        assert_eq!(
            assessment.stop_conditions[0].status,
            ContractAssessmentStatus::Bounded
        );
        let event = research_contract_assessment_event(&state, assessment)
            .expect("partial assessment event");
        state
            .apply(&event, &limits)
            .expect("apply partial assessment");
        assert_eq!(
            research_contract_outcome(&state),
            Some(ResearchContractOutcome::Qualified)
        );
    }
}
