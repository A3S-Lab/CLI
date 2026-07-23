use super::*;

#[test]
fn proposal_schema_keeps_model_output_block_only() {
    let schema = deep_research_report_proposal_schema();
    let properties = schema["properties"].as_object().expect("schema properties");
    assert_eq!(
        properties.keys().cloned().collect::<HashSet<_>>(),
        HashSet::from([
            "summary".to_string(),
            "findings".to_string(),
            "recommendations".to_string(),
            "limitations".to_string(),
        ])
    );
    assert!(schema.to_string().contains("source_aliases"));
    assert!(!schema.to_string().contains("markdown"));
    assert!(!schema.to_string().contains("url"));
}

#[test]
fn proposal_prompt_contains_no_catalog_anchor() {
    let mut catalog = proposal_catalog();
    catalog.sources[0]
        .chunks
        .push("Secondary release navigation without another finding.".to_string());
    let prompt = deep_research_report_proposal_prompt("Compare the records", &catalog)
        .expect("proposal prompt");
    assert!(prompt.contains("source-1"));
    assert!(prompt.contains("14 March 2026"));
    assert!(!prompt.contains("https://www.fifa.com"));
    assert!(!prompt.contains("https://www.olympics.com"));
    let packet = prompt
        .split_once("CLOSED_REPORT_PACKET=")
        .map(|(_, packet)| packet)
        .expect("closed report packet");
    let packet: serde_json::Value = serde_json::from_str(packet).expect("decode report packet");
    assert!(packet["sources"]
        .as_array()
        .expect("packet sources")
        .iter()
        .all(|source| source["excerpts"].as_array().map(Vec::len) == Some(1)));
}

#[test]
fn host_builds_fixed_sections_citations_and_ledger_from_valid_blocks() {
    let catalog = proposal_catalog();
    let proposal = serde_json::json!({
        "summary": [
            {
                "text": "The release record states 14 March 2026.",
                "source_aliases": ["source-1"]
            },
            {
                "text": "The deployment record states 18 March 2026.",
                "source_aliases": ["source-2"]
            }
        ],
        "findings": [
            {
                "text": "The release record states 14 March 2026.",
                "source_aliases": ["source-1"]
            },
            {
                "text": "The deployment record states 18 March 2026.",
                "source_aliases": ["source-2"]
            }
        ],
        "recommendations": [],
        "limitations": [{
            "text": "The records provide no explanation for the discrepancy.",
            "source_aliases": ["source-1", "source-2"]
        }]
    });

    let admitted = admit_deep_research_report_proposal("Compare the records", &catalog, proposal)
        .expect("admit proposal")
        .expect("admitted report");

    assert_eq!(admitted.accepted_block_count, 5);
    assert_eq!(admitted.rejected_block_count, 0);
    assert_eq!(admitted.direct_answer_block_count, 2);
    assert_eq!(admitted.finding_block_count, 2);
    assert_eq!(admitted.accepted_claim_count, 4);
    assert_eq!(admitted.cited_source_count, 2);
    assert!(admitted.markdown.contains("## Direct Answer"));
    assert!(admitted.markdown.contains("## Findings"));
    assert!(admitted.markdown.contains("## Limitations"));
    assert!(admitted
        .markdown
        .contains("[[1]](https://www.fifa.com/aurora)"));
    assert!(admitted
        .markdown
        .contains("[[2]](https://www.olympics.com/aurora)"));
    let ledger = admitted
        .markdown
        .split_once("## Sources")
        .map(|(_, ledger)| ledger)
        .expect("Host source ledger");
    assert_eq!(ledger.matches("Release record").count(), 1);
    assert_eq!(ledger.matches("Deployment record").count(), 1);
    assert!(!ledger.contains("1. [1]"), "{ledger}");
    assert!(!ledger.contains("2. [2]"), "{ledger}");
    assert!(!admitted.markdown.contains("source-1"));
}

#[test]
fn invalid_blocks_are_removed_without_losing_valid_siblings() {
    let catalog = proposal_catalog();
    let proposal = serde_json::json!({
        "summary": [{
            "text": "The release record states 14 March 2026.",
            "source_aliases": ["source-1"]
        }],
        "findings": [
            {
                "text": "A fabricated source says 20 March 2026.",
                "source_aliases": ["source-99"]
            },
            {
                "text": "Read more at https://invented.example.test.",
                "source_aliases": ["source-1"]
            },
            {
                "text": "The deployment record states 18 March 2026.",
                "source_aliases": ["source-2"]
            }
        ],
        "recommendations": [],
        "limitations": []
    });

    let admitted = admit_deep_research_report_proposal("Compare the records", &catalog, proposal)
        .expect("admit proposal")
        .expect("valid siblings survive");

    assert_eq!(admitted.accepted_block_count, 2);
    assert_eq!(admitted.rejected_block_count, 2);
    assert!(admitted.markdown.contains("18 March 2026"));
    assert!(!admitted.markdown.contains("20 March 2026"));
    assert!(!admitted.markdown.contains("invented.example.test"));
}

#[test]
fn unobserved_numeric_derivation_is_rejected() {
    let catalog = proposal_catalog();
    let proposal = serde_json::json!({
        "summary": [{
            "text": "The records differ by 4 days.",
            "source_aliases": ["source-1", "source-2"]
        }],
        "findings": [],
        "recommendations": [],
        "limitations": []
    });

    assert!(
        admit_deep_research_report_proposal("Compare the records", &catalog, proposal)
            .expect("admit proposal")
            .is_none()
    );
}

#[test]
fn a_publishable_report_requires_a_direct_answer_and_a_supported_finding() {
    let catalog = proposal_catalog();
    let summary_only = serde_json::json!({
        "summary": [{
            "text": "The release record states 14 March 2026.",
            "source_aliases": ["source-1"]
        }],
        "findings": [],
        "recommendations": [],
        "limitations": []
    });
    let limitations_only = serde_json::json!({
        "summary": [],
        "findings": [],
        "recommendations": [],
        "limitations": [{
            "text": "The records provide no explanation for the discrepancy.",
            "source_aliases": ["source-1", "source-2"]
        }]
    });

    assert!(
        admit_deep_research_report_proposal("Compare the records", &catalog, summary_only)
            .expect("decode summary-only proposal")
            .is_none()
    );
    assert!(
        admit_deep_research_report_proposal("Compare the records", &catalog, limitations_only)
            .expect("decode limitations-only proposal")
            .is_none()
    );
}

#[test]
fn low_confidence_sources_cannot_qualify_a_direct_answer() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![
            DeepResearchCatalogSource {
                alias: "source-1".to_string(),
                title: "FIFA Watch".to_string(),
                anchor: "https://fifawatch.example/world-cup".to_string(),
                chunks: vec![
                    "在 Skypiea.tv 免费观看，高清流畅，无需注册；页面声称西班牙 1-0 夺冠。"
                        .to_string(),
                ],
                claim_eligible: false,
            },
            DeepResearchCatalogSource {
                alias: "source-2".to_string(),
                title: "小红书世界杯集锦".to_string(),
                anchor: "https://www.xiaohongshu.com/worldcup/match".to_string(),
                chunks: vec!["用户发布的集锦声称西班牙 1-0 夺冠。".to_string()],
                claim_eligible: false,
            },
            DeepResearchCatalogSource {
                alias: "source-3".to_string(),
                title: "Olympics competition schedule".to_string(),
                anchor: "https://www.olympics.com/world-cup/schedule".to_string(),
                chunks: vec![
                    "The competition schedule runs from June through July 2026.".to_string()
                ],
                claim_eligible: true,
            },
        ],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    let prompt =
        deep_research_report_proposal_prompt("世界杯战况", &catalog).expect("closed report prompt");
    assert!(prompt.contains("\"claim_eligible\":true"), "{prompt}");
    assert!(
        prompt.contains("\"excluded_ineligible_source_count\":2"),
        "{prompt}"
    );
    assert!(!prompt.contains("Skypiea.tv"), "{prompt}");
    assert!(!prompt.contains("用户发布的集锦"), "{prompt}");

    let proposal = serde_json::json!({
        "summary": [{
            "text": "西班牙 1-0 夺得世界杯冠军。",
            "source_aliases": ["source-1", "source-2"]
        }],
        "findings": [{
            "text": "本届赛事安排在 2026 年 6 月至 7 月举行。",
            "source_aliases": ["source-3"]
        }],
        "recommendations": [],
        "limitations": []
    });

    assert!(
        admit_deep_research_report_proposal("世界杯战况", &catalog, proposal)
            .expect("admit low-confidence proposal")
            .is_none(),
        "low-confidence sources must not turn a source snapshot into synthesized success"
    );
}

#[test]
fn chinese_query_rejects_english_prose_and_keeps_chinese_sibling() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "Northwind platform policy".to_string(),
            anchor: "https://docs.rs/northwind".to_string(),
            chunks: vec![
                "Northwind SDK 3.0 supports Linux and macOS. Windows support is experimental."
                    .to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    let proposal = serde_json::json!({
        "summary": [
            {
                "text": "Northwind SDK 3.0 supports Linux and macOS.",
                "source_aliases": ["source-1"]
            },
            {
                "text": "Northwind SDK 3.0 支持 Linux 和 macOS。",
                "source_aliases": ["source-1"]
            }
        ],
        "findings": [{
            "text": "Northwind SDK 3.0 支持 Linux 和 macOS，Windows 支持仍处于实验阶段。",
            "source_aliases": ["source-1"]
        }],
        "recommendations": [],
        "limitations": []
    });

    let admitted =
        admit_deep_research_report_proposal("Northwind SDK 3.0 支持哪些平台？", &catalog, proposal)
            .expect("admit proposal")
            .expect("Chinese sibling survives");

    assert_eq!(admitted.accepted_block_count, 2);
    assert_eq!(admitted.rejected_block_count, 1);
    assert!(admitted.markdown.contains("## 直接回答"));
    assert!(admitted.markdown.contains("## 来源"));
}

#[test]
fn current_answer_rejects_cross_source_fact_stitching() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![
            DeepResearchCatalogSource {
                alias: "source-1".to_string(),
                title: "世界杯专题".to_string(),
                anchor: "https://sports.example.com/world-cup".to_string(),
                chunks: vec!["2026年7月20日报道，西班牙加时1-0击败阿根廷夺冠。".to_string()],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-2".to_string(),
                title: "Competition schedule".to_string(),
                anchor: "https://www.olympics.com/world-cup/schedule".to_string(),
                chunks: vec!["2026年世界杯于6月11日至7月19日举行，共有48支球队参加。".to_string()],
                claim_eligible: true,
            },
        ],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    let proposal = serde_json::json!({
        "summary": [{
            "text": "西班牙于2026年7月20日以1-0击败阿根廷夺冠，本届赛事于6月11日至7月19日举行，共有48支球队参加。",
            "source_aliases": ["source-1", "source-2"]
        }],
        "findings": [{
            "text": "本届赛事共有48支球队参加。",
            "source_aliases": ["source-2"]
        }],
        "recommendations": [],
        "limitations": []
    });

    assert!(
        admit_deep_research_report_proposal_at("世界杯战况", "2026-07-22", &catalog, proposal,)
            .expect("admit fact-stitched proposal")
            .is_none(),
        "one citation must not lend support to a different source's facts"
    );
}

#[test]
fn current_answer_rejects_an_outdated_stage_snapshot() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![
            DeepResearchCatalogSource {
                alias: "source-1".to_string(),
                title: "Final result".to_string(),
                anchor: "https://www.fifa.com/world-cup/2026/final".to_string(),
                chunks: vec!["2026年7月20日，西班牙1-0击败阿根廷夺冠。".to_string()],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-2".to_string(),
                title: "Earlier stage status".to_string(),
                anchor: "https://status.example.test/world-cup/group-stage".to_string(),
                chunks: vec![
                    "2026年6月26日，截至目前已有18支球队从小组赛出线，8支球队出局。".to_string(),
                ],
                claim_eligible: true,
            },
        ],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    let proposal = serde_json::json!({
        "summary": [{
            "text": "2026年7月20日，西班牙1-0击败阿根廷夺冠。",
            "source_aliases": ["source-1"]
        }],
        "findings": [{
            "text": "2026年6月26日，截至目前已有18支球队从小组赛出线，8支球队出局。",
            "source_aliases": ["source-2"]
        }],
        "recommendations": [],
        "limitations": []
    });

    assert!(
        admit_deep_research_report_proposal_at("世界杯战况", "2026-07-22", &catalog, proposal,)
            .expect("admit stale-stage proposal")
            .is_none(),
        "an earlier as-of snapshot must not become a current finding after the final"
    );
}

#[test]
fn competition_status_rejects_schedule_background_as_a_direct_answer() {
    assert!(!report_summary_answers_query_intent(
        "世界杯比分",
        "The event ended on 2026-07-19."
    ));
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "Competition schedule".to_string(),
            anchor: "https://www.olympics.com/world-cup/schedule".to_string(),
            chunks: vec!["2026年世界杯于6月11日至7月19日举行，共有48支球队参加。".to_string()],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    let proposal = serde_json::json!({
        "summary": [{
            "text": "2026年世界杯于6月11日至7月19日举行。",
            "source_aliases": ["source-1"]
        }],
        "findings": [{
            "text": "本届赛事共有48支球队参加。",
            "source_aliases": ["source-1"]
        }],
        "recommendations": [],
        "limitations": []
    });

    assert!(
        admit_deep_research_report_proposal_at("世界杯战况", "2026-07-22", &catalog, proposal,)
            .expect("admit schedule-only proposal")
            .is_none(),
        "schedule background must not be promoted as the current competition status"
    );
}

#[test]
fn broad_competition_status_rejects_a_scoreless_semifinal_caption_but_a_specific_match_keeps_it() {
    let broad_query = "World Cup result";
    let specific_query = "Argentina vs England match result";
    let semifinal = "Argentina beat England in the World Cup semi-final.";
    assert!(!report_summary_answers_query_intent(broad_query, semifinal));
    assert!(report_summary_answers_query_intent(
        specific_query,
        semifinal
    ));

    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "BBC World Cup semi-final photo caption".to_string(),
            anchor: "https://www.bbc.com/sport/football/world-cup-semifinal".to_string(),
            chunks: vec![
                "Argentina beat England in the World Cup semi-final. Lionel Messi won the player award after the match."
                    .to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    let proposal = serde_json::json!({
        "summary": [{
            "text": semifinal,
            "source_aliases": ["source-1"]
        }],
        "findings": [{
            "text": "Lionel Messi won the player award after the match.",
            "source_aliases": ["source-1"]
        }],
        "recommendations": [],
        "limitations": []
    });

    assert!(
        admit_deep_research_report_proposal_at(
            broad_query,
            "2026-07-22",
            &catalog,
            proposal.clone(),
        )
        .expect("admit broad scoreless-semifinal proposal")
        .is_none(),
        "an earlier-stage photo caption must not answer a tournament-wide result query"
    );
    assert!(
        admit_deep_research_report_proposal_at(specific_query, "2026-07-22", &catalog, proposal,)
            .expect("admit specific scoreless-match proposal")
            .is_some(),
        "a named match query may be answered by an accountable scoreless win statement"
    );
    assert!(
        deterministic_deep_research_outcome_report_at(broad_query, "2026-07-22", &catalog)
            .expect("evaluate broad deterministic outcome")
            .is_none(),
        "the deterministic path must enforce the same tournament-wide publication gate"
    );
    assert!(
        deterministic_deep_research_outcome_report_at(specific_query, "2026-07-22", &catalog)
            .expect("evaluate specific deterministic outcome")
            .is_some(),
        "the deterministic path must preserve named match-result queries"
    );
}

#[test]
fn broad_competition_status_rejects_a_scored_semifinal_inside_a_final_report() {
    let semifinal = "现年39岁的梅西仍是阿根廷的领袖，尤其是在准决赛以2:1击败英格兰的比赛中，他创造了球队的两个入球。";
    assert!(!report_summary_answers_query_intent(
        "世界杯战况",
        semifinal
    ));
    assert!(report_summary_answers_query_intent(
        "阿根廷对阵英格兰的准决赛结果",
        semifinal
    ));

    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "FIFA世界杯2026：西班牙击败阿根廷二度封王".to_string(),
            anchor: "https://www.bbc.com/zhongwen/articles/world-cup-final".to_string(),
            chunks: vec![
                "西班牙在世界杯决赛中实至名归地夺冠，费兰·托雷斯于加时赛攻入一球，最终打破十人应战的阿根廷的顽强抵抗。".to_string(),
                "加时赛第106分钟，费兰·托雷斯打入全场唯一进球，成为西班牙的英雄。".to_string(),
                semifinal.to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("evaluate the final report")
            .expect("the final outcome and goal detail support a report");
    let answer = report
        .markdown
        .split_once("## 直接回答")
        .and_then(|(_, rest)| rest.split_once("## 研究发现"))
        .map(|(answer, _)| answer)
        .expect("localized direct answer and findings");

    assert!(answer.contains("西班牙在世界杯决赛中"), "{answer}");
    assert!(!answer.contains("准决赛"), "{answer}");
    assert!(!answer.contains("2:1"), "{answer}");
}

#[test]
fn an_atomic_institutional_report_passes_the_strong_support_gate() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "Final result".to_string(),
            anchor: "https://www.fifa.com/world-cup/2026/final".to_string(),
            chunks: vec!["2026年7月20日，西班牙在决赛中以1-0击败阿根廷并夺冠。".to_string()],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    let proposal = serde_json::json!({
        "summary": [{
            "text": "2026年7月20日，西班牙以1-0击败阿根廷并夺冠。",
            "source_aliases": ["source-1"]
        }],
        "findings": [{
            "text": "西班牙在决赛中以1-0击败阿根廷。",
            "source_aliases": ["source-1"]
        }],
        "recommendations": [],
        "limitations": []
    });

    let admitted =
        admit_deep_research_report_proposal_at("世界杯战况", "2026-07-22", &catalog, proposal)
            .expect("admit institutional proposal")
            .expect("institutional source supports the atomic current answer");

    assert_eq!(admitted.direct_answer_block_count, 1);
    assert_eq!(admitted.finding_block_count, 1);
}

#[test]
fn an_atomic_accountable_publisher_report_passes_the_strong_support_gate() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "央视世界杯决赛报道".to_string(),
            anchor: "https://news.cctv.com/2026/07/20/world-cup-final.html".to_string(),
            chunks: vec![
                "2026年7月20日，西班牙在世界杯决赛中以1-0击败阿根廷并夺冠；这是西班牙时隔16年再次夺得世界杯冠军。"
                    .to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    assert!(!catalog_source_is_institutional(&catalog.sources[0].anchor));
    assert!(accountable_fallback_publisher(&catalog.sources[0].anchor));
    assert!(accountable_fallback_publisher(
        "https://sports.ifeng.com/c/world-cup-final"
    ));
    let proposal = serde_json::json!({
        "summary": [{
            "text": "2026年7月20日，西班牙以1-0击败阿根廷并夺冠。",
            "source_aliases": ["source-1"]
        }],
        "findings": [{
            "text": "西班牙时隔16年再次夺得世界杯冠军。",
            "source_aliases": ["source-1"]
        }],
        "recommendations": [],
        "limitations": []
    });

    let admitted =
        admit_deep_research_report_proposal_at("世界杯战况", "2026-07-22", &catalog, proposal)
            .expect("admit accountable publisher proposal")
            .expect("an accountable publisher supports complete atomic claims");

    assert_eq!(admitted.direct_answer_block_count, 1);
    assert_eq!(admitted.finding_block_count, 1);
    assert_eq!(admitted.cited_source_count, 1);
}

#[test]
fn a_semantically_admitted_unknown_publisher_cannot_pass_strong_support() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "世界杯决赛汇总".to_string(),
            anchor: "https://sports.example.test/2026/world-cup-final".to_string(),
            chunks: vec![
                "2026年7月20日，西班牙在世界杯决赛中以1-0击败阿根廷并夺冠；这是西班牙时隔16年再次夺得世界杯冠军。"
                    .to_string(),
            ],
            // Semantic admission may retain an unknown publisher for audit,
            // but it does not turn that publisher into an accountable source.
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };
    assert!(!catalog_source_is_institutional(&catalog.sources[0].anchor));
    assert!(!accountable_fallback_publisher(&catalog.sources[0].anchor));
    let proposal = serde_json::json!({
        "summary": [{
            "text": "2026年7月20日，西班牙以1-0击败阿根廷并夺冠。",
            "source_aliases": ["source-1"]
        }],
        "findings": [{
            "text": "西班牙时隔16年再次夺得世界杯冠军。",
            "source_aliases": ["source-1"]
        }],
        "recommendations": [],
        "limitations": []
    });

    assert!(
        admit_deep_research_report_proposal_at("世界杯战况", "2026-07-22", &catalog, proposal,)
            .expect("admit unknown-publisher proposal")
            .is_none(),
        "semantic selection must not manufacture publisher accountability"
    );
}

#[test]
fn deterministic_outcome_report_uses_exact_accountable_source_spans() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![
            DeepResearchCatalogSource {
                alias: "source-1".to_string(),
                title: "世界杯赛程".to_string(),
                anchor: "https://www.olympics.com/world-cup/schedule".to_string(),
                chunks: vec![
                    "谁将最终加冕世界冠军？答案将在2026年世界杯决赛后揭晓。"
                        .to_string(),
                ],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-2".to_string(),
                title: "2026美加墨世界杯_新华网".to_string(),
                anchor: "https://www.news.cn/sports/topic/fifa2026/index.htm".to_string(),
                chunks: vec![
                    "西班牙男足重回世界排名第一 中国队居第91位 世界杯冠军西班牙队重新回到世界第一的位置，阿根廷队排在第二位。国际足联调查世界杯决赛赛后冲突，赛后两队爆发冲突。"
                        .to_string(),
                    "阿根廷主帅斯卡洛尼表示西班牙队配得上胜利。西班牙后卫佩德罗·波罗在夺冠后准备把奖杯带回家。"
                        .to_string(),
                ],
                claim_eligible: true,
            },
        ],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("build deterministic outcome report")
            .expect("accountable outcome evidence supports an extractive report");

    let answer = report
        .markdown
        .split_once("## 直接回答")
        .and_then(|(_, rest)| rest.split_once("## 研究发现"))
        .map(|(answer, _)| answer)
        .expect("localized direct answer and findings");
    assert!(answer.contains("世界杯冠军西班牙队"), "{answer}");
    assert!(!answer.contains("谁将最终加冕"), "{answer}");
    assert!(report.markdown.contains("西班牙队配得上胜利"));
    assert_eq!(report.direct_answer_block_count, 1);
    assert!((1..=3).contains(&report.finding_block_count));
    assert_eq!(report.cited_source_count, 1);
}

#[test]
fn deterministic_outcome_report_prefers_the_atomic_final_score_and_corroborates_it() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![
            DeepResearchCatalogSource {
                alias: "source-1".to_string(),
                title: "费兰·托雷斯加时赛绝杀，西班牙加冕世界杯冠军".to_string(),
                anchor: "https://www.olympics.com/zh/news/world-cup-final".to_string(),
                chunks: vec![
                    "西班牙队在决赛中凭借费兰·托雷斯的加时进球1比0力克十人应战的阿根廷队，时隔16年再度捧起大力神杯。".to_string(),
                    "此役过后，西班牙队以7场比赛仅失1球的成绩夺冠，而卫冕冠军阿根廷的13场世界杯不败纪录就此终结。".to_string(),
                    "第106分钟，费兰·托雷斯打入全场比赛唯一进球。".to_string(),
                ],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-2".to_string(),
                title: "2026世界杯冠军：西班牙".to_string(),
                anchor: "https://www.xinmin.cn/2026WorldCup/".to_string(),
                chunks: vec![
                    "北京时间7月20日的决赛中，西班牙加时赛1:0战胜阿根廷，费兰·托雷斯第106分钟打入制胜球。".to_string(),
                    "7月20日 决赛：西班牙 1-0 阿根廷（加时，西班牙夺冠）。".to_string(),
                    "冠军：西班牙（决赛加时 1:0 战胜阿根廷，费兰·托雷斯第106分钟制胜）。".to_string(),
                ],
                claim_eligible: true,
            },
        ],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("build deterministic outcome report")
            .expect("the independently reported final score supports a report");
    let answer = report
        .markdown
        .split_once("## 直接回答")
        .and_then(|(_, rest)| rest.split_once("## 研究发现"))
        .map(|(answer, _)| answer)
        .expect("localized direct answer and findings");

    assert!(answer.contains("1比0力克"), "{answer}");
    assert!(!answer.contains("7场比赛仅失1球"), "{answer}");
    assert_eq!(report.cited_source_count, 2, "{}", report.markdown);
    assert_eq!(
        report.markdown.matches("[[2]]").count(),
        1,
        "{}",
        report.markdown
    );
}

#[test]
fn deterministic_outcome_report_rejects_navigation_piles_and_uses_atomic_result() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![
            DeepResearchCatalogSource {
                alias: "source-1".to_string(),
                title: "2026年国际足联世界杯：完整赛程、比赛结果、进球和积分榜一览"
                    .to_string(),
                anchor: "https://www.olympics.com/zh/news/fifa-world-cup-2026-schedule-results-scores-standings-list"
                    .to_string(),
                chunks: vec![
                    "运动项目 * 新闻 * Olympic Channel * 动就一起 # 2026年国际足联世界杯：完整赛程、比赛结果、进球和积分榜一览 2026年国际足联美加墨世界杯正在如火如荼进行当中，通过本文查看每日赛果、淘汰赛对阵情况、比分以及小组积分榜。"
                        .to_string(),
                    "谁将最终加冕世界冠军？答案将在2026年国际足联世界杯揭晓。 本届世界杯于2026年6月11日至7月19日举行。 ## 2026年国际足联世界杯：每日赛果及淘汰赛对阵情况 ## 2026年国际足联世界杯：小组赛积分榜一览 ### A组 - 墨西哥、南非、捷克、韩国 ### B组 - 波黑、加拿大、卡塔尔、瑞士"
                        .to_string(),
                ],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-2".to_string(),
                title: "2026美加墨世界杯_新华网".to_string(),
                anchor: "https://www.news.cn/sports/topic/fifa2026/index.htm".to_string(),
                chunks: vec![
                    "2026美加墨世界杯\\_新华网 * 新华体育 * 专题首页 * 聚焦世界杯 * 赛场零距离 * 赛场速递 * 直击现场 * 评球美加墨 ### 西班牙队二次捧杯 姆巴佩夺“金靴” 西班牙队凭借费兰·托雷斯在加时赛的进球，1:0战胜10人阿根廷队，队史第二次夺得世界杯冠军。法国队姆巴佩以10球获得金靴奖。"
                        .to_string(),
                    "西班牙男足重回世界排名第一 中国队居第91位 * 美加墨世界杯：中国伙伴的“新呈现” * 国际足联调查世界杯决赛赛后冲突 * 回看美加墨 世界杯经历大变革 * 西班牙队凯旋 首相接见 全城狂欢 * 世界杯｜美加墨世界杯的三个意难平 * 大咖说丨裁判马宁：谈退役、评AI、驳斥假消息 * 世界杯丨世界杯冠军西班牙队举行巡游庆祝活动"
                        .to_string(),
                    "西班牙男足重回世界排名第一 中国队居第91位 世界杯冠军西班牙队重新回到世界第一的位置，阿根廷队排在第二位。中国队则没有变化，依旧排在第91位。国际足联调查世界杯决赛赛后冲突，赛后两队爆发冲突。"
                        .to_string(),
                ],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-3".to_string(),
                title: "世界杯新闻_新浪体育_新浪网".to_string(),
                anchor: "https://sports.sina.com.cn/g/worldcup/".to_string(),
                chunks: vec![
                    "盘点近3届世界杯大冷门 巴西1-7德国比分500倍".to_string(),
                    "世界杯夺冠赔率:巴法西3强并列第1 国足1赔501".to_string(),
                    "世界杯 07/19 05:00 已结束 直播 数据 预测 世界杯 07/20 03:00 已结束"
                        .to_string(),
                ],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-4".to_string(),
                title: "2026世界杯冠军：西班牙｜决赛赛果·全部比分（北京时间）- 新民晚报"
                    .to_string(),
                anchor: "https://www.xinmin.cn/2026WorldCup/".to_string(),
                chunks: vec![
                    "北京时间7月20日的决赛中，西班牙加时赛1比0战胜阿根廷，费兰·托雷斯第106分钟打入制胜球。"
                        .to_string(),
                ],
                claim_eligible: true,
            },
        ],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("build deterministic outcome report")
            .expect("the accountable result sentence supports an extractive report");
    let answer = report
        .markdown
        .split_once("## 直接回答")
        .and_then(|(_, rest)| rest.split_once("## 研究发现"))
        .map(|(answer, _)| answer)
        .expect("localized direct answer and findings");

    assert!(
        answer.contains(
            "西班牙队凭借费兰·托雷斯在加时赛的进球，1:0战胜10人阿根廷队，队史第二次夺得世界杯冠军。"
        ),
        "{answer}"
    );
    assert!(answer.trim_start().starts_with("西班牙队凭借"), "{answer}");
    assert!(!answer.contains("专题首页"), "{answer}");
    assert!(!answer.contains("举行巡游庆祝活动"), "{answer}");
    assert!(
        !report.markdown.contains("通过本文查看"),
        "{}",
        report.markdown
    );
    assert!(
        !report.markdown.contains("小组赛积分榜一览"),
        "{}",
        report.markdown
    );
    assert!(
        report.markdown.contains("法国队姆巴佩以10球获得金靴奖。"),
        "{}",
        report.markdown
    );
    assert!(
        report
            .markdown
            .contains("世界杯冠军西班牙队重新回到世界第一的位置，阿根廷队排在第二位。"),
        "{}",
        report.markdown
    );
    assert!(
        !report
            .markdown
            .contains("中国队居第91位 世界杯冠军西班牙队"),
        "{}",
        report.markdown
    );
    assert!(
        !report.markdown.contains("世界杯大冷门"),
        "{}",
        report.markdown
    );
    assert!(!report.markdown.contains("夺冠赔率"), "{}", report.markdown);
    assert!(
        !report.markdown.contains("05:00 已结束"),
        "{}",
        report.markdown
    );
    assert!(
        report.markdown.contains(
            "北京时间7月20日的决赛中，西班牙加时赛1比0战胜阿根廷，费兰·托雷斯第106分钟打入制胜球。"
        ),
        "{}",
        report.markdown
    );
    assert_eq!(report.cited_source_count, 2, "{}", report.markdown);
    assert!(
        report
            .markdown
            .contains("[[1]](https://www.news.cn/sports/topic/fifa2026/index.htm)"),
        "the only cited source must be numbered one: {}",
        report.markdown
    );
    let sources = report
        .markdown
        .split_once("## 来源")
        .map(|(_, sources)| sources)
        .expect("localized source ledger");
    assert!(
        sources.contains(
            "1. [2026美加墨世界杯\\_新华网](https://www.news.cn/sports/topic/fifa2026/index.htm)"
        ),
        "{sources}"
    );
    assert!(!sources.contains("2. [2]"), "{sources}");
}

#[test]
fn deterministic_outcome_report_does_not_join_a_same_score_from_another_event() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![
            DeepResearchCatalogSource {
                alias: "source-1".to_string(),
                title: "世界杯决赛报道".to_string(),
                anchor: "https://www.news.cn/sports/world-cup-final".to_string(),
                chunks: vec![
                    "西班牙队加时赛1:0战胜阿根廷队，夺得世界杯冠军。法国队姆巴佩以10球获得金靴奖。"
                        .to_string(),
                ],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-2".to_string(),
                title: "另一场比赛报道".to_string(),
                anchor: "https://www.cctv.com/sports/another-final".to_string(),
                chunks: vec!["德国队加时赛1:0战胜法国队，穆西亚拉第106分钟打入制胜球。".to_string()],
                claim_eligible: true,
            },
        ],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("build deterministic report")
            .expect("the primary event has an answer and a distinct finding");

    assert!(!report.markdown.contains("德国队"), "{}", report.markdown);
    assert!(!report.markdown.contains("穆西亚拉"), "{}", report.markdown);
    assert_eq!(report.cited_source_count, 1, "{}", report.markdown);
}

#[test]
fn deterministic_outcome_report_preserves_an_english_sentence_subject() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "World Cup final report".to_string(),
            anchor: "https://www.fifa.com/tournaments/world-cup/final-report".to_string(),
            chunks: vec![
                "World Cup final: Spain beat Argentina 1-0 and became champion. Kylian Mbappe won the Golden Boot with 10 goals."
                    .to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("World Cup result", "2026-07-22", &catalog)
            .expect("build deterministic English outcome report")
            .expect("the official result sentence supports an extractive report");
    let answer = report
        .markdown
        .split_once("## Direct Answer")
        .and_then(|(_, rest)| rest.split_once("## Findings"))
        .map(|(answer, _)| answer.trim_start())
        .expect("English direct answer and findings");

    assert!(
        answer.starts_with("World Cup final: Spain beat Argentina"),
        "{answer}"
    );
    assert!(!answer.starts_with("beat Argentina"), "{answer}");
}

#[test]
fn deterministic_outcome_report_drops_a_headline_that_only_restates_the_answer() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "2026美加墨世界杯_新华网".to_string(),
            anchor: "https://www.news.cn/sports/topic/fifa2026/index.htm".to_string(),
            chunks: vec![
                "西班牙队凭借费兰·托雷斯在加时赛的进球，1:0战胜10人阿根廷队，队史第二次夺得世界杯冠军。法国队姆巴佩以10球获得金靴奖。"
                    .to_string(),
                "世界杯丨西班牙队夺冠".to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("build deterministic outcome report")
            .expect("the result and award support an extractive report");

    assert!(report.markdown.contains("法国队姆巴佩以10球获得金靴奖。"));
    assert!(
        !report.markdown.contains("世界杯丨西班牙队夺冠"),
        "{}",
        report.markdown
    );
    assert_eq!(report.finding_block_count, 1, "{}", report.markdown);
}

#[test]
fn deterministic_outcome_report_drops_a_navigation_headline_prefix_and_unrelated_accident() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "2026年fifa世界杯 - 联合早报".to_string(),
            anchor: "https://www.zaobao.com.sg/specials/fifa-world-cup-2026".to_string(),
            chunks: vec![
                "冠军球队凭借控制耐心 西班牙凭借托里斯加时赛下半场的进球，以1比0战胜韧性极强的阿根廷夺冠。费兰·托雷斯第106分钟打入全场唯一进球。"
                    .to_string(),
                "西班牙夺冠庆祝乐极生悲 喷泉坍塌致13岁少年身亡。".to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("build deterministic topic-page report")
            .expect("the atomic result and goal detail support a report");
    let answer = report
        .markdown
        .split_once("## 直接回答")
        .and_then(|(_, rest)| rest.split_once("## 研究发现"))
        .map(|(answer, _)| answer.trim_start())
        .expect("localized direct answer and findings");

    assert!(answer.starts_with("西班牙凭借"), "{answer}");
    assert!(!answer.contains("冠军球队凭借控制耐心"), "{answer}");
    assert!(!report.markdown.contains("喷泉坍塌"), "{}", report.markdown);
}

#[test]
fn deterministic_outcome_report_splits_medal_summary_and_drops_earlier_stage_findings() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "2026世界杯冠军：西班牙｜决赛赛果·全部比分".to_string(),
            anchor: "https://www.xinmin.cn/2026WorldCup/".to_string(),
            chunks: vec![
                "新民晚报全媒体 2026世界杯 最终结果 🏆 冠军：西班牙（决赛加时 1:0 战胜阿根廷，费兰·托雷斯第106分钟制胜） 🥉 季军：英格兰（季军赛 6:4 胜法国） 半决赛：法国 0:2 西班牙；"
                    .to_string(),
                "7月12日 1/4决赛：阿根廷 3-1 瑞士（加时）。费兰·托雷斯第106分钟打入全场唯一进球。"
                    .to_string(),
                "同阶段出局的球队按全部比赛总积分排序，完整战绩见本页最终排名栏目。"
                    .to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    let report =
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("build deterministic medal-summary report")
            .expect("the final result and goal detail support a report");
    let answer = report
        .markdown
        .split_once("## 直接回答")
        .and_then(|(_, rest)| rest.split_once("## 研究发现"))
        .map(|(answer, _)| answer)
        .expect("localized direct answer and findings");

    assert!(answer.contains("冠军：西班牙"), "{answer}");
    assert!(!answer.contains("季军"), "{answer}");
    assert!(!answer.contains("半决赛"), "{answer}");
    assert!(!report.markdown.contains("1/4决赛"), "{}", report.markdown);
    assert!(!report.markdown.contains("见本页"), "{}", report.markdown);
}

#[test]
fn deterministic_outcome_report_rejects_schedule_only_evidence() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "世界杯赛程".to_string(),
            anchor: "https://www.olympics.com/world-cup/schedule".to_string(),
            chunks: vec![
                "谁将最终加冕世界冠军？答案将在2026年世界杯决赛后揭晓。".to_string(),
                "本届世界杯于2026年6月11日至7月19日举行。".to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    assert!(
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog,)
            .expect("evaluate schedule-only evidence")
            .is_none(),
        "future schedule prose must never become an extractive current answer"
    );
}

#[test]
fn deterministic_outcome_report_rejects_betting_history_and_schedule_chrome() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "世界杯新闻_新浪体育_新浪网".to_string(),
            anchor: "https://sports.sina.com.cn/g/worldcup/".to_string(),
            chunks: vec![
                "盘点近3届世界杯大冷门 巴西1-7德国比分500倍".to_string(),
                "世界杯夺冠赔率:巴法西3强并列第1 国足1赔501".to_string(),
                "世界杯 07/19 05:00 已结束 直播 数据 预测 世界杯 07/20 03:00 已结束".to_string(),
            ],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    assert!(
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog)
            .expect("evaluate betting and schedule chrome")
            .is_none(),
        "historical roundups, betting odds, and time-only widgets cannot become a current result"
    );
}

#[test]
fn deterministic_outcome_report_rejects_an_unknown_publisher() {
    let catalog = DeepResearchSourceCatalog {
        sources: vec![DeepResearchCatalogSource {
            alias: "source-1".to_string(),
            title: "世界杯赛果汇总".to_string(),
            anchor: "https://scores.example.test/world-cup".to_string(),
            chunks: vec!["世界杯冠军西班牙队击败阿根廷队。西班牙队配得上这场胜利。".to_string()],
            claim_eligible: true,
        }],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    };

    assert!(
        deterministic_deep_research_outcome_report_at("世界杯战况", "2026-07-22", &catalog,)
            .expect("evaluate unknown publisher")
            .is_none(),
        "semantic admission alone must not authorize deterministic synthesis"
    );
}

fn proposal_catalog() -> DeepResearchSourceCatalog {
    DeepResearchSourceCatalog {
        sources: vec![
            DeepResearchCatalogSource {
                alias: "source-1".to_string(),
                title: "Release record".to_string(),
                anchor: "https://www.fifa.com/aurora".to_string(),
                chunks: vec![
                    "The release record states 14 March 2026. The record provides no explanation for a conflicting deployment date."
                        .to_string(),
                ],
                claim_eligible: true,
            },
            DeepResearchCatalogSource {
                alias: "source-2".to_string(),
                title: "Deployment record".to_string(),
                anchor: "https://www.olympics.com/aurora".to_string(),
                chunks: vec![
                    "The deployment record states 18 March 2026. The record provides no explanation for a conflicting release date."
                        .to_string(),
                ],
                claim_eligible: true,
            },
        ],
        omitted_source_count: 0,
        omitted_chunk_count: 0,
    }
}
