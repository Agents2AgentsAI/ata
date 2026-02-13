use pretty_assertions::assert_eq;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;
use wiremock::matchers::query_param;

use crate::ResearchToolkit;
use crate::config::ResearchConfig;
use crate::error::ResearchError;
use crate::tools::test_helpers::build_test_toolkit_with_config;
use crate::types::DocumentResolution;
use crate::types::DocumentSourceKind;
use crate::types::ZoteroAdvancedCandidateStrategy;
use crate::types::ZoteroAdvancedCompleteness;
use crate::types::ZoteroAdvancedSearchParams;
use crate::types::ZoteroAdvancedSortBy;
use crate::types::ZoteroAnnotation;
use crate::types::ZoteroAnnotationsParams;
use crate::types::ZoteroAttachment;
use crate::types::ZoteroCitationFormat;
use crate::types::ZoteroCitationGenerator;
use crate::types::ZoteroCitationParams;
use crate::types::ZoteroCollectionItemsParams;
use crate::types::ZoteroCollectionsParams;
use crate::types::ZoteroGrepCandidateStrategy;
use crate::types::ZoteroGrepField;
use crate::types::ZoteroGrepMatchMode;
use crate::types::ZoteroGrepParams;
use crate::types::ZoteroItemDetail;
use crate::types::ZoteroItemParams;
use crate::types::ZoteroListGroupsParams;
use crate::types::ZoteroRecentParams;
use crate::types::ZoteroRecentSortBy;
use crate::types::ZoteroSearchCondition;
use crate::types::ZoteroSearchConditionField;
use crate::types::ZoteroSearchConditionOperation;
use crate::types::ZoteroSearchNotesParams;
use crate::types::ZoteroSearchParams;
use crate::types::ZoteroSortDirection;
use crate::types::ZoteroTagSearchParams;
use crate::types::ZoteroTagsParams;

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_and_get_item_use_user_scope() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("q", "diffusion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Diffusion Models in Vision",
                    "creators": [{"firstName": "Alice", "lastName": "Kim"}],
                    "date": "2023-06-01",
                    "DOI": "10.1000/x",
                    "abstractNote": "A long abstract.",
                    "tags": [{"tag": "vision"}, {"tag": "diffusion"}]
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "ITEM1",
            "data": {
                "itemType": "journalArticle",
                "title": "Diffusion Models in Vision",
                "creators": [{"firstName": "Alice", "lastName": "Kim"}],
                "date": "2023-06-01",
                "DOI": "10.1000/x",
                "abstractNote": "A long abstract.",
                "publicationTitle": "NeurIPS",
                "tags": [{"tag": "vision"}, {"tag": "diffusion"}]
            }
        })))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());

    let search = toolkit
        .zotero_search(ZoteroSearchParams {
            query: "diffusion".to_string(),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(20),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search should succeed");

    assert_eq!(search.items.len(), 1);
    assert_eq!(search.items[0].key, "ITEM1");
    assert_eq!(search.items[0].tags, vec!["vision", "diffusion"]);

    let item = toolkit
        .zotero_get_item(ZoteroItemParams {
            item_key: "ITEM1".to_string(),
            library_type: None,
            library_id: None,
            max_chars_per_item: None,
            include_attachments: None,
            include_fulltext_resolution: None,
        })
        .await
        .expect("zotero_get_item should succeed");

    assert_eq!(item.key, "ITEM1");
    assert_eq!(item.publication, Some("NeurIPS".to_string()));
    assert_eq!(item.tags, vec!["vision", "diffusion"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_item_default_keeps_enrichment_fields_none() {
    let server = MockServer::start().await;
    mount_item_detail(
        &server,
        "ITEM1",
        serde_json::json!({
            "itemType": "journalArticle",
            "title": "Default Item",
            "creators": [{"firstName": "Alice", "lastName": "Kim"}],
            "date": "2023-06-01"
        }),
    )
    .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "attachment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(0)
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let item = toolkit
        .zotero_get_item(ZoteroItemParams {
            item_key: "ITEM1".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            max_chars_per_item: None,
            include_attachments: None,
            include_fulltext_resolution: None,
        })
        .await
        .expect("zotero_get_item should succeed");

    assert_eq!(item.attachments, None);
    assert_eq!(item.document_resolution, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_item_enrichment_fetches_attachments_once() {
    let server = MockServer::start().await;
    mount_item_detail(
        &server,
        "ITEM1",
        serde_json::json!({
            "itemType": "journalArticle",
            "title": "Enriched Item",
            "creators": [{"firstName": "Alice", "lastName": "Kim"}],
            "date": "2023-06-01"
        }),
    )
    .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "attachment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ATT1",
                "data": {
                    "itemType": "attachment",
                    "title": "paper.pdf",
                    "filename": "paper.pdf",
                    "contentType": "application/pdf",
                    "url": "https://example.com/paper.pdf",
                    "parentItem": "ITEM1"
                }
            }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let item = toolkit
        .zotero_get_item(ZoteroItemParams {
            item_key: "ITEM1".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            max_chars_per_item: None,
            include_attachments: Some(true),
            include_fulltext_resolution: Some(true),
        })
        .await
        .expect("zotero_get_item should succeed");

    let attachments = item
        .attachments
        .expect("include_attachments=true should return attachments");
    assert_eq!(attachments.len(), 1);
    let resolution = item
        .document_resolution
        .expect("include_fulltext_resolution=true should return resolution");
    assert_eq!(resolution.source_kind, DocumentSourceKind::AttachmentPdfUrl);
    assert_eq!(
        resolution.preferred_url,
        Some("https://example.com/paper.pdf".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_item_enrichment_resolves_arxiv_pdf_item_sources() {
    let server = MockServer::start().await;
    mount_item_detail(
        &server,
        "ITEM1",
        serde_json::json!({
            "itemType": "journalArticle",
            "title": "arXiv Item",
            "creators": [{"firstName": "Alice", "lastName": "Kim"}],
            "date": "2024-01-01",
            "url": "https://arxiv.org/abs/9999.99999v1"
        }),
    )
    .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "attachment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ATT1",
                "data": {
                    "itemType": "attachment",
                    "title": "paper.pdf",
                    "filename": "paper.pdf",
                    "contentType": "application/pdf",
                    "url": "https://example.com/paper.pdf",
                    "parentItem": "ITEM1"
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let item = toolkit
        .zotero_get_item(ZoteroItemParams {
            item_key: "ITEM1".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            max_chars_per_item: None,
            include_attachments: Some(false),
            include_fulltext_resolution: Some(true),
        })
        .await
        .expect("zotero_get_item should resolve arXiv sources");

    let resolution = item
        .document_resolution
        .expect("include_fulltext_resolution=true should return resolution");
    assert_eq!(resolution.source_kind, DocumentSourceKind::ArxivPdf);
    let preferred_url = resolution
        .preferred_url
        .expect("arXiv resolution should include a preferred URL");
    assert!(preferred_url.starts_with("https://arxiv.org/pdf/9999.99999v1.pdf"));
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_item_enrichment_tolerates_attachment_failures() {
    let server = MockServer::start().await;
    mount_item_detail(
        &server,
        "ITEM1",
        serde_json::json!({
            "itemType": "journalArticle",
            "title": "Enriched Item",
            "creators": [{"firstName": "Alice", "lastName": "Kim"}],
            "date": "2023-06-01"
        }),
    )
    .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "attachment"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let item = toolkit
        .zotero_get_item(ZoteroItemParams {
            item_key: "ITEM1".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            max_chars_per_item: None,
            include_attachments: Some(true),
            include_fulltext_resolution: Some(true),
        })
        .await
        .expect("zotero_get_item should still succeed when enrichment fails");

    assert_eq!(item.attachments, None);
    assert_eq!(item.document_resolution, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_item_citation_fallback_bibtex_maps_expected_fields() {
    let server = MockServer::start().await;
    mount_item_detail(
        &server,
        "ITEM1",
        serde_json::json!({
            "itemType": "journalArticle",
            "title": "Diffusion & Vision_101",
            "creators": [{"firstName": "Alice", "lastName": "Kim"}],
            "date": "2023-06-01",
            "DOI": "10.1000/example",
            "publicationTitle": "NeurIPS",
            "url": "https://example.com/paper"
        }),
    )
    .await;

    let toolkit = build_test_toolkit(server.uri());
    let citation = toolkit
        .zotero_get_item_citation(ZoteroCitationParams {
            item_key: "ITEM1".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            format: Some(ZoteroCitationFormat::Bibtex),
        })
        .await
        .expect("zotero_get_item_citation should succeed");

    assert_eq!(
        citation.generator,
        ZoteroCitationGenerator::FallbackFormatter
    );
    assert_eq!(citation.citation_key, Some("kim2023item1".to_string()));
    assert!(citation.citation.contains("@article{kim2023item1,"));
    assert!(
        citation
            .citation
            .contains("title = {Diffusion \\& Vision\\_101},")
    );
    assert!(citation.warnings.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_item_citation_fallback_key_handles_non_latin_authors() {
    let server = MockServer::start().await;
    mount_item_detail(
        &server,
        "ABCD1234",
        serde_json::json!({
            "itemType": "book",
            "title": "跨语言文献",
            "creators": [{"name": "李雷"}]
        }),
    )
    .await;

    let toolkit = build_test_toolkit(server.uri());
    let citation = toolkit
        .zotero_get_item_citation(ZoteroCitationParams {
            item_key: "ABCD1234".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            format: Some(ZoteroCitationFormat::Bibtex),
        })
        .await
        .expect("zotero_get_item_citation should succeed");

    assert_eq!(citation.citation_key, Some("itemabcd1234".to_string()));
    assert!(citation.citation.contains("@book{itemabcd1234,"));
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_item_citation_fallback_csl_json_has_expected_structure() {
    let server = MockServer::start().await;
    mount_item_detail(
        &server,
        "ITEM1",
        serde_json::json!({
            "itemType": "journalArticle",
            "title": "Structured Citation",
            "creators": [
                {"firstName": "Alice", "lastName": "Kim"},
                {"firstName": "Bob", "lastName": "Lee"}
            ],
            "date": "2021-06-01",
            "publicationTitle": "NeurIPS",
            "DOI": "10.1000/example",
            "url": "https://example.com/paper"
        }),
    )
    .await;

    let toolkit = build_test_toolkit(server.uri());
    let citation = toolkit
        .zotero_get_item_citation(ZoteroCitationParams {
            item_key: "ITEM1".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            format: Some(ZoteroCitationFormat::CslJson),
        })
        .await
        .expect("zotero_get_item_citation should succeed");

    assert_eq!(citation.citation_key, None);
    assert_eq!(
        citation.generator,
        ZoteroCitationGenerator::FallbackFormatter
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&citation.citation).expect("citation should be valid JSON");
    assert_eq!(parsed["id"], "ITEM1");
    assert_eq!(parsed["type"], "article-journal");
    assert_eq!(parsed["title"], "Structured Citation");
    assert_eq!(parsed["container-title"], "NeurIPS");
    assert_eq!(parsed["DOI"], "10.1000/example");
    assert_eq!(parsed["URL"], "https://example.com/paper");
    assert_eq!(parsed["issued"]["date-parts"], serde_json::json!([[2021]]));
    assert_eq!(
        parsed["author"],
        serde_json::json!([
            {"literal": "Alice Kim"},
            {"literal": "Bob Lee"}
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_item_citation_fallback_apa_formats_expected_fields() {
    let server = MockServer::start().await;
    mount_item_detail(
        &server,
        "ITEM1",
        serde_json::json!({
            "itemType": "journalArticle",
            "title": "Reliable Evaluation",
            "creators": [{"firstName": "Alice", "lastName": "Kim"}],
            "date": "2024-03-09",
            "publicationTitle": "Journal of Testing",
            "DOI": "10.1000/example"
        }),
    )
    .await;

    let toolkit = build_test_toolkit(server.uri());
    let citation = toolkit
        .zotero_get_item_citation(ZoteroCitationParams {
            item_key: "ITEM1".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            format: Some(ZoteroCitationFormat::Apa),
        })
        .await
        .expect("zotero_get_item_citation should succeed");

    assert_eq!(citation.citation_key, None);
    assert_eq!(
        citation.citation,
        "Alice Kim (2024). Reliable Evaluation. Journal of Testing. https://doi.org/10.1000/example"
    );
}

#[test]
fn fallback_apa_citation_handles_missing_author_and_year() {
    let citation = super::fallback_apa_citation(&ZoteroItemDetail {
        key: "ITEM1".to_string(),
        title: "Missing Metadata".to_string(),
        authors: vec!["   ".to_string(), "".to_string()],
        abstract_text: None,
        date: None,
        doi: None,
        url: None,
        publication: None,
        item_type: "journalArticle".to_string(),
        tags: Vec::new(),
        extra: None,
        source_meta: None,
        attachments: None,
        document_resolution: None,
    });

    assert_eq!(citation, "Unknown author (n.d.). Missing Metadata.");
}

#[test]
fn escape_bibtex_value_escapes_special_characters() {
    assert_eq!(
        super::escape_bibtex_value("&%#_{}~^\\"),
        "\\&\\%\\#\\_\\{\\}\\textasciitilde{}\\textasciicircum{}\\textbackslash{}"
    );
}

#[test]
fn zotero_citation_params_reject_invalid_format() {
    let parsed = serde_json::from_value::<ZoteroCitationParams>(serde_json::json!({
        "item_key": "ITEM1",
        "format": "invalid-format"
    }));
    assert!(parsed.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_fulltext_notes_and_attachments_are_supported() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/fulltext"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": "abcdefghijklmnopqrstuvwxyz"
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "note"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "title": "note title",
                    "note": "This is a very long note body.",
                    "parentItem": "ITEM1"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "attachment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ATT1",
                "data": {
                    "itemType": "attachment",
                    "title": "paper.pdf",
                    "filename": "paper.pdf",
                    "contentType": "application/pdf",
                    "linkMode": "imported_file",
                    "url": "https://example.com/paper.pdf",
                    "parentItem": "ITEM1"
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());

    let fulltext = toolkit
        .zotero_get_fulltext(ZoteroItemParams {
            item_key: "ITEM1".to_string(),
            library_type: None,
            library_id: None,
            max_chars_per_item: Some(5),
            include_attachments: None,
            include_fulltext_resolution: None,
        })
        .await
        .expect("zotero_get_fulltext should succeed");
    assert_eq!(fulltext.content, "ab...");
    let resolution = fulltext
        .resolution
        .expect("zotero_get_fulltext should include resolution");
    assert_eq!(resolution.source_kind, DocumentSourceKind::AttachmentPdfUrl);
    assert_eq!(
        resolution.preferred_url,
        Some("https://example.com/paper.pdf".to_string())
    );
    assert_eq!(resolution.fallback_urls, Vec::<String>::new());
    assert_eq!(resolution.local_path, None);
    assert!(
        resolution
            .trace
            .iter()
            .any(|entry| entry.contains("item metadata lookup failed")),
        "expected metadata lookup warning in trace: {:?}",
        resolution.trace
    );

    let notes = toolkit
        .zotero_get_notes(ZoteroItemParams {
            item_key: "ITEM1".to_string(),
            library_type: None,
            library_id: None,
            max_chars_per_item: Some(4),
            include_attachments: None,
            include_fulltext_resolution: None,
        })
        .await
        .expect("zotero_get_notes should succeed");
    assert_eq!(notes.notes.len(), 1);
    assert_eq!(notes.notes[0].title, Some("n...".to_string()));

    let attachments = toolkit
        .zotero_get_attachments(ZoteroItemParams {
            item_key: "ITEM1".to_string(),
            library_type: None,
            library_id: None,
            max_chars_per_item: Some(5),
            include_attachments: None,
            include_fulltext_resolution: None,
        })
        .await
        .expect("zotero_get_attachments should succeed");
    assert_eq!(attachments.attachments.len(), 1);
    assert_eq!(attachments.attachments[0].title, Some("pa...".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_annotations_item_scoped_parses_annotation_fields() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "annotation"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "10"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Total-Results", "1")
                .set_body_json(serde_json::json!([
                    {
                        "key": "ANNO1",
                        "data": {
                            "itemType": "annotation",
                            "parentItem": "ITEM1",
                            "annotationType": "highlight",
                            "annotationText": "<p>Important <b>finding</b></p>",
                            "annotationComment": "<div>See <i>section 2</i></div>",
                            "annotationColor": "#ffd400",
                            "annotationPageLabel": "12",
                            "annotationSortIndex": "00012|000001|00000"
                        }
                    }
                ])),
        )
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let mut result = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: Some("ITEM1".to_string()),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            include_parent_context: Some(false),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_annotations should succeed");

    for annotation in &mut result.annotations {
        annotation.source_meta = None;
    }

    assert_eq!(result.item_key, Some("ITEM1".to_string()));
    assert_eq!(result.total_available, Some(1));
    assert_eq!(result.has_more, false);
    assert_eq!(
        result.annotations,
        vec![ZoteroAnnotation {
            key: "ANNO1".to_string(),
            parent_item: Some("ITEM1".to_string()),
            annotation_type: "highlight".to_string(),
            annotation_text: Some("Important finding".to_string()),
            annotation_comment: Some("See section 2".to_string()),
            annotation_color: Some("#ffd400".to_string()),
            annotation_page_label: Some("12".to_string()),
            annotation_sort_index: Some("00012|000001|00000".to_string()),
            parent_item_title: None,
            source_meta: None,
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_annotations_library_scope_exposes_pagination_headers() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("itemType", "annotation"))
        .and(query_param("start", "1"))
        .and(query_param("limit", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Total-Results", "3")
                .set_body_json(serde_json::json!([
                    {
                        "key": "ANNO2",
                        "data": {
                            "itemType": "annotation",
                            "parentItem": "ITEM2",
                            "annotationType": "underline",
                            "annotationText": "alpha"
                        }
                    }
                ])),
        )
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let mut result = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: None,
            library_type: None,
            library_id: None,
            offset: Some(1),
            limit: Some(1),
            include_parent_context: Some(false),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_annotations should succeed");

    for annotation in &mut result.annotations {
        annotation.source_meta = None;
    }

    assert_eq!(result.item_key, None);
    assert_eq!(result.total_available, Some(3));
    assert_eq!(result.has_more, true);
    assert_eq!(
        result.annotations,
        vec![ZoteroAnnotation {
            key: "ANNO2".to_string(),
            parent_item: Some("ITEM2".to_string()),
            annotation_type: "underline".to_string(),
            annotation_text: Some("alpha".to_string()),
            annotation_comment: None,
            annotation_color: None,
            annotation_page_label: None,
            annotation_sort_index: None,
            parent_item_title: None,
            source_meta: None,
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_annotations_preserves_extended_annotation_types() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("itemType", "annotation"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ANNO_STRIKE",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "ITEM1",
                    "annotationType": "strikethrough",
                    "annotationText": "x"
                }
            },
            {
                "key": "ANNO_INK",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "ITEM1",
                    "annotationType": "ink",
                    "annotationComment": "pen"
                }
            },
            {
                "key": "ANNO_UNKNOWN",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "ITEM1",
                    "annotationType": "weird-new-type",
                    "annotationComment": "fallback"
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let mut result = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: None,
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            include_parent_context: Some(false),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_annotations should succeed");

    for annotation in &mut result.annotations {
        annotation.source_meta = None;
    }

    assert_eq!(
        result
            .annotations
            .iter()
            .map(|annotation| annotation.annotation_type.clone())
            .collect::<Vec<_>>(),
        vec![
            "strikethrough".to_string(),
            "ink".to_string(),
            "unknown".to_string()
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_annotations_returns_empty_list_when_item_has_no_annotations() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM_EMPTY/children"))
        .and(query_param("itemType", "annotation"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "50"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Total-Results", "0")
                .set_body_json(serde_json::json!([])),
        )
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: Some("ITEM_EMPTY".to_string()),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(50),
            include_parent_context: Some(false),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_annotations should succeed");

    assert_eq!(result.item_key, Some("ITEM_EMPTY".to_string()));
    assert_eq!(result.total_available, Some(0));
    assert_eq!(result.has_more, false);
    assert_eq!(result.annotations, Vec::<ZoteroAnnotation>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_annotations_can_enrich_parent_item_title_and_cache_result() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "annotation"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ANNO3",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "PARENT1",
                    "annotationType": "note",
                    "annotationComment": "memo"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/PARENT1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "PARENT1",
            "data": {
                "itemType": "journalArticle",
                "title": "Parent Item Title",
                "creators": []
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let mut first = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: Some("ITEM1".to_string()),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(50),
            include_parent_context: Some(true),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_annotations should succeed");

    let mut second = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: Some("ITEM1".to_string()),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(50),
            include_parent_context: Some(true),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_annotations should succeed");

    for annotation in &mut first.annotations {
        annotation.source_meta = None;
    }
    for annotation in &mut second.annotations {
        annotation.source_meta = None;
    }

    assert_eq!(
        first.annotations,
        vec![ZoteroAnnotation {
            key: "ANNO3".to_string(),
            parent_item: Some("PARENT1".to_string()),
            annotation_type: "note".to_string(),
            annotation_text: None,
            annotation_comment: Some("memo".to_string()),
            annotation_color: None,
            annotation_page_label: None,
            annotation_sort_index: None,
            parent_item_title: Some("Parent Item Title".to_string()),
            source_meta: None,
        }]
    );
    assert_eq!(first.annotations, second.annotations);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_annotations_parent_enrichment_tolerates_partial_failures() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "annotation"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ANNO_OK",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "PARENT_OK",
                    "annotationType": "highlight",
                    "annotationText": "keep"
                }
            },
            {
                "key": "ANNO_MISSING",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "PARENT_MISSING",
                    "annotationType": "note",
                    "annotationComment": "still return"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/PARENT_OK"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "PARENT_OK",
            "data": {
                "itemType": "journalArticle",
                "title": "Resolvable Parent",
                "creators": []
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/PARENT_MISSING"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let mut result = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: Some("ITEM1".to_string()),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(50),
            include_parent_context: Some(true),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_annotations should succeed");

    for annotation in &mut result.annotations {
        annotation.source_meta = None;
    }

    assert_eq!(
        result.annotations,
        vec![
            ZoteroAnnotation {
                key: "ANNO_OK".to_string(),
                parent_item: Some("PARENT_OK".to_string()),
                annotation_type: "highlight".to_string(),
                annotation_text: Some("keep".to_string()),
                annotation_comment: None,
                annotation_color: None,
                annotation_page_label: None,
                annotation_sort_index: None,
                parent_item_title: Some("Resolvable Parent".to_string()),
                source_meta: None,
            },
            ZoteroAnnotation {
                key: "ANNO_MISSING".to_string(),
                parent_item: Some("PARENT_MISSING".to_string()),
                annotation_type: "note".to_string(),
                annotation_text: None,
                annotation_comment: Some("still return".to_string()),
                annotation_color: None,
                annotation_page_label: None,
                annotation_sort_index: None,
                parent_item_title: None,
                source_meta: None,
            }
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_annotations_library_scope_can_enrich_parent_context() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("itemType", "annotation"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ANNO_LIB",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "PARENT_LIB",
                    "annotationType": "note",
                    "annotationComment": "library scoped"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/PARENT_LIB"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "PARENT_LIB",
            "data": {
                "itemType": "journalArticle",
                "title": "Library Parent",
                "creators": []
            }
        })))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let mut result = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: None,
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(50),
            include_parent_context: Some(true),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_annotations should succeed");

    for annotation in &mut result.annotations {
        annotation.source_meta = None;
    }

    assert_eq!(
        result.annotations,
        vec![ZoteroAnnotation {
            key: "ANNO_LIB".to_string(),
            parent_item: Some("PARENT_LIB".to_string()),
            annotation_type: "note".to_string(),
            annotation_text: None,
            annotation_comment: Some("library scoped".to_string()),
            annotation_color: None,
            annotation_page_label: None,
            annotation_sort_index: None,
            parent_item_title: Some("Library Parent".to_string()),
            source_meta: None,
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_annotations_truncates_text_and_comment_after_html_strip() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "annotation"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ANNO4",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "ITEM1",
                    "annotationType": "highlight",
                    "annotationText": "<p>abcdefghi</p>",
                    "annotationComment": "<div>123456789</div>"
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let mut result = toolkit
        .zotero_get_annotations(ZoteroAnnotationsParams {
            item_key: Some("ITEM1".to_string()),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(50),
            include_parent_context: Some(false),
            max_chars_per_item: Some(5),
        })
        .await
        .expect("zotero_get_annotations should succeed");

    for annotation in &mut result.annotations {
        annotation.source_meta = None;
    }

    assert_eq!(
        result.annotations,
        vec![ZoteroAnnotation {
            key: "ANNO4".to_string(),
            parent_item: Some("ITEM1".to_string()),
            annotation_type: "highlight".to_string(),
            annotation_text: Some("ab...".to_string()),
            annotation_comment: Some("12...".to_string()),
            annotation_color: None,
            annotation_page_label: None,
            annotation_sort_index: None,
            parent_item_title: None,
            source_meta: None,
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_by_tag_requires_all_tags() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("tag", "ml"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Item One",
                    "creators": [],
                    "tags": [{"tag": "ml"}, {"tag": "vision"}]
                }
            },
            {
                "key": "ITEM2",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Item Two",
                    "creators": [],
                    "tags": [{"tag": "ml"}]
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("tag", "vision"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Item One",
                    "creators": [],
                    "tags": [{"tag": "ml"}, {"tag": "vision"}]
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());

    let result = toolkit
        .zotero_search_by_tag(ZoteroTagSearchParams {
            tags: vec!["ml".to_string(), "vision".to_string()],
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search_by_tag should succeed");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].key, "ITEM1");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_by_tag_dedups_tags_case_insensitively() {
    let server = MockServer::start().await;

    let tag_result = serde_json::json!([
        {
            "key": "ITEM1",
            "data": {
                "itemType": "journalArticle",
                "title": "Item One",
                "creators": [],
                "tags": [{"tag": "ml"}]
            }
        }
    ]);

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("tag", "ML"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tag_result.clone()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("tag", "ml"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tag_result))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());

    let result = toolkit
        .zotero_search_by_tag(ZoteroTagSearchParams {
            tags: vec!["ML".to_string(), "ml".to_string()],
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search_by_tag should succeed");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].key, "ITEM1");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_by_tag_paginates_each_tag_before_intersection() {
    let server = MockServer::start().await;

    let first_ml_page = (0..100)
        .map(|idx| {
            serde_json::json!({
                "key": format!("ML{idx:03}"),
                "data": {
                    "itemType": "journalArticle",
                    "title": format!("ML Item {idx}"),
                    "creators": [],
                    "tags": [{"tag": "ml"}]
                }
            })
        })
        .collect::<Vec<_>>();

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("tag", "ml"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(first_ml_page))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("tag", "ml"))
        .and(query_param("start", "100"))
        .and(query_param("limit", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "TARGET",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Target Item",
                    "creators": [],
                    "tags": [{"tag": "ml"}, {"tag": "vision"}]
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("tag", "vision"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "TARGET",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Target Item",
                    "creators": [],
                    "tags": [{"tag": "ml"}, {"tag": "vision"}]
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());

    let result = toolkit
        .zotero_search_by_tag(ZoteroTagSearchParams {
            tags: vec!["ml".to_string(), "vision".to_string()],
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search_by_tag should succeed");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].key, "TARGET");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_tags_uses_tags_endpoint_and_link_header_for_pagination() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/tags"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(
                    "Link",
                    r#"<https://api.zotero.org/users/123/tags?start=100&limit=100>; rel="next""#,
                )
                .set_body_json(serde_json::json!([
                    { "tag": "ml" },
                    { "tag": "vision" },
                    { "tag": "   " }
                ])),
        )
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_get_tags(ZoteroTagsParams {
            library_type: None,
            library_id: None,
            offset: None,
            limit: None,
        })
        .await
        .expect("zotero_get_tags should succeed");

    assert_eq!(result.tags, vec!["ml".to_string(), "vision".to_string()]);
    assert_eq!(result.has_more, true);
    assert_eq!(result.total_available, 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_tags_dedups_case_insensitively_and_uses_raw_page_for_has_more() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/tags"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "tag": "ml" },
            { "tag": "ML" },
            { "tag": "ml" }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_get_tags(ZoteroTagsParams {
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(3),
        })
        .await
        .expect("zotero_get_tags should succeed");

    assert_eq!(result.tags, vec!["ml".to_string()]);
    assert_eq!(result.has_more, true);
    assert_eq!(result.total_available, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_recent_defaults_to_date_added_descending() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("sort", "dateAdded"))
        .and(query_param("direction", "desc"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "25"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "RECENT1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Recently Added",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_get_recent(ZoteroRecentParams {
            library_type: None,
            library_id: None,
            offset: None,
            limit: None,
            item_type: None,
            sort_by: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_recent should succeed");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].key, "RECENT1");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_get_recent_supports_item_type_and_date_modified_sort() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("itemType", "journalArticle"))
        .and(query_param("sort", "dateModified"))
        .and(query_param("direction", "desc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "RECENT2",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Recently Modified",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_get_recent(ZoteroRecentParams {
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(25),
            item_type: Some("journalArticle".to_string()),
            sort_by: Some(ZoteroRecentSortBy::DateModified),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_recent should succeed");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].key, "RECENT2");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_tags_and_recent_limits_are_clamped() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/tags"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "200"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("sort", "dateAdded"))
        .and(query_param("direction", "desc"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());

    let tags = toolkit
        .zotero_get_tags(ZoteroTagsParams {
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(500),
        })
        .await
        .expect("zotero_get_tags should clamp limit");
    assert_eq!(tags.tags, Vec::<String>::new());

    let recent = toolkit
        .zotero_get_recent(ZoteroRecentParams {
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(0),
            item_type: None,
            sort_by: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_recent should clamp limit");
    assert_eq!(recent.items.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_collections_support_group_scope_override() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/groups/999/collections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "COL1",
                "data": {"name": "Important"}
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/groups/999/collections/COL1/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Group Item",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());

    let collections = toolkit
        .zotero_get_collections(ZoteroCollectionsParams {
            library_type: Some("group".to_string()),
            library_id: Some("999".to_string()),
            offset: Some(0),
            limit: Some(20),
        })
        .await
        .expect("zotero_get_collections should succeed");

    assert_eq!(collections.collections.len(), 1);
    assert_eq!(collections.collections[0].name, "Important");

    let items = toolkit
        .zotero_get_collection_items(ZoteroCollectionItemsParams {
            collection_key: "COL1".to_string(),
            library_type: Some("group".to_string()),
            library_id: Some("999".to_string()),
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_get_collection_items should succeed");

    assert_eq!(items.items.len(), 1);
    assert_eq!(items.items[0].key, "ITEM1");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_list_groups_uses_local_user_zero_in_local_mode() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/users/0/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 999,
                "data": {
                    "name": "Research Lab",
                    "description": "Shared papers"
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit_with_config(ResearchConfig {
        zotero_api_key: None,
        zotero_user_id: None,
        zotero_group_id: None,
        zotero_base_url: format!("{}/api/", server.uri()),
        ..ResearchConfig::default()
    });

    let result = toolkit
        .zotero_list_groups(ZoteroListGroupsParams {
            user_id: None,
            offset: Some(0),
            limit: Some(20),
        })
        .await
        .expect("local zotero list groups should succeed without API key");

    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].id, "999");
    assert_eq!(result.groups[0].name, "Research Lab");
    assert_eq!(
        result.groups[0].description,
        Some("Shared papers".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_local_mode_defaults_to_user_zero_without_api_key() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/users/0/items"))
        .and(query_param("q", "diffusion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM_LOCAL",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Local Zotero Item",
                    "creators": [],
                    "tags": [{"tag": "local"}]
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit_with_config(ResearchConfig {
        zotero_api_key: None,
        zotero_user_id: None,
        zotero_group_id: None,
        zotero_base_url: format!("{}/api/", server.uri()),
        ..ResearchConfig::default()
    });

    let result = toolkit
        .zotero_search(ZoteroSearchParams {
            query: "diffusion".to_string(),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("local zotero search should succeed without API key");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].key, "ITEM_LOCAL");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_advanced_search_uses_collector_for_note_conditions() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("sort", "dateModified"))
        .and(query_param("direction", "desc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Item One",
                    "creators": []
                }
            },
            {
                "key": "ITEM2",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Item Two",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "note"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "note": "<p>Contains target keyword</p>",
                    "parentItem": "ITEM1"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM2/children"))
        .and(query_param("itemType", "note"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE2",
                "data": {
                    "itemType": "note",
                    "note": "<p>No signal here</p>",
                    "parentItem": "ITEM2"
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_advanced_search(ZoteroAdvancedSearchParams {
            conditions: vec![ZoteroSearchCondition {
                field: ZoteroSearchConditionField::Note,
                operation: ZoteroSearchConditionOperation::Contains,
                value: Some("target keyword".to_string()),
                case_sensitive: Some(false),
            }],
            join_mode: None,
            sort_by: None,
            sort_direction: None,
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_advanced_search should succeed");

    assert_eq!(
        result.candidate_strategy,
        ZoteroAdvancedCandidateStrategy::RecentModifiedFallback
    );
    assert_eq!(result.completeness, ZoteroAdvancedCompleteness::Approximate);
    assert_eq!(result.scanned_items, 2);
    assert_eq!(result.results.items.len(), 1);
    assert_eq!(result.results.items[0].key, "ITEM1");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_advanced_search_warns_on_default_fulltext_cap() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("sort", "dateModified"))
        .and(query_param("direction", "desc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Item One",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/fulltext"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": "fulltext signal appears here"
        })))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_advanced_search(ZoteroAdvancedSearchParams {
            conditions: vec![ZoteroSearchCondition {
                field: ZoteroSearchConditionField::Fulltext,
                operation: ZoteroSearchConditionOperation::Contains,
                value: Some("signal".to_string()),
                case_sensitive: Some(false),
            }],
            join_mode: None,
            sort_by: None,
            sort_direction: None,
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_advanced_search should succeed");

    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("default 10000 character cap"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_advanced_search_does_not_emit_empty_hints_for_pagination_gap() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("sort", "dateModified"))
        .and(query_param("direction", "desc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Transformer Models",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_advanced_search(ZoteroAdvancedSearchParams {
            conditions: vec![ZoteroSearchCondition {
                field: ZoteroSearchConditionField::Title,
                operation: ZoteroSearchConditionOperation::Contains,
                value: Some("Transformer".to_string()),
                case_sensitive: Some(false),
            }],
            join_mode: None,
            sort_by: None,
            sort_direction: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            offset: Some(10),
            limit: Some(5),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_advanced_search should succeed");

    assert_eq!(result.results.items.len(), 0);
    assert_eq!(result.results.total_available, Some(1));
    assert_eq!(result.results.has_more, false);
    assert_eq!(result.hints, Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_advanced_search_uses_server_filtered_strategy_for_tag_equals() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("tag", "ml"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "ML Systems",
                    "creators": [],
                    "tags": [{ "tag": "ml" }]
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_advanced_search(ZoteroAdvancedSearchParams {
            conditions: vec![ZoteroSearchCondition {
                field: ZoteroSearchConditionField::Tag,
                operation: ZoteroSearchConditionOperation::Equals,
                value: Some("ml".to_string()),
                case_sensitive: Some(false),
            }],
            join_mode: None,
            sort_by: None,
            sort_direction: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_advanced_search should succeed");

    assert_eq!(
        result.candidate_strategy,
        ZoteroAdvancedCandidateStrategy::ServerFiltered
    );
    assert_eq!(result.completeness, ZoteroAdvancedCompleteness::Exact);
    assert_eq!(result.scanned_items, 1);
    assert_eq!(result.results.items.len(), 1);
    assert_eq!(result.results.items[0].key, "ITEM1");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_grep_text_matches_title_literal() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("sort", "dateModified"))
        .and(query_param("direction", "desc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Diffusion Models for Vision",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_grep_text(ZoteroGrepParams {
            pattern: "diffusion".to_string(),
            match_mode: Some(ZoteroGrepMatchMode::Literal),
            case_sensitive: Some(false),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            parent_item_key: None,
            query_hint: None,
            item_type: None,
            fields: Some(vec![ZoteroGrepField::Title]),
            limit_items: Some(10),
            limit_matches: Some(5),
            max_matches_per_item: Some(5),
            context_chars: Some(50),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_grep_text should succeed");

    assert_eq!(result.returned_matches, 1);
    assert_eq!(result.matches[0].item_key, "ITEM1");
    assert_eq!(result.matches[0].field, "title");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_notes_uses_parent_scoped_grep_adapter() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/PARENT1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "title": "Note One",
                    "parentItem": "PARENT1"
                }
            },
            {
                "key": "ANN1",
                "data": {
                    "itemType": "annotation",
                    "title": "Annotation One",
                    "parentItem": "PARENT1"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/NOTE1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "NOTE1",
            "data": {
                "itemType": "note",
                "note": "<p>Contains target phrase</p>",
                "parentItem": "PARENT1"
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ANN1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "ANN1",
            "data": {
                "itemType": "annotation",
                "parentItem": "PARENT1",
                "annotationType": "highlight",
                "annotationText": "<p>Target evidence</p>"
            }
        })))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_search_notes(ZoteroSearchNotesParams {
            query: "target".to_string(),
            match_mode: None,
            case_sensitive: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            parent_item_key: Some("PARENT1".to_string()),
            include_annotations: Some(true),
            limit: Some(10),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search_notes should succeed");

    assert_eq!(result.query, "target");
    assert_eq!(
        result.candidate_strategy,
        ZoteroGrepCandidateStrategy::ParentScoped
    );
    assert_eq!(result.scanned_items, 2);
    assert_eq!(result.notes.len(), 2);
    assert_eq!(result.total_available, Some(2));
    assert_eq!(result.has_more, false);
    assert_eq!(result.hints, Vec::<String>::new());
    assert_eq!(
        result
            .notes
            .iter()
            .map(|note| note.parent_item.clone())
            .collect::<Vec<_>>(),
        vec![Some("PARENT1".to_string()), Some("PARENT1".to_string())]
    );
    assert!(
        result
            .notes
            .iter()
            .any(|note| note.field == "note" && note.item_key == "NOTE1")
    );
    assert!(
        result
            .notes
            .iter()
            .any(|note| note.field == "annotation_text" && note.item_key == "ANN1")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_notes_reports_note_origin_for_library_scoped_matches() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("q", "target"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Parent Item",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "note"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "parentItem": "ITEM1",
                    "note": "<p>Target in note body</p>"
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_search_notes(ZoteroSearchNotesParams {
            query: "target".to_string(),
            match_mode: None,
            case_sensitive: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            parent_item_key: None,
            include_annotations: Some(false),
            limit: Some(10),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search_notes should succeed");

    assert_eq!(
        result.candidate_strategy,
        ZoteroGrepCandidateStrategy::QueryFiltered
    );
    assert_eq!(result.scanned_items, 1);
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.total_available, Some(1));
    assert_eq!(result.has_more, false);
    assert_eq!(result.hints, Vec::<String>::new());
    assert_eq!(result.notes[0].item_key, "NOTE1");
    assert_eq!(result.notes[0].parent_item.as_deref(), Some("ITEM1"));
    assert_eq!(result.notes[0].field, "note");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_notes_regex_mode_disables_query_prefilter_hint() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("sort", "dateModified"))
        .and(query_param("direction", "desc"))
        .and(query_param("start", "0"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "title": "Note One",
                    "parentItem": "PARENT1"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/NOTE1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "NOTE1",
            "data": {
                "itemType": "note",
                "note": "<p>Cell cycle evidence</p>",
                "parentItem": "PARENT1"
            }
        })))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_search_notes(ZoteroSearchNotesParams {
            query: r"\bcell\b".to_string(),
            match_mode: Some(ZoteroGrepMatchMode::Regex),
            case_sensitive: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            parent_item_key: None,
            include_annotations: Some(false),
            limit: Some(10),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search_notes should succeed");

    assert_eq!(
        result.candidate_strategy,
        ZoteroGrepCandidateStrategy::RecentModified
    );
    assert_eq!(result.scanned_items, 1);
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].item_key, "NOTE1");
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_notes_parent_scope_respects_include_annotations_false() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/PARENT1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "title": "Note One",
                    "parentItem": "PARENT1"
                }
            },
            {
                "key": "ANN1",
                "data": {
                    "itemType": "annotation",
                    "title": "Annotation One",
                    "parentItem": "PARENT1"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/NOTE1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "NOTE1",
            "data": {
                "itemType": "note",
                "note": "<p>No keyword here</p>",
                "parentItem": "PARENT1"
            }
        })))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_search_notes(ZoteroSearchNotesParams {
            query: "target".to_string(),
            match_mode: None,
            case_sensitive: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            parent_item_key: Some("PARENT1".to_string()),
            include_annotations: Some(false),
            limit: Some(10),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search_notes should succeed");

    assert_eq!(
        result.candidate_strategy,
        ZoteroGrepCandidateStrategy::ParentScoped
    );
    assert_eq!(result.scanned_items, 2);
    assert_eq!(result.notes, Vec::new());
    assert_eq!(result.total_available, Some(0));
    assert_eq!(result.has_more, false);
    assert!(!result.hints.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_notes_respects_max_chars_per_item_bounds() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("q", "needle"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Parent Item",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/ITEM1/children"))
        .and(query_param("itemType", "note"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "parentItem": "ITEM1",
                    "note": "<p>aaaaaaaaaaaaaaa needle</p>"
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_search_notes(ZoteroSearchNotesParams {
            query: "needle".to_string(),
            match_mode: None,
            case_sensitive: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            parent_item_key: None,
            include_annotations: Some(false),
            limit: Some(10),
            max_chars_per_item: Some(10),
        })
        .await
        .expect("zotero_search_notes should succeed");

    assert_eq!(
        result.candidate_strategy,
        ZoteroGrepCandidateStrategy::QueryFiltered
    );
    assert_eq!(result.scanned_items, 1);
    assert_eq!(result.notes, Vec::new());
    assert_eq!(result.total_available, Some(0));
    assert_eq!(result.has_more, false);
    assert!(!result.hints.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_notes_limit_sets_has_more_and_total_available() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/PARENT1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "title": "Note One",
                    "parentItem": "PARENT1"
                }
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items/NOTE1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "NOTE1",
            "data": {
                "itemType": "note",
                "note": "<p>target alpha target beta target gamma</p>",
                "parentItem": "PARENT1"
            }
        })))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_search_notes(ZoteroSearchNotesParams {
            query: "target".to_string(),
            match_mode: None,
            case_sensitive: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            parent_item_key: Some("PARENT1".to_string()),
            include_annotations: Some(false),
            limit: Some(2),
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_search_notes should succeed");

    assert_eq!(result.notes.len(), 2);
    assert_eq!(result.total_available, None);
    assert_eq!(result.has_more, true);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_search_notes_rejects_empty_query() {
    let toolkit = build_test_toolkit("http://127.0.0.1".to_string());
    let err = toolkit
        .zotero_search_notes(ZoteroSearchNotesParams {
            query: "   ".to_string(),
            match_mode: None,
            case_sensitive: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            parent_item_key: None,
            include_annotations: Some(true),
            limit: Some(10),
            max_chars_per_item: None,
        })
        .await
        .expect_err("empty queries should be rejected");

    assert!(
        matches!(err, ResearchError::InvalidInput(message) if message.contains("must not be empty"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_advanced_search_multi_scope_resorts_and_computes_pagination_metadata() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Total-Results", "2")
                .set_body_json(serde_json::json!([
                    {
                        "key": "USER_2020",
                        "data": {
                            "itemType": "journalArticle",
                            "title": "User 2020",
                            "creators": [],
                            "date": "2020"
                        }
                    },
                    {
                        "key": "USER_2023",
                        "data": {
                            "itemType": "journalArticle",
                            "title": "User 2023",
                            "creators": [],
                            "date": "2023"
                        }
                    }
                ])),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/groups/456/items"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Total-Results", "2")
                .set_body_json(serde_json::json!([
                    {
                        "key": "GROUP_2019",
                        "data": {
                            "itemType": "journalArticle",
                            "title": "Group 2019",
                            "creators": [],
                            "date": "2019"
                        }
                    },
                    {
                        "key": "GROUP_2021",
                        "data": {
                            "itemType": "journalArticle",
                            "title": "Group 2021",
                            "creators": [],
                            "date": "2021"
                        }
                    }
                ])),
        )
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_advanced_search(ZoteroAdvancedSearchParams {
            conditions: vec![ZoteroSearchCondition {
                field: ZoteroSearchConditionField::ItemType,
                operation: ZoteroSearchConditionOperation::IsNotEmpty,
                value: None,
                case_sensitive: None,
            }],
            join_mode: None,
            sort_by: Some(ZoteroAdvancedSortBy::Year),
            sort_direction: Some(ZoteroSortDirection::Asc),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(3),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_advanced_search should succeed");

    assert_eq!(
        result
            .results
            .items
            .iter()
            .map(|item| item.key.clone())
            .collect::<Vec<_>>(),
        vec![
            "GROUP_2019".to_string(),
            "USER_2020".to_string(),
            "GROUP_2021".to_string()
        ]
    );
    assert_eq!(result.results.total_available, Some(4));
    assert_eq!(result.results.has_more, true);
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_advanced_search_multi_scope_surfaces_scope_errors_in_warnings() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/123/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/groups/456/items"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Total-Results", "1")
                .set_body_json(serde_json::json!([
                    {
                        "key": "GROUP_1",
                        "data": {
                            "itemType": "journalArticle",
                            "title": "Group Item",
                            "creators": [],
                            "date": "2021"
                        }
                    }
                ])),
        )
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());
    let result = toolkit
        .zotero_advanced_search(ZoteroAdvancedSearchParams {
            conditions: vec![ZoteroSearchCondition {
                field: ZoteroSearchConditionField::ItemType,
                operation: ZoteroSearchConditionOperation::IsNotEmpty,
                value: None,
                case_sensitive: None,
            }],
            join_mode: None,
            sort_by: None,
            sort_direction: None,
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("zotero_advanced_search should succeed");

    assert_eq!(result.results.items.len(), 1);
    assert_eq!(result.results.items[0].key, "GROUP_1");
    assert_eq!(result.completeness, ZoteroAdvancedCompleteness::Approximate);
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("advanced search failed for scope user/123"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zotero_advanced_search_rejects_conflicting_item_type_filters() {
    let server = MockServer::start().await;
    let toolkit = build_test_toolkit(server.uri());

    let error = toolkit
        .zotero_advanced_search(ZoteroAdvancedSearchParams {
            conditions: vec![ZoteroSearchCondition {
                field: ZoteroSearchConditionField::ItemType,
                operation: ZoteroSearchConditionOperation::Equals,
                value: Some("journalArticle".to_string()),
                case_sensitive: Some(false),
            }],
            join_mode: None,
            sort_by: None,
            sort_direction: None,
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            offset: Some(0),
            limit: Some(10),
            item_type: Some("book".to_string()),
            max_chars_per_item: None,
        })
        .await
        .expect_err("conflicting item_type filters should fail");

    assert!(matches!(error, ResearchError::InvalidInput(_)));
    assert!(error.to_string().contains("conflicts"));
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_scope_search_merges_user_and_group_results() {
    let server = MockServer::start().await;

    // User scope returns one item.
    Mock::given(method("GET"))
        .and(path("/users/42/items"))
        .and(query_param("q", "robots"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "U1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "User Robot Paper",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    // Groups discovery returns group 999.
    Mock::given(method("GET"))
        .and(path("/users/42/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 999, "data": { "name": "Lab" } }
        ])))
        .mount(&server)
        .await;

    // Group scope returns a different item.
    Mock::given(method("GET"))
        .and(path("/groups/999/items"))
        .and(query_param("q", "robots"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "G1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Group Robot Paper",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    // No explicit library_type/library_id -> multi-scope discovery.
    let toolkit = build_test_toolkit_with_config(ResearchConfig {
        zotero_api_key: Some("test-key".to_string()),
        zotero_user_id: Some("42".to_string()),
        zotero_group_id: None,
        zotero_base_url: server.uri(),
        ..ResearchConfig::default()
    });

    let result = toolkit
        .zotero_search(ZoteroSearchParams {
            query: "robots".to_string(),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(20),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("multi-scope search should succeed");

    assert_eq!(result.items.len(), 2);
    let keys: Vec<_> = result.items.iter().map(|i| i.key.as_str()).collect();
    assert!(keys.contains(&"U1"));
    assert!(keys.contains(&"G1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_scope_get_item_finds_item_in_group() {
    let server = MockServer::start().await;

    // User scope returns 404.
    Mock::given(method("GET"))
        .and(path("/users/42/items/GRP_ITEM"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    // Groups discovery returns group 999.
    Mock::given(method("GET"))
        .and(path("/users/42/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 999, "data": { "name": "Lab" } }
        ])))
        .mount(&server)
        .await;

    // Group scope has the item.
    Mock::given(method("GET"))
        .and(path("/groups/999/items/GRP_ITEM"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "GRP_ITEM",
            "data": {
                "itemType": "journalArticle",
                "title": "Group Only Paper",
                "creators": [{"firstName": "Bob", "lastName": "Z"}],
                "tags": []
            }
        })))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit_with_config(ResearchConfig {
        zotero_api_key: Some("test-key".to_string()),
        zotero_user_id: Some("42".to_string()),
        zotero_group_id: None,
        zotero_base_url: server.uri(),
        ..ResearchConfig::default()
    });

    let item = toolkit
        .zotero_get_item(ZoteroItemParams {
            item_key: "GRP_ITEM".to_string(),
            library_type: None,
            library_id: None,
            max_chars_per_item: None,
            include_attachments: None,
            include_fulltext_resolution: None,
        })
        .await
        .expect("should find item in group scope");

    assert_eq!(item.key, "GRP_ITEM");
    assert_eq!(item.title, "Group Only Paper");
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_scope_queries_only_that_scope() {
    let server = MockServer::start().await;

    // Only mount user scope mock.
    Mock::given(method("GET"))
        .and(path("/users/123/items"))
        .and(query_param("q", "test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Explicit User Item",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit(server.uri());

    // Explicit library_type="user" should NOT trigger group discovery.
    let result = toolkit
        .zotero_search(ZoteroSearchParams {
            query: "test".to_string(),
            library_type: Some("user".to_string()),
            library_id: Some("123".to_string()),
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("explicit scope search should succeed");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].key, "ITEM1");
}

#[tokio::test(flavor = "multi_thread")]
async fn group_discovery_failure_falls_back_to_configured_scopes() {
    let server = MockServer::start().await;

    // User scope search works.
    Mock::given(method("GET"))
        .and(path("/users/42/items"))
        .and(query_param("q", "fallback"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "key": "FB1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Fallback Item",
                    "creators": []
                }
            }
        ])))
        .mount(&server)
        .await;

    // No groups mock -> discovery fails silently.
    // Configured group 777 also has no items mock -> skipped.

    let toolkit = build_test_toolkit_with_config(ResearchConfig {
        zotero_api_key: Some("test-key".to_string()),
        zotero_user_id: Some("42".to_string()),
        zotero_group_id: Some("777".to_string()),
        zotero_base_url: server.uri(),
        ..ResearchConfig::default()
    });

    let result = toolkit
        .zotero_search(ZoteroSearchParams {
            query: "fallback".to_string(),
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(10),
            item_type: None,
            max_chars_per_item: None,
        })
        .await
        .expect("should succeed with fallback to user scope");

    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].key, "FB1");
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_scope_collections_merges_across_libraries() {
    let server = MockServer::start().await;

    // User scope has one collection.
    Mock::given(method("GET"))
        .and(path("/users/42/collections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "key": "COL_U", "data": { "name": "User Collection" } }
        ])))
        .mount(&server)
        .await;

    // Groups discovery.
    Mock::given(method("GET"))
        .and(path("/users/42/groups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 999, "data": { "name": "Lab" } }
        ])))
        .mount(&server)
        .await;

    // Group scope has another collection.
    Mock::given(method("GET"))
        .and(path("/groups/999/collections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "key": "COL_G", "data": { "name": "Group Collection" } }
        ])))
        .mount(&server)
        .await;

    let toolkit = build_test_toolkit_with_config(ResearchConfig {
        zotero_api_key: Some("test-key".to_string()),
        zotero_user_id: Some("42".to_string()),
        zotero_group_id: None,
        zotero_base_url: server.uri(),
        ..ResearchConfig::default()
    });

    let result = toolkit
        .zotero_get_collections(ZoteroCollectionsParams {
            library_type: None,
            library_id: None,
            offset: Some(0),
            limit: Some(20),
        })
        .await
        .expect("multi-scope collections should succeed");

    assert_eq!(result.collections.len(), 2);
    let names: Vec<_> = result.collections.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"User Collection"));
    assert!(names.contains(&"Group Collection"));
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_prefers_arxiv_pdf_when_available() {
    let item = sample_item_with_source(Some("https://arxiv.org/abs/2401.12345v2"), None);
    let attachments = vec![sample_attachment(
        "ATT1",
        Some("application/pdf"),
        Some("https://example.com/fallback.pdf"),
        None,
    )];

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &attachments,
        None,
        Some(true),
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::ArxivPdf,
            preferred_url: Some("https://arxiv.org/pdf/2401.12345v2.pdf".to_string()),
            fallback_urls: vec!["https://example.com/fallback.pdf".to_string()],
            local_path: None,
            trace: vec![
                "detected arXiv id from item URL: 2401.12345v2".to_string(),
                "using arXiv PDF".to_string()
            ],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_returns_arxiv_pdf_for_arxiv_items() {
    let item = sample_item_with_source(Some("https://arxiv.org/abs/2401.12345v2"), None);
    let attachments = vec![sample_attachment(
        "ATT1",
        Some("application/pdf"),
        Some("https://example.com/fallback.pdf"),
        None,
    )];

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &attachments,
        None,
        Some(true),
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::ArxivPdf,
            preferred_url: Some("https://arxiv.org/pdf/2401.12345v2.pdf".to_string()),
            fallback_urls: vec!["https://example.com/fallback.pdf".to_string()],
            local_path: None,
            trace: vec![
                "detected arXiv id from item URL: 2401.12345v2".to_string(),
                "using arXiv PDF".to_string()
            ],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_extracts_arxiv_id_from_extra_when_url_missing() {
    let item = sample_item_with_source(None, Some("Notes\narXiv:2401.54321v3"));
    let attachments = vec![sample_attachment(
        "ATT1",
        Some("application/pdf"),
        Some("https://example.com/fallback.pdf"),
        None,
    )];

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &attachments,
        None,
        Some(true),
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::ArxivPdf,
            preferred_url: Some("https://arxiv.org/pdf/2401.54321v3.pdf".to_string()),
            fallback_urls: vec!["https://example.com/fallback.pdf".to_string()],
            local_path: None,
            trace: vec![
                "detected arXiv id from item extra: 2401.54321v3".to_string(),
                "using arXiv PDF".to_string()
            ],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_supports_old_style_arxiv_ids() {
    let item = sample_item_with_source(Some("https://arxiv.org/abs/hep-ph/0001234v1"), None);

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &Vec::new(),
        None,
        Some(true),
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::ArxivPdf,
            preferred_url: Some("https://arxiv.org/pdf/hep-ph/0001234v1.pdf".to_string()),
            fallback_urls: Vec::new(),
            local_path: None,
            trace: vec![
                "detected arXiv id from item URL: hep-ph/0001234v1".to_string(),
                "using arXiv PDF".to_string()
            ],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_prefers_attachment_pdf_urls_for_non_arxiv_items() {
    let item = sample_item_with_source(Some("https://example.com/paper"), None);
    let attachments = vec![
        sample_attachment(
            "ATT1",
            Some("application/pdf"),
            Some("https://example.com/primary.pdf"),
            None,
        ),
        sample_attachment(
            "ATT2",
            Some("application/pdf"),
            Some("https://example.com/secondary.pdf"),
            None,
        ),
    ];

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &attachments,
        None,
        Some(false),
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::AttachmentPdfUrl,
            preferred_url: Some("https://example.com/primary.pdf".to_string()),
            fallback_urls: vec!["https://example.com/secondary.pdf".to_string()],
            local_path: None,
            trace: vec!["using attachment PDF URL".to_string()],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_handles_missing_item_metadata() {
    let attachments = vec![sample_attachment(
        "ATT1",
        Some("application/pdf"),
        Some("https://example.com/primary.pdf"),
        None,
    )];

    let resolution = super::resolve_document_sources_with_storage_root(
        None,
        &attachments,
        None,
        Some(false),
        vec!["item metadata lookup failed: missing".to_string()],
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::AttachmentPdfUrl,
            preferred_url: Some("https://example.com/primary.pdf".to_string()),
            fallback_urls: Vec::new(),
            local_path: None,
            trace: vec![
                "item metadata lookup failed: missing".to_string(),
                "using attachment PDF URL".to_string()
            ],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_uses_indexed_fulltext_trace_when_no_sources_and_no_content() {
    let item = sample_item_with_source(Some("https://example.com/paper"), None);

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &Vec::new(),
        None,
        Some(false),
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::IndexedFulltext,
            preferred_url: None,
            fallback_urls: Vec::new(),
            local_path: None,
            trace: vec![
                "local path fallback unavailable: ZOTERO_STORAGE_DIR is not set".to_string(),
                "no canonical document source resolved and indexed fulltext is empty".to_string()
            ],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_marks_unknown_indexed_content_when_not_provided() {
    let item = sample_item_with_source(Some("https://example.com/paper"), None);

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &Vec::new(),
        None,
        None,
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::IndexedFulltext,
            preferred_url: None,
            fallback_urls: Vec::new(),
            local_path: None,
            trace: vec![
                "local path fallback unavailable: ZOTERO_STORAGE_DIR is not set".to_string(),
                "no canonical document source resolved; indexed fulltext availability unknown"
                    .to_string()
            ],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_skips_malformed_arxiv_urls() {
    let item = sample_item_with_source(Some("https://arxiv.org/foo/2401.12345"), None);
    let attachments = vec![sample_attachment(
        "ATT1",
        Some("application/pdf"),
        Some("https://example.com/primary.pdf"),
        None,
    )];

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &attachments,
        None,
        Some(true),
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::AttachmentPdfUrl,
            preferred_url: Some("https://example.com/primary.pdf".to_string()),
            fallback_urls: Vec::new(),
            local_path: None,
            trace: vec!["using attachment PDF URL".to_string()],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn normalize_arxiv_id_strips_prefix_case_insensitively() {
    assert_eq!(
        super::normalize_arxiv_id("ArXiV:2401.12345v1"),
        Some("2401.12345v1".to_string())
    );
    assert_eq!(
        super::normalize_arxiv_id("arxiv:2401.12345v2"),
        Some("2401.12345v2".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_uses_local_storage_path_when_no_pdf_url_exists() {
    let temp_dir = tempfile::tempdir().expect("temp directory");
    let storage_root = temp_dir.path().to_string_lossy().to_string();
    let item = sample_item_with_source(Some("https://example.com/paper"), None);
    let attachments = vec![sample_attachment(
        "ATT1",
        Some("application/pdf"),
        None,
        Some("storage:paper.pdf"),
    )];

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &attachments,
        Some(storage_root.as_str()),
        Some(false),
        Vec::new(),
    )
    .await;

    let expected_path = temp_dir
        .path()
        .join("ATT1")
        .join("paper.pdf")
        .to_string_lossy()
        .to_string();
    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::LocalPdfPath,
            preferred_url: None,
            fallback_urls: Vec::new(),
            local_path: Some(expected_path),
            trace: vec!["resolved local PDF path from attachment ATT1".to_string()],
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_document_sources_rejects_unsafe_local_paths() {
    let temp_dir = tempfile::tempdir().expect("temp directory");
    let storage_root = temp_dir.path().to_string_lossy().to_string();
    let item = sample_item_with_source(Some("https://example.com/paper"), None);
    let attachments = vec![sample_attachment(
        "ATT1",
        Some("application/pdf"),
        None,
        Some("storage:../../escape.pdf"),
    )];

    let resolution = super::resolve_document_sources_with_storage_root(
        Some(&item),
        &attachments,
        Some(storage_root.as_str()),
        Some(true),
        Vec::new(),
    )
    .await;

    assert_eq!(
        resolution,
        DocumentResolution {
            source_kind: DocumentSourceKind::IndexedFulltext,
            preferred_url: None,
            fallback_urls: Vec::new(),
            local_path: None,
            trace: vec![
                "ignored attachment path for ATT1 due to unsafe relative path".to_string(),
                "no canonical document source resolved; using indexed fulltext fallback"
                    .to_string()
            ],
        }
    );
}

async fn mount_item_detail(server: &MockServer, item_key: &str, data: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/users/123/items/{item_key}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": item_key,
            "data": data
        })))
        .mount(server)
        .await;
}

fn sample_item_with_source(url: Option<&str>, extra: Option<&str>) -> ZoteroItemDetail {
    ZoteroItemDetail {
        key: "ITEM1".to_string(),
        title: "Sample Item".to_string(),
        authors: vec!["Alice Example".to_string()],
        abstract_text: None,
        date: Some("2024".to_string()),
        doi: None,
        url: url.map(ToString::to_string),
        publication: None,
        item_type: "journalArticle".to_string(),
        tags: Vec::new(),
        extra: extra.map(ToString::to_string),
        source_meta: None,
        attachments: None,
        document_resolution: None,
    }
}

fn sample_attachment(
    key: &str,
    content_type: Option<&str>,
    url: Option<&str>,
    path: Option<&str>,
) -> ZoteroAttachment {
    ZoteroAttachment {
        key: key.to_string(),
        title: Some("attachment".to_string()),
        filename: Some("paper.pdf".to_string()),
        content_type: content_type.map(ToString::to_string),
        link_mode: Some("imported_file".to_string()),
        url: url.map(ToString::to_string),
        path: path.map(ToString::to_string),
        parent_item: Some("ITEM1".to_string()),
        source_meta: None,
    }
}

fn build_test_toolkit(zotero_base_url: String) -> ResearchToolkit {
    build_test_toolkit_with_config(ResearchConfig {
        zotero_api_key: Some("test-key".to_string()),
        zotero_user_id: Some("123".to_string()),
        zotero_group_id: Some("456".to_string()),
        zotero_base_url,
        ..ResearchConfig::default()
    })
}
