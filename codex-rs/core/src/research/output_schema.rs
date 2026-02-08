use serde_json::Map;
use serde_json::Value;
use serde_json::json;

#[must_use]
pub fn research_output_schema() -> Value {
    let mut defs = Map::new();

    defs.insert(
        "problem_decomposition".to_string(),
        json!({
            "type": "object",
            "properties": {
                "problem_statement": { "type": "string" },
                "core_challenges": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "description": { "type": "string" },
                            "domains": { "type": "array", "items": { "type": "string" } },
                            "search_keywords": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["name", "description"]
                    }
                },
                "constraints": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "category": { "type": "string" },
                            "description": { "type": "string" },
                            "value": { "type": "string" }
                        },
                        "required": ["category", "description"]
                    }
                },
                "scope": {
                    "type": "object",
                    "properties": {
                        "in_scope": { "type": "array", "items": { "type": "string" } },
                        "out_of_scope": { "type": "array", "items": { "type": "string" } }
                    }
                }
            },
            "required": ["problem_statement", "core_challenges", "constraints"]
        }),
    );

    defs.insert(
        "literature_review".to_string(),
        json!({
            "type": "object",
            "properties": {
                "total_papers_found": { "type": "integer", "minimum": 0 },
                "papers_by_id": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "object",
                        "properties": {
                            "paper_id": { "type": "string" },
                            "title": { "type": "string" },
                            "authors": { "type": "string" },
                            "year": { "type": "integer" },
                            "venue": { "type": "string" },
                            "doi": { "type": "string" },
                            "arxiv_id": { "type": "string" },
                            "url": { "type": "string" },
                            "pdf_url": { "type": "string" },
                            "code_url": { "type": "string" },
                            "citation_count": { "type": "integer", "minimum": 0 },
                            "abstract_text": { "type": "string" },
                            "relevance_score": { "type": "integer", "minimum": 0 },
                            "key_contribution": { "type": "string" },
                            "relevant_to_subproblems": { "type": "array", "items": { "type": "string" } },
                            "source": { "type": "string" }
                        },
                        "required": ["title", "authors"]
                    }
                },
                "paper_ids_ranked": { "type": "array", "items": { "type": "string" } },
                "papers": {
                    "type": "array",
                    "description": "DEPRECATED flat list. Use papers_by_id + paper_ids_ranked.",
                    "items": { "type": "object" }
                },
                "sota_results": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "task": { "type": "string" },
                            "dataset": { "type": "string" },
                            "metric": { "type": "string" },
                            "best_method": { "type": "string" },
                            "best_score": { "type": "number" },
                            "paper_title": { "type": "string" },
                            "paper_url": { "type": "string" },
                            "code_url": { "type": "string" }
                        },
                        "required": ["task", "dataset", "metric", "best_method"]
                    }
                },
                "code_repositories": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "url": { "type": "string" },
                            "name": { "type": "string" },
                            "framework": { "type": "string" },
                            "stars": { "type": "integer", "minimum": 0 },
                            "is_official": { "type": "boolean" },
                            "related_paper": { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["url", "name"]
                    }
                },
                "key_insights": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["total_papers_found", "papers_by_id"]
        }),
    );

    defs.insert(
        "proposal_key_paper".to_string(),
        json!({
            "type": "object",
            "properties": {
                "citation": { "type": "string" },
                "title": { "type": "string" },
                "url": { "type": "string" },
                "relevance": { "type": "string" }
            },
            "required": ["citation", "title"]
        }),
    );

    defs.insert(
        "proposal_technical_approach".to_string(),
        json!({
            "type": "object",
            "properties": {
                "architecture": { "type": "string" },
                "training_procedure": { "type": "string" },
                "inference_pipeline": { "type": "string" },
                "integration_approach": { "type": "string" }
            }
        }),
    );

    defs.insert(
        "proposal_acceptance_criterion".to_string(),
        json!({
            "type": "object",
            "properties": {
                "metric": { "type": "string" },
                "threshold": { "type": "string" },
                "priority": { "type": "string" },
                "source": { "type": "string" }
            },
            "required": ["metric", "threshold", "priority"]
        }),
    );

    defs.insert(
        "proposal_experiment_plan".to_string(),
        json!({
            "type": "object",
            "properties": {
                "dataset_subset": { "type": "string" },
                "expected_training_time": { "type": "string" },
                "key_metric": { "type": "string" },
                "expected_range": { "type": "string" },
                "go_no_go": { "type": "string" }
            }
        }),
    );

    defs.insert(
        "proposal_risk_entry".to_string(),
        json!({
            "type": "object",
            "properties": {
                "risk": { "type": "string" },
                "likelihood": { "type": "string" },
                "impact": { "type": "string" },
                "mitigation": { "type": "string" }
            },
            "required": ["risk", "likelihood", "impact"]
        }),
    );

    defs.insert(
        "proposal_instrumentation_requirements".to_string(),
        json!({
            "type": "object",
            "properties": {
                "training_metrics": { "type": "array", "items": { "type": "string" } },
                "deployment_metrics": { "type": "array", "items": { "type": "string" } },
                "alerting_thresholds": { "type": "array", "items": { "type": "string" } }
            }
        }),
    );

    defs.insert(
        "proposal_deployment_constraints".to_string(),
        json!({
            "type": "object",
            "properties": {
                "target_hardware": { "type": "string" },
                "model_format": { "type": "string" },
                "runtime": { "type": "string" },
                "memory_budget": { "type": "string" },
                "rollback_strategy": { "type": "string" }
            }
        }),
    );

    defs.insert(
        "proposal_data_requirements".to_string(),
        json!({
            "type": "object",
            "properties": {
                "description": { "type": "string" },
                "estimated_volume": { "type": "string" },
                "availability": { "type": "string" },
                "labeling_effort": { "type": "string" }
            }
        }),
    );

    defs.insert(
        "proposal_compute_requirements".to_string(),
        json!({
            "type": "object",
            "properties": {
                "training_gpu": { "type": "string" },
                "training_time": { "type": "string" },
                "inference_device": { "type": "string" },
                "inference_latency": { "type": "string" }
            }
        }),
    );

    defs.insert(
        "proposal_risk_assessment".to_string(),
        json!({
            "type": "object",
            "properties": {
                "level": { "type": "string" },
                "explanation": { "type": "string" }
            },
            "description": "Short overall risk summary."
        }),
    );

    defs.insert(
        "proposal_evidence".to_string(),
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "type": { "type": "string" },
                "source_tool": { "type": "string" },
                "source_ref": { "type": "string" },
                "extracted_text_snippet": { "type": "string" }
            },
            "required": ["id", "type", "source_ref"]
        }),
    );

    defs.insert(
        "proposal_adoption_strategy".to_string(),
        json!({
            "type": "object",
            "properties": {
                "strategy": { "type": "string" },
                "rationale": { "type": "string" }
            },
            "required": ["strategy", "rationale"]
        }),
    );

    defs.insert(
        "proposal_repo_provenance".to_string(),
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "pinned_commit": { "type": "string" },
                "license": { "type": "string" },
                "health_summary": { "type": "string" },
                "caveats": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["url", "license"]
        }),
    );

    defs.insert(
        "proposal_implementation_packet".to_string(),
        json!({
            "type": "object",
            "properties": {
                "adoption_strategy": { "$ref": "#/$defs/proposal_adoption_strategy" },
                "repo_provenance": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/proposal_repo_provenance" }
                },
                "repro_plan": { "type": "object" },
                "integration_plan": { "type": "object" },
                "codebase_fit": { "type": "object" }
            },
            "required": ["adoption_strategy", "repo_provenance"]
        }),
    );

    defs.insert(
        "proposal_repo_assessment".to_string(),
        json!({
            "type": "object",
            "properties": {
                "repo_url": { "type": "string" },
                "commit_sha": { "type": "string" },
                "license": { "type": "string" },
                "repo_unavailable": { "type": "boolean" },
                "blocked_reason": { "type": "string" }
            },
            "required": ["repo_url"]
        }),
    );

    defs.insert(
        "proposal".to_string(),
        json!({
            "type": "object",
            "properties": {
                "rank": { "type": "integer", "minimum": 1 },
                "name": { "type": "string" },
                "status": { "type": "string" },
                "summary": { "type": "string" },
                "key_papers": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/proposal_key_paper" }
                },
                "technical_approach": { "$ref": "#/$defs/proposal_technical_approach" },
                "justification": { "type": "string" },
                "acceptance_criteria": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/proposal_acceptance_criterion" }
                },
                "experiment_plan": { "$ref": "#/$defs/proposal_experiment_plan" },
                "risk_register": {
                    "type": "array",
                    "description": "Itemized risk details.",
                    "items": { "$ref": "#/$defs/proposal_risk_entry" }
                },
                "instrumentation_requirements": {
                    "$ref": "#/$defs/proposal_instrumentation_requirements"
                },
                "deployment_constraints": { "$ref": "#/$defs/proposal_deployment_constraints" },
                "strengths": { "type": "array", "items": { "type": "string" } },
                "weaknesses": { "type": "array", "items": { "type": "string" } },
                "failure_modes": { "type": "array", "items": { "type": "string" } },
                "data_requirements": { "$ref": "#/$defs/proposal_data_requirements" },
                "compute_requirements": { "$ref": "#/$defs/proposal_compute_requirements" },
                "expertise_required": { "type": "array", "items": { "type": "string" } },
                "risk_assessment": { "$ref": "#/$defs/proposal_risk_assessment" },
                "evidence": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/proposal_evidence" }
                },
                "comparison_to_alternatives": { "type": "string" },
                "implementation_files": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "implementation_packet": { "$ref": "#/$defs/proposal_implementation_packet" },
                "repo_assessments": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/proposal_repo_assessment" }
                }
            },
            "required": ["name", "summary"]
        }),
    );

    defs.insert(
        "champion".to_string(),
        json!({
            "type": "object",
            "properties": {
                "proposal_name": { "type": "string" },
                "justification": { "type": "string" },
                "promotion_criteria": { "type": "string" }
            },
            "required": ["proposal_name", "justification"]
        }),
    );

    defs.insert(
        "decision_rule".to_string(),
        json!({
            "type": "object",
            "properties": {
                "condition": { "type": "string" },
                "action": { "type": "string" },
                "rationale": { "type": "string" }
            },
            "required": ["condition", "action"]
        }),
    );

    defs.insert(
        "iteration_context".to_string(),
        json!({
            "type": "object",
            "properties": {
                "iteration_number": { "type": "integer", "minimum": 0 },
                "prior_proposal_ids": { "type": "array", "items": { "type": "string" } },
                "feedback_received": { "type": "array", "items": { "type": "object" } },
                "changes_from_prior": { "type": "array", "items": { "type": "string" } },
                "dead_approaches": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["iteration_number"]
        }),
    );

    defs.insert(
        "file_manifest_entry".to_string(),
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "kind": { "type": "string" },
                "solution_rank": { "type": "integer", "minimum": 1 },
                "description": { "type": "string" }
            },
            "required": ["path", "kind"]
        }),
    );

    defs.insert(
        "research_metadata".to_string(),
        json!({
            "type": "object",
            "properties": {
                "research_date": { "type": "string" },
                "model_used": { "type": "string" },
                "total_papers_reviewed": { "type": "integer", "minimum": 0 },
                "num_sub_agents": { "type": "integer", "minimum": 0 },
                "framework": { "type": "string" },
                "codex_version": { "type": "string" }
            }
        }),
    );

    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "ResearchOutput",
        "type": "object",
        "properties": {
            "problem_decomposition": { "$ref": "#/$defs/problem_decomposition" },
            "literature_review": { "$ref": "#/$defs/literature_review" },
            "proposals": {
                "type": "array",
                "items": { "$ref": "#/$defs/proposal" }
            },
            "solutions": {
                "type": "array",
                "description": "DEPRECATED alias of `proposals`.",
                "items": { "$ref": "#/$defs/proposal" }
            },
            "comparison_table": { "type": "string" },
            "champion": { "$ref": "#/$defs/champion" },
            "recommended_solution": {
                "type": "string",
                "description": "DEPRECATED alias of `champion.proposal_name`."
            },
            "decision_rules": {
                "type": "array",
                "items": { "$ref": "#/$defs/decision_rule" }
            },
            "iteration_context": { "$ref": "#/$defs/iteration_context" },
            "next_steps": { "type": "array", "items": { "type": "string" } },
            "open_questions": { "type": "array", "items": { "type": "string" } },
            "run_warnings": { "type": "array", "items": { "type": "string" } },
            "file_manifest": {
                "type": "array",
                "items": { "$ref": "#/$defs/file_manifest_entry" }
            },
            "metadata": { "$ref": "#/$defs/research_metadata" }
        },
        "required": [
            "problem_decomposition",
            "literature_review",
            "proposals",
            "champion"
        ],
        "$defs": defs
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn top_level_required_fields_use_canonical_names() {
        let schema = research_output_schema();
        let required = schema["required"].as_array().expect("array");
        let required = required
            .iter()
            .map(|value| value.as_str().expect("string"))
            .collect::<Vec<_>>();
        assert_eq!(
            required,
            vec![
                "problem_decomposition",
                "literature_review",
                "proposals",
                "champion"
            ]
        );
    }

    #[test]
    fn schema_keeps_deprecated_aliases_optional() {
        let schema = research_output_schema();
        assert!(schema["properties"]["solutions"].is_object());
        assert!(schema["properties"]["recommended_solution"].is_object());
    }

    #[test]
    fn literature_review_requires_papers_by_id() {
        let schema = research_output_schema();
        let required = schema["$defs"]["literature_review"]["required"]
            .as_array()
            .expect("array")
            .iter()
            .map(|value| value.as_str().expect("string"))
            .collect::<Vec<_>>();
        assert_eq!(required, vec!["total_papers_found", "papers_by_id"]);
    }

    #[test]
    fn proposals_shape_points_to_proposal_definition() {
        let schema = research_output_schema();
        assert_eq!(
            schema["properties"]["proposals"]["items"]["$ref"]
                .as_str()
                .expect("string"),
            "#/$defs/proposal"
        );
    }

    #[test]
    fn literature_review_includes_extended_fields_from_types() {
        let schema = research_output_schema();
        let properties = &schema["$defs"]["literature_review"]["properties"];
        assert!(properties["sota_results"].is_object());
        assert!(properties["code_repositories"].is_object());
        assert!(properties["key_insights"].is_object());
    }

    #[test]
    fn proposal_includes_extended_fields_from_types() {
        let schema = research_output_schema();
        let properties = &schema["$defs"]["proposal"]["properties"];
        assert!(properties["evidence"].is_object());
        assert!(properties["instrumentation_requirements"].is_object());
        assert!(properties["deployment_constraints"].is_object());
        assert!(properties["failure_modes"].is_object());
        assert!(properties["data_requirements"].is_object());
        assert!(properties["compute_requirements"].is_object());
        assert!(properties["expertise_required"].is_object());
        assert!(properties["comparison_to_alternatives"].is_object());
        assert!(properties["implementation_files"].is_object());
    }

    #[test]
    fn proposal_nested_contract_fields_are_structured() {
        let schema = research_output_schema();
        let properties = &schema["$defs"]["proposal"]["properties"];
        assert_eq!(
            properties["key_papers"]["items"]["$ref"],
            json!("#/$defs/proposal_key_paper")
        );
        assert_eq!(
            schema["$defs"]["proposal_key_paper"]["required"],
            json!(["citation", "title"])
        );
        assert_eq!(
            properties["technical_approach"]["$ref"],
            json!("#/$defs/proposal_technical_approach")
        );
        assert_eq!(
            properties["acceptance_criteria"]["items"]["$ref"],
            json!("#/$defs/proposal_acceptance_criterion")
        );
        assert_eq!(
            schema["$defs"]["proposal_acceptance_criterion"]["required"],
            json!(["metric", "threshold", "priority"])
        );
        assert_eq!(
            properties["experiment_plan"]["$ref"],
            json!("#/$defs/proposal_experiment_plan")
        );
        assert_eq!(
            properties["risk_register"]["items"]["$ref"],
            json!("#/$defs/proposal_risk_entry")
        );
        assert_eq!(
            schema["$defs"]["proposal_risk_entry"]["required"],
            json!(["risk", "likelihood", "impact"])
        );
        assert_eq!(
            properties["risk_assessment"]["$ref"],
            json!("#/$defs/proposal_risk_assessment")
        );
        assert_eq!(
            properties["implementation_packet"]["$ref"],
            json!("#/$defs/proposal_implementation_packet")
        );
        assert_eq!(
            schema["$defs"]["proposal_implementation_packet"]["required"],
            json!(["adoption_strategy", "repo_provenance"])
        );
        assert_eq!(
            properties["repo_assessments"]["items"]["$ref"],
            json!("#/$defs/proposal_repo_assessment")
        );
        assert_eq!(
            schema["$defs"]["proposal_repo_assessment"]["required"],
            json!(["repo_url"])
        );
    }
}
