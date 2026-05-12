use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use clap::Args;
use clap::CommandFactory;
use clap::Parser;
use clap::Subcommand;
use codex_research_tools::ResearchToolkit;
use codex_research_tools::config::DEFAULT_LOCAL_ZOTERO_BASE_URL;
use codex_research_tools::config::ResearchConfig;
use codex_research_tools::types::Paper;
use codex_research_tools::types::PaperSearchParams;
use codex_research_tools::types::SearchResult;
use codex_research_tools::types::ZoteroAddItemsToCollectionParams;
use codex_research_tools::types::ZoteroAnnotationsParams;
use codex_research_tools::types::ZoteroCitationParams;
use codex_research_tools::types::ZoteroCollection;
use codex_research_tools::types::ZoteroCollectionItemsParams;
use codex_research_tools::types::ZoteroCollectionsParams;
use codex_research_tools::types::ZoteroCreateAttachmentImportUrlParams;
use codex_research_tools::types::ZoteroCreateAttachmentLinkParams;
use codex_research_tools::types::ZoteroCreateCollectionParams;
use codex_research_tools::types::ZoteroCreateItemsParams;
use codex_research_tools::types::ZoteroFindOrCreateCollectionParams;
use codex_research_tools::types::ZoteroGrepParams;
use codex_research_tools::types::ZoteroGroup;
use codex_research_tools::types::ZoteroItem;
use codex_research_tools::types::ZoteroItemDetail;
use codex_research_tools::types::ZoteroItemParams;
use codex_research_tools::types::ZoteroListGroupsParams;
use codex_research_tools::types::ZoteroQuickSearchMode;
use codex_research_tools::types::ZoteroRecentParams;
use codex_research_tools::types::ZoteroSearchNotesParams;
use codex_research_tools::types::ZoteroSearchParams;
use codex_research_tools::types::ZoteroTagsParams;
use codex_research_tools::types::ZoteroUpdateItemsParams;
use codex_utils_cli::CliConfigOverrides;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn parse_positive_usize(input: &str) -> std::result::Result<usize, String> {
    let value = input
        .parse::<usize>()
        .map_err(|err| format!("invalid usize value '{input}': {err}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_string());
    }
    Ok(value)
}

#[derive(Debug, Parser)]
pub struct ZoteroCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,

    #[command(subcommand)]
    pub command: ZoteroCommand,
}

#[derive(Debug, Subcommand)]
pub enum ZoteroCommand {
    /// Search the Zotero command catalog and print simplified manuals for the best matches.
    SearchCommands(SearchCommandsArgs),

    /// Show the effective Zotero mode, scope, and fallback path for this shell.
    Status(StatusArgs),

    /// Resolve one paper from Zotero and enrich it with document metadata.
    ResolvePaper(ResolvePaperArgs),

    /// Add a paper to a Zotero collection, attach its PDF, and link a source repo when available.
    AddPaper(AddPaperArgs),

    /// Find repository URLs in Zotero items, collections, or linked records.
    FindRepos(FindReposArgs),

    /// Search items by keyword across titles, creators, and tags.
    Search(SearchArgs),

    /// List tags for autocomplete and filtering flows.
    Tags(TagsArgs),

    /// List recently added or modified items.
    Recent(RecentArgs),

    /// Run multi-condition metadata/fulltext search from a JSON payload.
    AdvancedSearch(JsonPayloadCommand),

    /// Run bounded literal or regex matching from a JSON payload.
    GrepText(JsonPayloadCommand),

    /// Search note and annotation text.
    SearchNotes(SearchNotesArgs),

    /// Read or inspect a Zotero item.
    Item(ItemCli),

    /// List Zotero collections.
    Collections(CollectionsListArgs),

    /// Operate on a specific collection.
    Collection(CollectionCli),

    /// List accessible Zotero groups.
    Groups(GroupsCli),

    /// Create or update batches of items.
    Items(ItemsCli),

    /// Create linked attachments under existing items.
    Attachment(AttachmentCli),
}

#[derive(Debug, Args)]
pub struct SearchCommandsArgs {
    #[arg(value_name = "QUERY", num_args = 1.., required = true)]
    pub query: Vec<String>,

    #[arg(long, default_value_t = 3, value_parser = parse_positive_usize)]
    pub limit: usize,
}

#[derive(Debug, Args, Clone, Default)]
pub struct CompactOutputArgs {
    #[arg(long)]
    pub compact: bool,
}

#[derive(Debug, Args, Clone, Default)]
pub struct JsonOutputArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[command(flatten)]
    pub output: JsonOutputArgs,
}

#[derive(Debug, Args)]
pub struct ResolvePaperArgs {
    #[arg(
        long,
        conflicts_with = "item_key",
        required_unless_present = "item_key"
    )]
    pub query: Option<String>,

    #[arg(long, conflicts_with = "query", required_unless_present = "query")]
    pub item_key: Option<String>,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long, default_value_t = 5)]
    pub limit: u32,

    #[command(flatten)]
    pub output: JsonOutputArgs,
}

#[derive(Debug, Args)]
pub struct AddPaperArgs {
    #[arg(long, conflicts_with_all = ["doi", "arxiv", "url"], required_unless_present_any = ["doi", "arxiv", "url"])]
    pub query: Option<String>,

    #[arg(long, conflicts_with_all = ["query", "arxiv", "url"], required_unless_present_any = ["query", "arxiv", "url"])]
    pub doi: Option<String>,

    #[arg(long, conflicts_with_all = ["query", "doi", "url"], required_unless_present_any = ["query", "doi", "url"])]
    pub arxiv: Option<String>,

    #[arg(long, conflicts_with_all = ["query", "doi", "arxiv"], required_unless_present_any = ["query", "doi", "arxiv"])]
    pub url: Option<String>,

    #[arg(long)]
    pub collection: String,

    #[arg(long, default_value = "Source Repos")]
    pub repo_collection: String,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[command(flatten)]
    pub output: JsonOutputArgs,
}

#[derive(Debug, Args)]
pub struct FindReposArgs {
    #[arg(long)]
    pub query: Option<String>,

    #[arg(long)]
    pub collection: Option<String>,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long, default_value_t = 10)]
    pub limit: u32,

    #[arg(long, default_value_t = 20)]
    pub inspect_limit: u32,

    #[command(flatten)]
    pub output: JsonOutputArgs,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    #[arg(long)]
    pub query: String,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long)]
    pub item_type: Option<String>,

    #[arg(long)]
    pub qmode: Option<String>,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub offset: Option<u32>,

    #[arg(long)]
    pub max_chars_per_item: Option<u32>,

    #[command(flatten)]
    pub output: CompactOutputArgs,
}

#[derive(Debug, Args)]
pub struct TagsArgs {
    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub offset: Option<u32>,

    #[command(flatten)]
    pub output: CompactOutputArgs,
}

#[derive(Debug, Args)]
pub struct RecentArgs {
    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long)]
    pub item_type: Option<String>,

    #[arg(long)]
    pub sort_by: Option<String>,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub offset: Option<u32>,

    #[arg(long)]
    pub max_chars_per_item: Option<u32>,

    #[command(flatten)]
    pub output: CompactOutputArgs,
}

#[derive(Debug, Args)]
pub struct SearchNotesArgs {
    #[arg(long)]
    pub query: String,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long)]
    pub parent_item_key: Option<String>,

    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub include_annotations: Option<bool>,

    #[arg(long)]
    pub match_mode: Option<String>,

    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub case_sensitive: Option<bool>,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Args)]
pub struct CollectionsListArgs {
    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub offset: Option<u32>,

    #[command(flatten)]
    pub output: CompactOutputArgs,
}

#[derive(Debug, Parser)]
pub struct ItemCli {
    #[command(subcommand)]
    pub command: ItemCommand,
}

#[derive(Debug, Subcommand)]
pub enum ItemCommand {
    /// Get item metadata and optional attachment/fulltext resolution.
    Get(ItemGetArgs),

    /// Generate a citation for an item.
    Citation(ItemCitationArgs),

    /// Get indexed fulltext and document resolution.
    Fulltext(ItemBaseArgs),

    /// Get notes attached to an item.
    Notes(ItemBaseArgs),

    /// Get annotations for an item or library scope.
    Annotations(ItemAnnotationsArgs),

    /// Get attachments for an item.
    Attachments(ItemBaseArgs),
}

#[derive(Debug, Parser)]
pub struct CollectionCli {
    #[command(subcommand)]
    pub command: CollectionCommand,
}

#[derive(Debug, Subcommand)]
pub enum CollectionCommand {
    /// List items in a collection.
    Items(CollectionItemsArgs),

    /// Create a collection.
    Create(CollectionCreateArgs),

    /// Find a collection by exact name or create it.
    FindOrCreate(CollectionCreateArgs),

    /// Add existing items to a collection.
    AddItems(CollectionAddItemsArgs),
}

#[derive(Debug, Parser)]
pub struct GroupsCli {
    #[command(subcommand)]
    pub command: GroupsCommand,
}

#[derive(Debug, Subcommand)]
pub enum GroupsCommand {
    /// List groups visible to the configured account.
    List(GroupsListArgs),
}

#[derive(Debug, Parser)]
pub struct ItemsCli {
    #[command(subcommand)]
    pub command: ItemsCommand,
}

#[derive(Debug, Subcommand)]
pub enum ItemsCommand {
    /// Create items from a JSON payload.
    Create(JsonPayloadCommand),

    /// Update items from a JSON payload.
    Update(JsonPayloadCommand),
}

#[derive(Debug, Parser)]
pub struct AttachmentCli {
    #[command(subcommand)]
    pub command: AttachmentCommand,
}

#[derive(Debug, Subcommand)]
pub enum AttachmentCommand {
    /// Create a linked attachment under an existing item.
    CreateLink(AttachmentCreateLinkArgs),

    /// Import a URL as a stored attachment under an existing item.
    ImportUrl(AttachmentImportUrlArgs),
}

#[derive(Debug, Args, Clone)]
pub struct LibraryScopeArgs {
    #[arg(long)]
    pub library_type: Option<String>,

    #[arg(long)]
    pub library_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct ItemBaseArgs {
    #[arg(long)]
    pub item_key: String,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long)]
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Args)]
pub struct ItemGetArgs {
    #[command(flatten)]
    pub base: ItemBaseArgs,

    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub include_attachments: Option<bool>,

    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub include_fulltext_resolution: Option<bool>,

    #[command(flatten)]
    pub output: CompactOutputArgs,
}

#[derive(Debug, Args)]
pub struct ItemCitationArgs {
    #[arg(long)]
    pub item_key: String,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long)]
    pub format: Option<String>,
}

#[derive(Debug, Args)]
pub struct ItemAnnotationsArgs {
    #[arg(long)]
    pub item_key: Option<String>,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub include_parent_context: Option<bool>,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub offset: Option<u32>,

    #[arg(long)]
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Args)]
pub struct CollectionItemsArgs {
    #[arg(long)]
    pub collection_key: String,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,

    #[arg(long)]
    pub item_type: Option<String>,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub offset: Option<u32>,

    #[arg(long)]
    pub max_chars_per_item: Option<u32>,

    #[command(flatten)]
    pub output: CompactOutputArgs,
}

#[derive(Debug, Args)]
pub struct CollectionCreateArgs {
    #[arg(long)]
    pub name: String,

    #[arg(long)]
    pub parent_collection_key: Option<String>,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,
}

#[derive(Debug, Args)]
pub struct CollectionAddItemsArgs {
    #[arg(long)]
    pub collection_key: String,

    #[arg(long, num_args = 1..)]
    pub item_keys: Vec<String>,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,
}

#[derive(Debug, Args)]
pub struct GroupsListArgs {
    #[arg(long)]
    pub user_id: Option<String>,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub offset: Option<u32>,

    #[command(flatten)]
    pub output: CompactOutputArgs,
}

#[derive(Debug, Args)]
pub struct AttachmentCreateLinkArgs {
    #[arg(long)]
    pub parent_item_key: String,

    #[arg(long)]
    pub title: String,

    #[arg(long)]
    pub url: String,

    #[arg(long)]
    pub content_type: Option<String>,

    #[arg(long, num_args = 1..)]
    pub collections: Vec<String>,

    #[arg(long, num_args = 1..)]
    pub tags: Vec<String>,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,
}

#[derive(Debug, Args)]
pub struct AttachmentImportUrlArgs {
    #[arg(long)]
    pub parent_item_key: String,

    #[arg(long)]
    pub title: String,

    #[arg(long)]
    pub url: String,

    #[arg(long)]
    pub content_type: Option<String>,

    #[arg(long)]
    pub filename: Option<String>,

    #[arg(long, num_args = 1..)]
    pub tags: Vec<String>,

    #[command(flatten)]
    pub scope: LibraryScopeArgs,
}

#[derive(Debug, Args)]
pub struct JsonPayloadCommand {
    #[command(flatten)]
    pub payload: JsonPayloadArgs,
}

#[derive(Debug, Args)]
pub struct JsonPayloadArgs {
    #[arg(
        long,
        conflicts_with = "json_file",
        required_unless_present = "json_file"
    )]
    pub json: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "json",
        required_unless_present = "json"
    )]
    pub json_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ZoteroMode {
    Local,
    Remote,
}

impl std::fmt::Display for ZoteroMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Remote => write!(f, "remote"),
        }
    }
}

struct ZoteroRuntime {
    toolkit: ResearchToolkit,
    config: ResearchConfig,
    mode: ZoteroMode,
}

struct ZoteroCliContext {
    primary: ZoteroRuntime,
    alternate: Option<ZoteroRuntime>,
}

#[derive(Debug, serde::Serialize)]
struct ZoteroStatusResult {
    effective_mode: ZoteroMode,
    zotero_base_url: String,
    api_key_configured: bool,
    library_type: Option<String>,
    library_id: Option<String>,
    default_write_library_type: Option<String>,
    default_write_library_id: Option<String>,
    user_id: Option<String>,
    group_id: Option<String>,
    alternate_mode: Option<ZoteroMode>,
    alternate_base_url: Option<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ZoteroCollectionMatch {
    key: String,
    name: String,
    parent_collection: Option<String>,
    scope: Option<ZoteroScopeRef>,
    score: u32,
}

#[derive(Debug, serde::Serialize)]
struct ZoteroResolvedPaperResult {
    effective_mode: ZoteroMode,
    fallback_used: bool,
    warnings: Vec<String>,
    candidates: Vec<ZoteroItem>,
    best_match: Option<ZoteroItemDetail>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ZoteroScopeRef {
    library_type: String,
    library_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ZoteroRepoMatch {
    item_key: String,
    title: String,
    repo_url: String,
    discovered_via: String,
    parent_item: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct ZoteroFindReposResult {
    effective_mode: ZoteroMode,
    fallback_used: bool,
    warnings: Vec<String>,
    matched_collections: Vec<ZoteroCollectionMatch>,
    repos: Vec<ZoteroRepoMatch>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ZoteroAddPaperAttachmentResult {
    item_key: Option<String>,
    status: String,
    url: Option<String>,
    warning: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ZoteroAddPaperRepoResult {
    item_key: String,
    title: String,
    url: String,
    status: String,
    collection_key: String,
}

#[derive(Debug, serde::Serialize)]
struct ZoteroAddPaperResult {
    effective_mode: ZoteroMode,
    scope: ZoteroScopeRef,
    collection: ZoteroCollectionMatch,
    paper_key: String,
    paper_title: String,
    paper_created: bool,
    pdf: ZoteroAddPaperAttachmentResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot: Option<ZoteroAddPaperAttachmentResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<ZoteroAddPaperRepoResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedExternalPaper {
    paper: Paper,
    warnings: Vec<String>,
}

pub async fn run_zotero_command(cli: ZoteroCli) -> Result<()> {
    let ZoteroCli {
        config_overrides,
        command,
    } = cli;

    macro_rules! print_json {
        ($expr:expr) => {{
            let output = $expr.await?;
            print_pretty_json(&output)?;
            Ok(())
        }};
    }

    match command {
        ZoteroCommand::SearchCommands(args) => {
            let query = args.query.join(" ");
            let matches = search_command_catalog(&query, args.limit);
            if matches.is_empty() {
                println!("No matching Zotero command found for \"{}\".", query.trim());
                return Ok(());
            }
            let best_match = matches[0];
            let manual = render_command_manual(best_match)?;
            println!("{}", render_search_results(&matches, &manual));
            Ok(())
        }
        ZoteroCommand::Status(args) => {
            let context = load_zotero_context(&config_overrides).await?;
            let status = build_status_result(&context.primary, context.alternate.as_ref());
            if args.output.json {
                print_pretty_json(&status)?;
            } else {
                print_status_compact(&status);
            }
            Ok(())
        }
        ZoteroCommand::ResolvePaper(args) => {
            let context = load_zotero_context(&config_overrides).await?;
            let result = resolve_paper_with_fallback(&context, &args).await?;
            if args.output.json {
                print_pretty_json(&result)?;
            } else {
                print_resolved_paper_compact(&result);
            }
            Ok(())
        }
        ZoteroCommand::AddPaper(args) => {
            let context = load_zotero_context(&config_overrides).await?;
            let result = add_paper(&context.primary, &args).await?;
            if args.output.json {
                print_pretty_json(&result)?;
            } else {
                print_add_paper_compact(&result);
            }
            Ok(())
        }
        ZoteroCommand::FindRepos(args) => {
            let context = load_zotero_context(&config_overrides).await?;
            let result = find_repos_with_fallback(&context, &args).await?;
            if args.output.json {
                print_pretty_json(&result)?;
            } else {
                print_repo_matches_compact(&result);
            }
            Ok(())
        }
        ZoteroCommand::Search(args) => {
            let toolkit = load_toolkit(&config_overrides).await?;
            let output = toolkit
                .zotero_search(ZoteroSearchParams {
                    query: args.query,
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    offset: args.offset,
                    limit: args.limit,
                    item_type: args.item_type,
                    qmode: parse_optional_enum(args.qmode, "qmode")?,
                    max_chars_per_item: args.max_chars_per_item,
                })
                .await?;
            if args.output.compact {
                print_items_compact(&output.items);
                Ok(())
            } else {
                print_pretty_json(&output)
            }
        }
        ZoteroCommand::Tags(args) => {
            let toolkit = load_toolkit(&config_overrides).await?;
            let output = toolkit
                .zotero_get_tags(ZoteroTagsParams {
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    limit: args.limit,
                    offset: args.offset,
                })
                .await?;
            if args.output.compact {
                print_tags_compact(&output.tags);
                Ok(())
            } else {
                print_pretty_json(&output)
            }
        }
        ZoteroCommand::Recent(args) => {
            let toolkit = load_toolkit(&config_overrides).await?;
            let output = toolkit
                .zotero_get_recent(ZoteroRecentParams {
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    limit: args.limit,
                    offset: args.offset,
                    item_type: args.item_type,
                    sort_by: parse_optional_enum(args.sort_by, "sort_by")?,
                    max_chars_per_item: args.max_chars_per_item,
                })
                .await?;
            if args.output.compact {
                print_items_compact(&output.items);
                Ok(())
            } else {
                print_pretty_json(&output)
            }
        }
        ZoteroCommand::AdvancedSearch(args) => {
            let toolkit = load_toolkit(&config_overrides).await?;
            let params = parse_json_payload::<
                codex_research_tools::types::ZoteroAdvancedSearchParams,
            >(&args.payload, "advanced-search")?;
            print_json!(toolkit.zotero_advanced_search(params))
        }
        ZoteroCommand::GrepText(args) => {
            let toolkit = load_toolkit(&config_overrides).await?;
            let params = parse_json_payload::<ZoteroGrepParams>(&args.payload, "grep-text")?;
            print_json!(toolkit.zotero_grep_text(params))
        }
        ZoteroCommand::SearchNotes(args) => {
            let toolkit = load_toolkit(&config_overrides).await?;
            print_json!(toolkit.zotero_search_notes(ZoteroSearchNotesParams {
                query: args.query,
                match_mode: parse_optional_enum(args.match_mode, "match_mode")?,
                case_sensitive: args.case_sensitive,
                library_type: args.scope.library_type,
                library_id: args.scope.library_id,
                parent_item_key: args.parent_item_key,
                include_annotations: args.include_annotations,
                limit: args.limit,
                max_chars_per_item: args.max_chars_per_item,
            }))
        }
        ZoteroCommand::Item(cli) => match cli.command {
            ItemCommand::Get(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                let output = toolkit
                    .zotero_get_item(ZoteroItemParams {
                        item_key: args.base.item_key,
                        library_type: args.base.scope.library_type,
                        library_id: args.base.scope.library_id,
                        max_chars_per_item: args.base.max_chars_per_item,
                        include_attachments: args.include_attachments,
                        include_fulltext_resolution: args.include_fulltext_resolution,
                    })
                    .await?;
                if args.output.compact {
                    print_item_detail_compact(&output);
                    Ok(())
                } else {
                    print_pretty_json(&output)
                }
            }
            ItemCommand::Citation(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_get_item_citation(ZoteroCitationParams {
                    item_key: args.item_key,
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    format: parse_optional_enum(args.format, "format")?,
                }))
            }
            ItemCommand::Fulltext(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_get_fulltext(ZoteroItemParams {
                    item_key: args.item_key,
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    max_chars_per_item: args.max_chars_per_item,
                    include_attachments: None,
                    include_fulltext_resolution: None,
                }))
            }
            ItemCommand::Notes(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_get_notes(ZoteroItemParams {
                    item_key: args.item_key,
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    max_chars_per_item: args.max_chars_per_item,
                    include_attachments: None,
                    include_fulltext_resolution: None,
                }))
            }
            ItemCommand::Annotations(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_get_annotations(ZoteroAnnotationsParams {
                    item_key: args.item_key,
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    include_parent_context: args.include_parent_context,
                    limit: args.limit,
                    offset: args.offset,
                    max_chars_per_item: args.max_chars_per_item,
                }))
            }
            ItemCommand::Attachments(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_get_attachments(ZoteroItemParams {
                    item_key: args.item_key,
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    max_chars_per_item: args.max_chars_per_item,
                    include_attachments: None,
                    include_fulltext_resolution: None,
                }))
            }
        },
        ZoteroCommand::Collections(args) => {
            let toolkit = load_toolkit(&config_overrides).await?;
            let requested_scope = explicit_scope(&args.scope);
            let output = toolkit
                .zotero_get_collections(ZoteroCollectionsParams {
                    library_type: args.scope.library_type.clone(),
                    library_id: args.scope.library_id.clone(),
                    limit: args.limit,
                    offset: args.offset,
                })
                .await?;
            if args.output.compact {
                let matches = output
                    .collections
                    .into_iter()
                    .map(|collection| {
                        let scope =
                            scope_from_collection(&collection).or_else(|| requested_scope.clone());
                        ZoteroCollectionMatch {
                            key: collection.key,
                            name: collection.name,
                            parent_collection: collection.parent_collection,
                            scope,
                            score: 0,
                        }
                    })
                    .collect::<Vec<_>>();
                print_collection_matches_compact(&matches, &[]);
                Ok(())
            } else {
                print_pretty_json(&output)
            }
        }
        ZoteroCommand::Collection(cli) => match cli.command {
            CollectionCommand::Items(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                let output = toolkit
                    .zotero_get_collection_items(ZoteroCollectionItemsParams {
                        collection_key: args.collection_key,
                        library_type: args.scope.library_type,
                        library_id: args.scope.library_id,
                        limit: args.limit,
                        offset: args.offset,
                        item_type: args.item_type,
                        max_chars_per_item: args.max_chars_per_item,
                    })
                    .await?;
                if args.output.compact {
                    print_items_compact(&output.items);
                    Ok(())
                } else {
                    print_pretty_json(&output)
                }
            }
            CollectionCommand::Create(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(
                    toolkit.zotero_create_collection(ZoteroCreateCollectionParams {
                        name: args.name,
                        parent_collection_key: args.parent_collection_key,
                        library_type: args.scope.library_type,
                        library_id: args.scope.library_id,
                    })
                )
            }
            CollectionCommand::FindOrCreate(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_find_or_create_collection(
                    ZoteroFindOrCreateCollectionParams {
                        name: args.name,
                        parent_collection_key: args.parent_collection_key,
                        library_type: args.scope.library_type,
                        library_id: args.scope.library_id,
                    }
                ))
            }
            CollectionCommand::AddItems(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_add_items_to_collection(
                    ZoteroAddItemsToCollectionParams {
                        collection_key: args.collection_key,
                        item_keys: args.item_keys,
                        library_type: args.scope.library_type,
                        library_id: args.scope.library_id,
                    }
                ))
            }
        },
        ZoteroCommand::Groups(cli) => match cli.command {
            GroupsCommand::List(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                let output = toolkit
                    .zotero_list_groups(ZoteroListGroupsParams {
                        user_id: args.user_id,
                        limit: args.limit,
                        offset: args.offset,
                    })
                    .await?;
                if args.output.compact {
                    print_groups_compact(&output.groups);
                    Ok(())
                } else {
                    print_pretty_json(&output)
                }
            }
        },
        ZoteroCommand::Items(cli) => match cli.command {
            ItemsCommand::Create(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                let params =
                    parse_json_payload::<ZoteroCreateItemsParams>(&args.payload, "items create")?;
                print_json!(toolkit.zotero_create_items(params))
            }
            ItemsCommand::Update(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                let params =
                    parse_json_payload::<ZoteroUpdateItemsParams>(&args.payload, "items update")?;
                print_json!(toolkit.zotero_update_items(params))
            }
        },
        ZoteroCommand::Attachment(cli) => match cli.command {
            AttachmentCommand::CreateLink(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_create_attachment_link(
                    ZoteroCreateAttachmentLinkParams {
                        parent_item_key: args.parent_item_key,
                        title: args.title,
                        url: args.url,
                        content_type: args.content_type,
                        collections: (!args.collections.is_empty()).then_some(args.collections),
                        tags: (!args.tags.is_empty()).then_some(args.tags),
                        library_type: args.scope.library_type,
                        library_id: args.scope.library_id,
                    }
                ))
            }
            AttachmentCommand::ImportUrl(args) => {
                let toolkit = load_toolkit(&config_overrides).await?;
                print_json!(toolkit.zotero_create_attachment_import_url(
                    ZoteroCreateAttachmentImportUrlParams {
                        parent_item_key: args.parent_item_key,
                        title: args.title,
                        url: args.url,
                        content_type: args.content_type,
                        filename: args.filename,
                        tags: (!args.tags.is_empty()).then_some(args.tags),
                        library_type: args.scope.library_type,
                        library_id: args.scope.library_id,
                    }
                ))
            }
        },
    }
}

async fn load_research_config(config_overrides: &CliConfigOverrides) -> Result<ResearchConfig> {
    let cli_overrides = config_overrides
        .parse_overrides()
        .map_err(|e| anyhow!("invalid -c override: {e}"))?;
    let cwd = std::env::current_dir().context("failed to read current working directory")?;

    match codex_core::config::Config::load_with_cli_overrides(cli_overrides).await {
        Ok(config) => {
            let toml: Option<codex_core::config::types::ResearchToolsToml> = config
                .config_layer_stack
                .effective_config()
                .as_table()
                .and_then(|t| t.get("research"))
                .and_then(|v| v.clone().try_into().ok());
            Ok(codex_core::research::build_research_config(
                toml.as_ref(),
                config.codex_home.as_path(),
                &cwd,
            ))
        }
        Err(_) => Ok(ResearchConfig::from_env()),
    }
}

async fn load_toolkit(config_overrides: &CliConfigOverrides) -> Result<ResearchToolkit> {
    Ok(ResearchToolkit::from_config(
        load_research_config(config_overrides).await?,
    ))
}

async fn load_zotero_context(config_overrides: &CliConfigOverrides) -> Result<ZoteroCliContext> {
    let config = load_research_config(config_overrides).await?;
    let primary_mode = zotero_mode(&config);
    let primary = ZoteroRuntime {
        toolkit: ResearchToolkit::from_config(config.clone()),
        config: config.clone(),
        mode: primary_mode,
    };
    let alternate = build_alternate_zotero_runtime(&config);
    Ok(ZoteroCliContext { primary, alternate })
}

fn zotero_mode(config: &ResearchConfig) -> ZoteroMode {
    if config.uses_local_zotero_api() {
        ZoteroMode::Local
    } else {
        ZoteroMode::Remote
    }
}

fn build_alternate_zotero_runtime(config: &ResearchConfig) -> Option<ZoteroRuntime> {
    if config.uses_local_zotero_api() {
        return None;
    }

    let mut alternate = config.clone();
    alternate.zotero_api_key = None;
    alternate.zotero_base_url = DEFAULT_LOCAL_ZOTERO_BASE_URL.to_string();
    Some(ZoteroRuntime {
        toolkit: ResearchToolkit::from_config(alternate.clone()),
        config: alternate,
        mode: ZoteroMode::Local,
    })
}

fn effective_library_id(config: &ResearchConfig) -> Option<String> {
    match config.zotero_library_type.as_deref() {
        Some("group") => config.zotero_group_id.clone(),
        Some("user") => config.zotero_user_id.clone(),
        _ => config
            .zotero_group_id
            .clone()
            .or_else(|| config.zotero_user_id.clone()),
    }
}

fn default_write_scope(config: &ResearchConfig) -> (Option<String>, Option<String>) {
    match config.zotero_library_type.as_deref() {
        Some("group") => (Some("group".to_string()), config.zotero_group_id.clone()),
        Some("user") => (Some("user".to_string()), config.zotero_user_id.clone()),
        _ => {
            if let Some(user_id) = config.zotero_user_id.clone() {
                (Some("user".to_string()), Some(user_id))
            } else {
                (Some("group".to_string()), config.zotero_group_id.clone())
            }
        }
    }
}

fn build_status_result(
    primary: &ZoteroRuntime,
    alternate: Option<&ZoteroRuntime>,
) -> ZoteroStatusResult {
    let mut warnings = Vec::new();
    if matches!(primary.mode, ZoteroMode::Remote) && alternate.is_some() {
        warnings.push(
            "High-level Zotero discovery commands will retry via the local Zotero API when the remote path returns no useful results.".to_string(),
        );
    }
    if matches!(primary.mode, ZoteroMode::Remote) {
        warnings.push(
            "The effective Zotero mode is remote because an API key is configured for this shell."
                .to_string(),
        );
    } else {
        warnings.push(
            "The effective Zotero mode is local because no Zotero API key is configured for this shell.".to_string(),
        );
    }

    ZoteroStatusResult {
        effective_mode: primary.mode,
        zotero_base_url: primary.config.zotero_base_url.clone(),
        api_key_configured: primary.config.has_zotero_api_key(),
        library_type: primary.config.zotero_library_type.clone(),
        library_id: effective_library_id(&primary.config),
        default_write_library_type: default_write_scope(&primary.config).0,
        default_write_library_id: default_write_scope(&primary.config).1,
        user_id: primary.config.zotero_user_id.clone(),
        group_id: primary.config.zotero_group_id.clone(),
        alternate_mode: alternate.map(|runtime| runtime.mode),
        alternate_base_url: alternate.map(|runtime| runtime.config.zotero_base_url.clone()),
        warnings,
    }
}

async fn resolve_paper_with_fallback(
    context: &ZoteroCliContext,
    args: &ResolvePaperArgs,
) -> Result<ZoteroResolvedPaperResult> {
    match resolve_paper_once(&context.primary, args).await {
        Ok(result) if result.best_match.is_some() || !result.candidates.is_empty() => Ok(result),
        Ok(result) => {
            let Some(alternate_runtime) = context.alternate.as_ref() else {
                return Ok(result);
            };
            let mut alternate = resolve_paper_once(alternate_runtime, args).await?;
            if alternate.best_match.is_none() && alternate.candidates.is_empty() {
                return Ok(result);
            }
            alternate.fallback_used = true;
            alternate.warnings.insert(
                0,
                format!(
                    "No useful paper match was found via {} mode, so the CLI retried via {} mode.",
                    context.primary.mode, alternate_runtime.mode
                ),
            );
            Ok(alternate)
        }
        Err(primary_error) => {
            let Some(alternate_runtime) = context.alternate.as_ref() else {
                return Err(primary_error);
            };
            let mut alternate = resolve_paper_once(alternate_runtime, args).await?;
            alternate.fallback_used = true;
            alternate.warnings.insert(
                0,
                format!(
                    "{} mode failed with: {}. The CLI retried via {} mode.",
                    context.primary.mode, primary_error, alternate_runtime.mode
                ),
            );
            Ok(alternate)
        }
    }
}

async fn resolve_paper_once(
    runtime: &ZoteroRuntime,
    args: &ResolvePaperArgs,
) -> Result<ZoteroResolvedPaperResult> {
    let mut warnings = Vec::new();
    let mut candidates = Vec::new();
    let best_match = if let Some(item_key) = args.item_key.as_deref() {
        Some(
            fetch_item_detail(runtime, &args.scope, item_key, true, true)
                .await
                .with_context(|| format!("resolve Zotero item `{item_key}`"))?,
        )
    } else if let Some(query) = args.query.as_deref() {
        let result = runtime
            .toolkit
            .zotero_search(ZoteroSearchParams {
                query: query.to_string(),
                library_type: args.scope.library_type.clone(),
                library_id: args.scope.library_id.clone(),
                offset: None,
                limit: Some(args.limit.clamp(1, 10)),
                item_type: None,
                qmode: None,
                max_chars_per_item: None,
            })
            .await?;
        candidates = result.items;
        if candidates.is_empty() {
            warnings.push(format!("No Zotero items matched query `{query}`."));
            None
        } else {
            let mut best_detail = None;
            for item in candidates.iter().take(3) {
                let detail = fetch_item_detail(runtime, &args.scope, item.key.as_str(), true, true)
                    .await
                    .with_context(|| format!("inspect Zotero item `{}`", item.key))?;
                let should_replace =
                    best_detail
                        .as_ref()
                        .is_none_or(|current: &ZoteroItemDetail| {
                            score_item_detail(&detail) > score_item_detail(current)
                        });
                if should_replace {
                    best_detail = Some(detail);
                }
            }
            best_detail
        }
    } else {
        bail!("either `--query` or `--item-key` is required");
    };

    Ok(ZoteroResolvedPaperResult {
        effective_mode: runtime.mode,
        fallback_used: false,
        warnings,
        candidates,
        best_match,
    })
}

async fn add_paper(runtime: &ZoteroRuntime, args: &AddPaperArgs) -> Result<ZoteroAddPaperResult> {
    let (target_collection, target_scope) =
        resolve_target_collection(runtime, &args.collection, &args.scope).await?;
    let target_scope_args = scope_args(&target_scope);

    let resolved_paper = resolve_external_paper(runtime, args).await?;
    let mut warnings = resolved_paper.warnings;
    let existing_paper =
        find_existing_paper_in_scope(runtime, &target_scope_args, &resolved_paper.paper).await?;

    let (paper_key, paper_title, paper_created) = if let Some(existing) = existing_paper.as_ref() {
        runtime
            .toolkit
            .zotero_add_items_to_collection(ZoteroAddItemsToCollectionParams {
                collection_key: target_collection.key.clone(),
                item_keys: vec![existing.key.clone()],
                library_type: Some(target_scope.library_type.clone()),
                library_id: Some(target_scope.library_id.clone()),
            })
            .await?;
        (existing.key.clone(), existing.title.clone(), false)
    } else {
        let created = runtime
            .toolkit
            .zotero_create_items(ZoteroCreateItemsParams {
                items: vec![paper_to_zotero_item(&resolved_paper.paper)],
                library_type: Some(target_scope.library_type.clone()),
                library_id: Some(target_scope.library_id.clone()),
            })
            .await?;
        let record = created
            .records
            .into_iter()
            .next()
            .context("zotero_create_items returned no paper record")?;
        runtime
            .toolkit
            .zotero_add_items_to_collection(ZoteroAddItemsToCollectionParams {
                collection_key: target_collection.key.clone(),
                item_keys: vec![record.key.clone()],
                library_type: Some(target_scope.library_type.clone()),
                library_id: Some(target_scope.library_id.clone()),
            })
            .await?;
        (
            record.key,
            record
                .title
                .unwrap_or_else(|| resolved_paper.paper.title.clone()),
            true,
        )
    };

    let current_paper =
        fetch_item_detail(runtime, &target_scope_args, &paper_key, true, false).await?;
    let pdf = ensure_pdf_attachment(
        runtime,
        &target_scope_args,
        &current_paper,
        &resolved_paper.paper,
    )
    .await;
    let pdf = match pdf {
        Ok(result) => result,
        Err(err) => {
            warnings.push(format!("PDF attachment failed: {err}"));
            ZoteroAddPaperAttachmentResult {
                item_key: None,
                status: "missing".to_string(),
                url: paper_preferred_pdf_url(&resolved_paper.paper),
                warning: Some(err.to_string()),
            }
        }
    };

    // The parent item URL remains the paper landing page. Do not add that same
    // URL as a child attachment, because Zotero may open the linked webpage
    // attachment instead of the PDF when the parent item is activated.
    let snapshot = None;

    let repo = ensure_repo_link(
        runtime,
        &target_scope,
        &target_scope_args,
        RepoLinkRequest {
            repo_collection_name: &args.repo_collection,
            paper_key: &paper_key,
            paper_title: &paper_title,
            paper: &resolved_paper.paper,
            warnings: &mut warnings,
        },
    )
    .await?;

    Ok(ZoteroAddPaperResult {
        effective_mode: runtime.mode,
        scope: target_scope,
        collection: target_collection,
        paper_key,
        paper_title,
        paper_created,
        pdf,
        snapshot,
        repo,
        warnings,
    })
}

async fn resolve_target_collection(
    runtime: &ZoteroRuntime,
    collection_ref: &str,
    scope: &LibraryScopeArgs,
) -> Result<(ZoteroCollectionMatch, ZoteroScopeRef)> {
    let collections = fetch_all_collections(runtime, scope, 300).await?;
    let requested_scope = explicit_scope(scope);
    let resolved = resolve_collection_reference(
        collections.as_slice(),
        collection_ref,
        requested_scope.as_ref(),
    );
    match resolved.as_slice() {
        [] => bail!("No Zotero collection matched `{collection_ref}`."),
        [collection] => {
            let scope = collection
                .scope
                .clone()
                .or_else(|| requested_scope.clone())
                .with_context(|| {
                    format!(
                        "missing scope metadata for Zotero collection `{}`",
                        collection.name
                    )
                })?;
            Ok((collection.clone(), scope))
        }
        matches => {
            let rendered = matches
                .iter()
                .map(|collection| {
                    let suffix = collection
                        .scope
                        .as_ref()
                        .map(|scope| format!(" [{}]", scope_label(scope)))
                        .unwrap_or_default();
                    format!("{}{}", collection.key, suffix)
                })
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "Collection reference `{collection_ref}` matched multiple Zotero collections: {rendered}. Pass an exact collection key or scope."
            )
        }
    }
}

async fn resolve_external_paper(
    runtime: &ZoteroRuntime,
    args: &AddPaperArgs,
) -> Result<ResolvedExternalPaper> {
    if let Some(doi) = args.doi.as_deref() {
        return resolve_paper_by_doi(runtime, doi).await;
    }
    if let Some(arxiv) = args.arxiv.as_deref() {
        return resolve_paper_by_arxiv(runtime, arxiv).await;
    }
    if let Some(url) = args.url.as_deref() {
        if let Some(doi) = extract_doi_like(url) {
            return resolve_paper_by_doi(runtime, doi.as_str()).await;
        }
        if let Some(arxiv) = extract_arxiv_like(url) {
            return resolve_paper_by_arxiv(runtime, arxiv.as_str()).await;
        }
        return resolve_paper_by_url(runtime, url).await;
    }

    let query = args.query.as_deref().context("missing add-paper query")?;
    resolve_paper_by_query(runtime, query).await
}

async fn resolve_paper_by_doi(runtime: &ZoteroRuntime, doi: &str) -> Result<ResolvedExternalPaper> {
    let normalized_doi =
        extract_doi_like(doi).with_context(|| format!("invalid DOI `{doi}` for add-paper"))?;
    let mut warnings = Vec::new();
    let search = search_external_papers(runtime, &normalized_doi, Some("openalex"), 10).await?;
    warnings.extend(search.warnings);
    if let Some(paper) = find_exact_doi_match(search.papers.as_slice(), normalized_doi.as_str()) {
        return Ok(ResolvedExternalPaper {
            paper: paper.clone(),
            warnings,
        });
    }

    let paper = runtime
        .toolkit
        .paper_get_metadata(normalized_doi.as_str())
        .await
        .with_context(|| {
            semantic_scholar_fallback_context("DOI", normalized_doi.as_str(), &warnings)
        })?;

    Ok(ResolvedExternalPaper { paper, warnings })
}

async fn resolve_paper_by_arxiv(
    runtime: &ZoteroRuntime,
    arxiv: &str,
) -> Result<ResolvedExternalPaper> {
    let normalized_arxiv = extract_arxiv_like(arxiv)
        .with_context(|| format!("invalid arXiv ID `{arxiv}` for add-paper"))?;
    let mut warnings = Vec::new();

    let arxiv_search =
        search_external_papers(runtime, &normalized_arxiv, Some("arxiv"), 10).await?;
    warnings.extend(arxiv_search.warnings);
    if let Some(paper) =
        find_exact_arxiv_match(arxiv_search.papers.as_slice(), normalized_arxiv.as_str())
    {
        return Ok(ResolvedExternalPaper {
            paper: paper.clone(),
            warnings,
        });
    }

    let openalex_search =
        search_external_papers(runtime, &normalized_arxiv, Some("openalex"), 10).await?;
    warnings.extend(openalex_search.warnings);
    if let Some(paper) =
        find_exact_arxiv_match(openalex_search.papers.as_slice(), normalized_arxiv.as_str())
    {
        return Ok(ResolvedExternalPaper {
            paper: paper.clone(),
            warnings,
        });
    }

    let paper = runtime
        .toolkit
        .paper_get_metadata(normalized_arxiv.as_str())
        .await
        .with_context(|| {
            semantic_scholar_fallback_context("arXiv ID", normalized_arxiv.as_str(), &warnings)
        })?;

    Ok(ResolvedExternalPaper { paper, warnings })
}

async fn resolve_paper_by_url(runtime: &ZoteroRuntime, url: &str) -> Result<ResolvedExternalPaper> {
    let search = search_external_papers(runtime, url, None, 5).await?;
    let paper = find_exact_url_match(search.papers.as_slice(), url)
        .ok_or_else(|| exact_url_match_error(url, search.warnings.as_slice()))?;

    Ok(ResolvedExternalPaper {
        paper: paper.clone(),
        warnings: search.warnings,
    })
}

async fn resolve_paper_by_query(
    runtime: &ZoteroRuntime,
    query: &str,
) -> Result<ResolvedExternalPaper> {
    let search = search_external_papers(runtime, query, None, 5).await?;
    let paper = choose_best_paper(query, search.papers.as_slice())
        .ok_or_else(|| no_paper_match_error(query, search.warnings.as_slice()))?;

    Ok(ResolvedExternalPaper {
        paper: paper.clone(),
        warnings: search.warnings,
    })
}

async fn search_external_papers(
    runtime: &ZoteroRuntime,
    query: &str,
    source: Option<&str>,
    limit: u32,
) -> Result<SearchResult> {
    runtime
        .toolkit
        .paper_search(PaperSearchParams {
            query: query.to_string(),
            year_from: None,
            year_to: None,
            fields_of_study: None,
            source: source.map(ToString::to_string),
            sort_by: None,
            offset: Some(0),
            limit: Some(limit),
            include_abstract: Some(true),
            fields: None,
            max_chars_per_item: None,
        })
        .await
        .map_err(Into::into)
}

fn semantic_scholar_fallback_context(kind: &str, value: &str, warnings: &[String]) -> String {
    if warnings.is_empty() {
        return format!("Semantic Scholar fallback failed for {kind} `{value}`");
    }

    format!(
        "Semantic Scholar fallback failed for {kind} `{value}` after prior warnings: {}",
        warnings.join("; ")
    )
}

fn no_paper_match_error(query: &str, warnings: &[String]) -> anyhow::Error {
    if warnings.is_empty() {
        anyhow!("no paper result matched `{query}`")
    } else {
        anyhow!(
            "no paper result matched `{query}`; warnings: {}",
            warnings.join("; ")
        )
    }
}

fn exact_url_match_error(url: &str, warnings: &[String]) -> anyhow::Error {
    if warnings.is_empty() {
        anyhow!("no paper result matched URL `{url}` exactly")
    } else {
        anyhow!(
            "no paper result matched URL `{url}` exactly; warnings: {}",
            warnings.join("; ")
        )
    }
}

fn find_exact_doi_match<'a>(papers: &'a [Paper], doi: &str) -> Option<&'a Paper> {
    papers.iter().find(|paper| {
        paper
            .doi
            .as_deref()
            .is_some_and(|candidate| normalize_doi_like(candidate) == doi)
    })
}

fn find_exact_arxiv_match<'a>(papers: &'a [Paper], arxiv_id: &str) -> Option<&'a Paper> {
    papers.iter().find(|paper| {
        paper
            .arxiv_id
            .as_deref()
            .is_some_and(|candidate| normalize_arxiv_like(candidate) == arxiv_id)
    })
}

fn find_exact_url_match<'a>(papers: &'a [Paper], url: &str) -> Option<&'a Paper> {
    let expected = url.trim();
    papers.iter().find(|paper| {
        paper
            .url
            .as_deref()
            .is_some_and(|candidate| candidate.trim() == expected)
            || paper
                .pdf_url
                .as_deref()
                .is_some_and(|candidate| candidate.trim() == expected)
    })
}

fn choose_best_paper<'a>(query: &str, papers: &'a [Paper]) -> Option<&'a Paper> {
    let normalized_query = normalize_title_key(query);
    papers.iter().max_by_key(|paper| {
        let normalized_title = normalize_title_key(&paper.title);
        let mut score = 0_u32;
        if normalized_title == normalized_query {
            score += 500;
        }
        if normalized_title.contains(&normalized_query)
            || normalized_query.contains(&normalized_title)
        {
            score += 200;
        }
        if paper.pdf_url.is_some() {
            score += 50;
        }
        if paper.code_url.is_some() {
            score += 25;
        }
        score
    })
}

async fn find_existing_paper_in_scope(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    paper: &Paper,
) -> Result<Option<ZoteroItemDetail>> {
    let mut queries = Vec::new();
    if let Some(doi) = paper.doi.as_deref() {
        queries.push(doi.to_string());
    }
    if let Some(arxiv) = paper.arxiv_id.as_deref() {
        queries.push(arxiv.to_string());
    }
    queries.push(paper.title.clone());

    for query in queries {
        let result = runtime
            .toolkit
            .zotero_search(ZoteroSearchParams {
                query,
                library_type: scope.library_type.clone(),
                library_id: scope.library_id.clone(),
                offset: None,
                limit: Some(10),
                item_type: None,
                qmode: Some(ZoteroQuickSearchMode::TitleCreatorYear),
                max_chars_per_item: None,
            })
            .await?;
        for item in result.items {
            let detail = fetch_item_detail(runtime, scope, &item.key, false, false).await?;
            if paper_matches_item(paper, &detail) {
                return Ok(Some(detail));
            }
        }
    }
    Ok(None)
}

fn paper_matches_item(paper: &Paper, item: &ZoteroItemDetail) -> bool {
    if let (Some(left), Some(right)) = (paper.doi.as_deref(), item.doi.as_deref())
        && normalize_doi_like(left) == normalize_doi_like(right)
    {
        return true;
    }
    if let Some(arxiv) = paper.arxiv_id.as_deref() {
        let normalized_arxiv = normalize_arxiv_like(arxiv);
        if item
            .url
            .as_deref()
            .and_then(extract_arxiv_like)
            .is_some_and(|candidate| candidate == normalized_arxiv)
        {
            return true;
        }
        if item
            .extra
            .as_deref()
            .and_then(extract_arxiv_like)
            .is_some_and(|candidate| candidate == normalized_arxiv)
        {
            return true;
        }
    }
    normalize_title_key(&paper.title) == normalize_title_key(&item.title)
}

fn paper_to_zotero_item(paper: &Paper) -> Value {
    let url = paper
        .url
        .clone()
        .or_else(|| {
            paper
                .arxiv_id
                .as_ref()
                .map(|id| format!("https://arxiv.org/abs/{id}"))
        })
        .unwrap_or_default();
    let item_type = if paper.arxiv_id.is_some() || url.contains("arxiv.org") {
        "preprint"
    } else {
        "journalArticle"
    };
    let mut item = serde_json::json!({
        "itemType": item_type,
        "title": paper.title,
        "creators": creators_from_authors(&paper.authors),
        "abstractNote": paper.abstract_text.clone().unwrap_or_default(),
        "url": url,
    });
    if let Some(year) = paper.year {
        item["date"] = Value::String(year.to_string());
    }
    if let Some(doi) = paper.doi.as_ref() {
        item["DOI"] = Value::String(doi.clone());
    }
    if item_type != "preprint"
        && let Some(venue) = paper.venue.as_ref()
    {
        item["publicationTitle"] = Value::String(venue.clone());
    }
    if let Some(arxiv_id) = paper.arxiv_id.as_ref() {
        item["extra"] = Value::String(format!("arXiv: {arxiv_id}"));
    }
    item
}

fn creators_from_authors(authors: &str) -> Value {
    let parts = if authors.contains(" and ") {
        authors
            .split(" and ")
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    } else {
        authors
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    Value::Array(
        parts
            .into_iter()
            .map(|name| serde_json::json!({ "name": name, "creatorType": "author" }))
            .collect(),
    )
}

async fn ensure_pdf_attachment(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    item: &ZoteroItemDetail,
    paper: &Paper,
) -> Result<ZoteroAddPaperAttachmentResult> {
    if let Some(existing) = item.attachments.as_ref().and_then(|attachments| {
        attachments.iter().find(|attachment| {
            attachment
                .content_type
                .as_deref()
                .is_some_and(|content_type| content_type.eq_ignore_ascii_case("application/pdf"))
                && attachment
                    .link_mode
                    .as_deref()
                    .is_some_and(|mode| mode.starts_with("imported"))
        })
    }) {
        let (status, warning) = match promote_pdf_attachment(runtime, scope, existing).await {
            Ok(true) => ("existing_promoted".to_string(), None),
            Ok(false) => ("existing".to_string(), None),
            Err(err) => (
                "existing".to_string(),
                Some(format!("PDF attachment promotion failed: {err}")),
            ),
        };
        return Ok(ZoteroAddPaperAttachmentResult {
            item_key: Some(existing.key.clone()),
            status,
            url: existing.url.clone(),
            warning,
        });
    }

    let Some(pdf_url) = paper_preferred_pdf_url(paper) else {
        return Ok(ZoteroAddPaperAttachmentResult {
            item_key: None,
            status: "missing".to_string(),
            url: None,
            warning: Some("no PDF URL could be resolved".to_string()),
        });
    };

    match runtime
        .toolkit
        .zotero_create_attachment_import_url(ZoteroCreateAttachmentImportUrlParams {
            parent_item_key: item.key.clone(),
            title: "PDF".to_string(),
            url: pdf_url.clone(),
            content_type: Some("application/pdf".to_string()),
            filename: Some(pdf_filename_for_paper(paper)),
            tags: None,
            library_type: scope.library_type.clone(),
            library_id: scope.library_id.clone(),
        })
        .await
    {
        Ok(result) => {
            let record = result
                .records
                .into_iter()
                .next()
                .context("imported attachment creation returned no record")?;
            Ok(ZoteroAddPaperAttachmentResult {
                item_key: Some(record.key),
                status: "imported".to_string(),
                url: Some(pdf_url),
                warning: None,
            })
        }
        Err(import_err) => {
            let linked = runtime
                .toolkit
                .zotero_create_attachment_link(ZoteroCreateAttachmentLinkParams {
                    parent_item_key: item.key.clone(),
                    title: "PDF".to_string(),
                    url: pdf_url.clone(),
                    content_type: Some("application/pdf".to_string()),
                    collections: None,
                    tags: None,
                    library_type: scope.library_type.clone(),
                    library_id: scope.library_id.clone(),
                })
                .await?;
            let record = linked
                .records
                .into_iter()
                .next()
                .context("linked attachment creation returned no record")?;
            Ok(ZoteroAddPaperAttachmentResult {
                item_key: Some(record.key),
                status: "linked_fallback".to_string(),
                url: Some(pdf_url),
                warning: Some(import_err.to_string()),
            })
        }
    }
}

async fn promote_pdf_attachment(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    attachment: &codex_research_tools::types::ZoteroAttachment,
) -> Result<bool> {
    let Some(title) = attachment.title.as_deref() else {
        return Ok(false);
    };
    if title != "Preprint PDF" {
        return Ok(false);
    }

    runtime
        .toolkit
        .zotero_update_items(ZoteroUpdateItemsParams {
            items: vec![codex_research_tools::types::ZoteroItemUpdatePayload {
                item_key: attachment.key.clone(),
                patch: serde_json::json!({
                    "title": "PDF"
                }),
            }],
            library_type: scope.library_type.clone(),
            library_id: scope.library_id.clone(),
        })
        .await?;
    Ok(true)
}

struct RepoLinkRequest<'a> {
    repo_collection_name: &'a str,
    paper_key: &'a str,
    paper_title: &'a str,
    paper: &'a Paper,
    warnings: &'a mut Vec<String>,
}

async fn ensure_repo_link(
    runtime: &ZoteroRuntime,
    target_scope: &ZoteroScopeRef,
    scope: &LibraryScopeArgs,
    request: RepoLinkRequest<'_>,
) -> Result<Option<ZoteroAddPaperRepoResult>> {
    let Some(repo_url) = request
        .paper
        .code_url
        .as_deref()
        .and_then(normalize_repo_url)
    else {
        return Ok(None);
    };
    let repo_collection = runtime
        .toolkit
        .zotero_find_or_create_collection(ZoteroFindOrCreateCollectionParams {
            name: request.repo_collection_name.to_string(),
            parent_collection_key: None,
            library_type: Some(target_scope.library_type.clone()),
            library_id: Some(target_scope.library_id.clone()),
        })
        .await?;
    let existing = find_existing_repo_in_scope(runtime, scope, repo_url.as_str()).await?;
    let (repo_key, repo_title, status) = if let Some(existing) = existing {
        (existing.key, existing.title, "linked_existing".to_string())
    } else {
        let created = runtime
            .toolkit
            .zotero_create_items(ZoteroCreateItemsParams {
                items: vec![serde_json::json!({
                    "itemType": "computerProgram",
                    "title": repo_title_from_url(repo_url.as_str()),
                    "abstractNote": format!(
                        "Official code repository for {}.",
                        request.paper_title
                    ),
                    "url": repo_url.clone(),
                })],
                library_type: Some(target_scope.library_type.clone()),
                library_id: Some(target_scope.library_id.clone()),
            })
            .await?;
        let record = created
            .records
            .into_iter()
            .next()
            .context("repo item creation returned no record")?;
        (
            record.key,
            record
                .title
                .unwrap_or_else(|| repo_title_from_url(repo_url.as_str())),
            "created_and_linked".to_string(),
        )
    };
    runtime
        .toolkit
        .zotero_add_items_to_collection(ZoteroAddItemsToCollectionParams {
            collection_key: repo_collection.collection.key.clone(),
            item_keys: vec![repo_key.clone()],
            library_type: Some(target_scope.library_type.clone()),
            library_id: Some(target_scope.library_id.clone()),
        })
        .await?;
    ensure_bidirectional_relation(runtime, scope, request.paper_key, &repo_key, target_scope)
        .await?;
    if !repo_url.contains("github.com") && !repo_url.contains("gitlab.com") {
        request.warnings.push(format!(
            "linked repository URL does not look like a standard Git host: {repo_url}"
        ));
    }
    Ok(Some(ZoteroAddPaperRepoResult {
        item_key: repo_key,
        title: repo_title,
        url: repo_url,
        status,
        collection_key: repo_collection.collection.key,
    }))
}

async fn find_existing_repo_in_scope(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    repo_url: &str,
) -> Result<Option<ZoteroItemDetail>> {
    let query = repo_title_from_url(repo_url);
    let result = runtime
        .toolkit
        .zotero_search(ZoteroSearchParams {
            query,
            library_type: scope.library_type.clone(),
            library_id: scope.library_id.clone(),
            offset: None,
            limit: Some(10),
            item_type: Some("computerProgram".to_string()),
            qmode: Some(ZoteroQuickSearchMode::TitleCreatorYear),
            max_chars_per_item: None,
        })
        .await?;
    for item in result.items {
        let detail = fetch_item_detail(runtime, scope, &item.key, false, false).await?;
        if detail
            .url
            .as_deref()
            .and_then(normalize_repo_url)
            .as_deref()
            == Some(repo_url)
        {
            return Ok(Some(detail));
        }
    }
    Ok(None)
}

async fn ensure_bidirectional_relation(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    left_key: &str,
    right_key: &str,
    target_scope: &ZoteroScopeRef,
) -> Result<()> {
    ensure_relation(runtime, scope, left_key, right_key, target_scope).await?;
    ensure_relation(runtime, scope, right_key, left_key, target_scope).await?;
    Ok(())
}

async fn ensure_relation(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    item_key: &str,
    related_key: &str,
    target_scope: &ZoteroScopeRef,
) -> Result<()> {
    let detail = fetch_item_detail(runtime, scope, item_key, false, false).await?;
    let target_uri = raw_item_uri(target_scope, related_key);
    if detail
        .linked_items
        .iter()
        .any(|linked| linked.relation == "dc:relation" && linked.raw_uri == target_uri)
    {
        return Ok(());
    }
    let mut relations = detail
        .linked_items
        .iter()
        .filter(|linked| linked.relation == "dc:relation")
        .map(|linked| linked.raw_uri.clone())
        .collect::<Vec<_>>();
    relations.push(target_uri);
    relations.sort();
    relations.dedup();
    runtime
        .toolkit
        .zotero_update_items(ZoteroUpdateItemsParams {
            items: vec![codex_research_tools::types::ZoteroItemUpdatePayload {
                item_key: item_key.to_string(),
                patch: serde_json::json!({
                    "relations": {
                        "dc:relation": relations
                    }
                }),
            }],
            library_type: scope.library_type.clone(),
            library_id: scope.library_id.clone(),
        })
        .await?;
    Ok(())
}

fn paper_preferred_pdf_url(paper: &Paper) -> Option<String> {
    paper
        .pdf_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .filter(|url| looks_like_pdf_url(url))
        .map(ToString::to_string)
        .or_else(|| {
            paper
                .arxiv_id
                .as_ref()
                .map(|id| format!("https://arxiv.org/pdf/{id}.pdf"))
        })
}

fn looks_like_pdf_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("arxiv.org/pdf/")
        || lower.ends_with(".pdf")
        || lower.contains("/pdf/")
        || lower.contains("/pdf?")
        || lower.contains("download=1")
}

fn pdf_filename_for_paper(paper: &Paper) -> String {
    if let Some(arxiv_id) = paper.arxiv_id.as_deref() {
        return format!("{arxiv_id}.pdf");
    }

    let stem = paper
        .title
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .take(10)
        .collect::<Vec<_>>()
        .join(" ");
    if stem.is_empty() {
        "paper.pdf".to_string()
    } else {
        format!("{stem}.pdf")
    }
}

fn repo_title_from_url(repo_url: &str) -> String {
    normalize_repo_url(repo_url)
        .and_then(|normalized| {
            normalized
                .trim_start_matches("https://")
                .split_once('/')
                .map(|(_, repo)| repo.to_string())
        })
        .unwrap_or_else(|| repo_url.to_string())
}

fn normalize_title_key(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_doi_like(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("doi:")
        .to_string()
}

fn extract_doi_like(value: &str) -> Option<String> {
    let normalized = normalize_doi_like(value);
    normalized.starts_with("10.").then_some(normalized)
}

fn normalize_arxiv_like(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("arxiv:")
        .trim_start_matches("arXiv:")
        .trim_start_matches("https://arxiv.org/abs/")
        .trim_start_matches("http://arxiv.org/abs/")
        .trim_start_matches("https://arxiv.org/pdf/")
        .trim_start_matches("http://arxiv.org/pdf/")
        .trim_end_matches(".pdf")
        .to_string()
}

fn extract_arxiv_like(value: &str) -> Option<String> {
    let normalized = normalize_arxiv_like(value);
    let trimmed = normalized
        .split('?')
        .next()
        .unwrap_or(normalized.as_str())
        .to_string();
    (!trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ['.', '-', 'v'].contains(&ch)))
    .then_some(trimmed)
}

async fn find_repos_with_fallback(
    context: &ZoteroCliContext,
    args: &FindReposArgs,
) -> Result<ZoteroFindReposResult> {
    match find_repos_once(&context.primary, args).await {
        Ok(primary) if !primary.repos.is_empty() || context.alternate.is_none() => Ok(primary),
        Ok(primary) => {
            let Some(alternate_runtime) = context.alternate.as_ref() else {
                return Ok(primary);
            };
            let mut alternate = find_repos_once(alternate_runtime, args).await?;
            if alternate.repos.is_empty() {
                return Ok(primary);
            }
            alternate.fallback_used = true;
            alternate.warnings.insert(
                0,
                format!(
                    "No repository URLs were found via {} mode, so the CLI retried via {} mode.",
                    context.primary.mode, alternate_runtime.mode
                ),
            );
            Ok(alternate)
        }
        Err(primary_error) => {
            let Some(alternate_runtime) = context.alternate.as_ref() else {
                return Err(primary_error);
            };
            let mut alternate = find_repos_once(alternate_runtime, args).await?;
            alternate.fallback_used = true;
            alternate.warnings.insert(
                0,
                format!(
                    "{} mode failed with: {}. The CLI retried via {} mode.",
                    context.primary.mode, primary_error, alternate_runtime.mode
                ),
            );
            Ok(alternate)
        }
    }
}

async fn find_repos_once(
    runtime: &ZoteroRuntime,
    args: &FindReposArgs,
) -> Result<ZoteroFindReposResult> {
    if args.query.is_none() && args.collection.is_none() {
        bail!("provide at least one of `--query` or `--collection`");
    }

    let mut warnings = Vec::new();
    let mut matched_collections = Vec::new();
    let mut collections_cache = None;
    let mut candidates = Vec::new();

    if let Some(collection_ref) = args.collection.as_deref() {
        let collections = fetch_all_collections(runtime, &args.scope, 300).await?;
        let resolved = resolve_collection_reference(
            collections.as_slice(),
            collection_ref,
            explicit_scope(&args.scope).as_ref(),
        );
        collections_cache = Some(collections);
        if resolved.is_empty() {
            warnings.push(format!("No Zotero collection matched `{collection_ref}`."));
        } else {
            matched_collections.extend(resolved.clone());
            for collection in resolved.iter().take(3) {
                let page = runtime
                    .toolkit
                    .zotero_get_collection_items(ZoteroCollectionItemsParams {
                        collection_key: collection.key.clone(),
                        library_type: args.scope.library_type.clone(),
                        library_id: args.scope.library_id.clone(),
                        offset: None,
                        limit: Some(args.inspect_limit.clamp(1, 100)),
                        item_type: None,
                        max_chars_per_item: None,
                    })
                    .await?;
                candidates.extend(page.items);
            }
        }
    }

    if let Some(query) = args.query.as_deref() {
        let collections = match collections_cache {
            Some(ref collections) => collections.clone(),
            None => fetch_all_collections(runtime, &args.scope, 300).await?,
        };
        let collection_matches = score_matching_collections(
            collections.as_slice(),
            query,
            explicit_scope(&args.scope).as_ref(),
            3,
        );
        matched_collections.extend(collection_matches.clone());
        for collection in collection_matches.iter().take(3) {
            let page = runtime
                .toolkit
                .zotero_get_collection_items(ZoteroCollectionItemsParams {
                    collection_key: collection.key.clone(),
                    library_type: args.scope.library_type.clone(),
                    library_id: args.scope.library_id.clone(),
                    offset: None,
                    limit: Some(args.inspect_limit.clamp(1, 100)),
                    item_type: None,
                    max_chars_per_item: None,
                })
                .await?;
            candidates.extend(page.items);
        }
        candidates.extend(
            search_repo_candidates(runtime, &args.scope, query, args.inspect_limit.clamp(1, 50))
                .await?,
        );
    }

    let matched_collections = dedup_collection_matches(matched_collections);
    let seed_candidates = candidates.clone();
    let candidates = rank_candidate_items(candidates);
    let mut repos = Vec::new();
    let mut seen_repo_urls = BTreeSet::new();
    let mut seen_item_keys = BTreeSet::new();
    let mut repo_state = RepoDiscoveryState {
        repos: &mut repos,
        seen_repo_urls: &mut seen_repo_urls,
        seen_item_keys: &mut seen_item_keys,
        limit: args.limit.clamp(1, 50) as usize,
    };
    for candidate in candidates
        .iter()
        .take(args.inspect_limit.clamp(1, 50) as usize)
    {
        inspect_item_for_repo_urls(
            runtime,
            &args.scope,
            candidate.key.as_str(),
            None,
            0,
            &mut repo_state,
        )
        .await?;
        if repo_state.is_full() {
            break;
        }
    }

    if repo_state.repos.is_empty()
        && args.query.is_none()
        && args.collection.is_some()
        && !matched_collections.is_empty()
    {
        let fallback_queries = build_repo_discovery_queries(
            matched_collections.as_slice(),
            seed_candidates.as_slice(),
        );
        warnings.push(
            "No repository URLs were found by direct collection inspection, so the CLI searched for related webpage and attachment items using matched collection names, inspected paper titles, and strongest tokens."
                .to_string(),
        );

        let mut fallback_candidates = Vec::new();
        for query in fallback_queries {
            fallback_candidates.extend(
                search_repo_candidates(runtime, &args.scope, query.as_str(), args.inspect_limit)
                    .await?,
            );
        }

        let fallback_candidates = rank_candidate_items(fallback_candidates);
        for candidate in fallback_candidates
            .iter()
            .take(args.inspect_limit.clamp(1, 50) as usize)
        {
            inspect_item_for_repo_urls(
                runtime,
                &args.scope,
                candidate.key.as_str(),
                None,
                0,
                &mut repo_state,
            )
            .await?;
            if repo_state.is_full() {
                break;
            }
        }
    }

    Ok(ZoteroFindReposResult {
        effective_mode: runtime.mode,
        fallback_used: false,
        warnings,
        matched_collections,
        repos,
    })
}

async fn search_repo_candidates(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    query: &str,
    limit: u32,
) -> Result<Vec<ZoteroItem>> {
    let mut candidates = Vec::new();
    let limit = limit.clamp(1, 50);
    for item_type in [Some("webpage"), Some("attachment"), None] {
        let search = runtime
            .toolkit
            .zotero_search(ZoteroSearchParams {
                query: query.to_string(),
                library_type: scope.library_type.clone(),
                library_id: scope.library_id.clone(),
                offset: None,
                limit: Some(limit),
                item_type: item_type.map(ToString::to_string),
                qmode: Some(ZoteroQuickSearchMode::Everything),
                max_chars_per_item: None,
            })
            .await?;
        candidates.extend(search.items);
    }
    Ok(rank_candidate_items(candidates))
}

fn build_repo_discovery_queries(
    matched_collections: &[ZoteroCollectionMatch],
    seed_items: &[ZoteroItem],
) -> Vec<String> {
    let mut queries = BTreeSet::new();
    for collection in matched_collections.iter().take(3) {
        queries.insert(collection.name.clone());
        for token in tokenize_query(collection.name.as_str()) {
            if token.len() >= 5 {
                queries.insert(token.to_string());
            }
        }
    }

    for item in rank_candidate_items(seed_items.to_vec())
        .into_iter()
        .take(5)
    {
        let title = item.title;
        if !title.trim().is_empty() {
            queries.insert(title.clone());
        }
        for token in tokenize_query(title.as_str()) {
            if token.len() >= 6 {
                queries.insert(token.to_string());
            }
        }
    }

    queries.into_iter().take(12).collect()
}

struct RepoDiscoveryState<'a> {
    repos: &'a mut Vec<ZoteroRepoMatch>,
    seen_repo_urls: &'a mut BTreeSet<String>,
    seen_item_keys: &'a mut BTreeSet<String>,
    limit: usize,
}

impl RepoDiscoveryState<'_> {
    fn is_full(&self) -> bool {
        self.repos.len() >= self.limit
    }

    fn append_repo_match(
        &mut self,
        item_key: &str,
        title: &str,
        parent_item: Option<&str>,
        discovered_via: &str,
        raw_url: Option<&str>,
    ) {
        if self.is_full() {
            return;
        }
        let Some(raw_url) = raw_url else {
            return;
        };
        let Some(repo_url) = normalize_repo_url(raw_url) else {
            return;
        };
        if self.seen_repo_urls.insert(repo_url.clone()) {
            self.repos.push(ZoteroRepoMatch {
                item_key: item_key.to_string(),
                title: title.to_string(),
                repo_url,
                discovered_via: discovered_via.to_string(),
                parent_item: parent_item.map(ToString::to_string),
            });
        }
    }

    fn append_repo_urls_from_text(
        &mut self,
        item_key: &str,
        title: &str,
        parent_item: Option<&str>,
        discovered_via: &str,
        text: &str,
    ) {
        if self.is_full() {
            return;
        }

        for repo_url in extract_repo_urls_from_text(text) {
            if self.is_full() {
                break;
            }
            if self.seen_repo_urls.insert(repo_url.clone()) {
                self.repos.push(ZoteroRepoMatch {
                    item_key: item_key.to_string(),
                    title: title.to_string(),
                    repo_url,
                    discovered_via: discovered_via.to_string(),
                    parent_item: parent_item.map(ToString::to_string),
                });
            }
        }
    }
}

async fn inspect_item_for_repo_urls(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    item_key: &str,
    parent_item: Option<&str>,
    depth: u8,
    state: &mut RepoDiscoveryState<'_>,
) -> Result<()> {
    let mut pending = vec![(
        item_key.to_string(),
        parent_item.map(ToString::to_string),
        depth,
    )];
    while let Some((current_item_key, current_parent_item, current_depth)) = pending.pop() {
        if state.is_full() || !state.seen_item_keys.insert(current_item_key.clone()) {
            continue;
        }

        let detail = fetch_item_detail(runtime, scope, &current_item_key, true, false).await?;
        state.append_repo_match(
            detail.key.as_str(),
            detail.title.as_str(),
            current_parent_item.as_deref(),
            "item_url",
            detail.url.as_deref(),
        );
        if let Some(abstract_text) = detail.abstract_text.as_deref() {
            state.append_repo_urls_from_text(
                detail.key.as_str(),
                detail.title.as_str(),
                current_parent_item.as_deref(),
                "abstract_text",
                abstract_text,
            );
        }
        if let Some(extra) = detail.extra.as_deref() {
            state.append_repo_urls_from_text(
                detail.key.as_str(),
                detail.title.as_str(),
                current_parent_item.as_deref(),
                "extra",
                extra,
            );
        }
        if let Some(attachments) = detail.attachments.as_ref() {
            for attachment in attachments {
                state.append_repo_match(
                    detail.key.as_str(),
                    detail.title.as_str(),
                    current_parent_item.as_deref(),
                    "attachment_url",
                    attachment.url.as_deref(),
                );
            }
        }

        if current_depth >= 1 || state.is_full() {
            continue;
        }

        for linked in detail.linked_items.iter().rev() {
            if let Some(linked_key) = linked.item_key.as_deref() {
                pending.push((
                    linked_key.to_string(),
                    Some(detail.key.clone()),
                    current_depth + 1,
                ));
            }
        }
    }

    Ok(())
}

fn extract_repo_urls_from_text(text: &str) -> Vec<String> {
    let mut urls = BTreeSet::new();
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "https://www.github.com/",
        "http://www.github.com/",
        "https://gitlab.com/",
        "http://gitlab.com/",
        "https://www.gitlab.com/",
        "http://www.gitlab.com/",
    ] {
        let mut remainder = text;
        while let Some(index) = remainder.find(prefix) {
            let candidate = &remainder[index..];
            let end = candidate
                .find(|character: char| {
                    character.is_whitespace()
                        || ['"', '\'', ')', ']', '}', '>', ','].contains(&character)
                })
                .unwrap_or(candidate.len());
            if let Some(repo_url) = normalize_repo_url(&candidate[..end]) {
                urls.insert(repo_url);
            }
            remainder = &candidate[end..];
        }
    }
    urls.into_iter().collect()
}

fn normalize_repo_url(raw_url: &str) -> Option<String> {
    let trimmed = raw_url.trim().trim_end_matches(|character: char| {
        ['.', ',', ';', ':', ')', ']', '}'].contains(&character)
    });
    for (host, prefixes) in [
        (
            "github.com",
            [
                "https://github.com/",
                "http://github.com/",
                "https://www.github.com/",
                "http://www.github.com/",
            ],
        ),
        (
            "gitlab.com",
            [
                "https://gitlab.com/",
                "http://gitlab.com/",
                "https://www.gitlab.com/",
                "http://www.gitlab.com/",
            ],
        ),
    ] {
        for prefix in prefixes {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let mut segments = rest
                    .split('/')
                    .take(2)
                    .map(str::trim)
                    .filter(|segment| !segment.is_empty())
                    .collect::<Vec<_>>();
                if segments.len() == 2 {
                    let repo = segments.pop()?.trim_end_matches(".git");
                    let owner = segments.pop()?;
                    return Some(format!("https://{host}/{owner}/{repo}"));
                }
            }
        }
    }
    None
}

fn scope_from_canonical_id(canonical_id: &str) -> Option<ZoteroScopeRef> {
    let (prefix, rest) = canonical_id.split_once(':')?;
    if prefix != "zotero" {
        return None;
    }
    let mut parts = rest.split('/');
    let scope_type = parts.next()?;
    let library_id = parts.next()?;
    let library_type = match scope_type {
        "user" => "user",
        "group" => "group",
        _ => return None,
    };
    Some(ZoteroScopeRef {
        library_type: library_type.to_string(),
        library_id: library_id.to_string(),
    })
}

fn scope_from_collection(collection: &ZoteroCollection) -> Option<ZoteroScopeRef> {
    collection
        .source_meta
        .as_ref()
        .and_then(|meta| meta.canonical_id.as_deref())
        .and_then(scope_from_canonical_id)
}

fn scope_from_item(item: &ZoteroItem) -> Option<ZoteroScopeRef> {
    item.source_meta
        .as_ref()
        .and_then(|meta| meta.canonical_id.as_deref())
        .and_then(scope_from_canonical_id)
}

fn scope_label(scope: &ZoteroScopeRef) -> String {
    format!("{}/{}", scope.library_type, scope.library_id)
}

fn raw_item_uri(scope: &ZoteroScopeRef, item_key: &str) -> String {
    match scope.library_type.as_str() {
        "group" => format!(
            "http://zotero.org/groups/{}/items/{item_key}",
            scope.library_id
        ),
        _ => format!(
            "http://zotero.org/users/{}/items/{item_key}",
            scope.library_id
        ),
    }
}

fn scope_args(scope: &ZoteroScopeRef) -> LibraryScopeArgs {
    LibraryScopeArgs {
        library_type: Some(scope.library_type.clone()),
        library_id: Some(scope.library_id.clone()),
    }
}

fn explicit_scope(scope: &LibraryScopeArgs) -> Option<ZoteroScopeRef> {
    Some(ZoteroScopeRef {
        library_type: scope.library_type.clone()?,
        library_id: scope.library_id.clone()?,
    })
}

async fn fetch_all_collections(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    hard_limit: u32,
) -> Result<Vec<ZoteroCollection>> {
    let mut collections = Vec::new();
    let mut offset = 0_u32;
    let page_limit = hard_limit.clamp(1, 100);

    loop {
        let page = runtime
            .toolkit
            .zotero_get_collections(ZoteroCollectionsParams {
                library_type: scope.library_type.clone(),
                library_id: scope.library_id.clone(),
                offset: Some(offset),
                limit: Some(page_limit),
            })
            .await?;
        let fetched = u32::try_from(page.collections.len()).unwrap_or(0);
        collections.extend(page.collections);
        if !page.has_more || fetched == 0 || collections.len() >= hard_limit as usize {
            break;
        }
        offset = offset.saturating_add(fetched);
    }

    collections.truncate(hard_limit as usize);
    Ok(collections)
}

async fn fetch_item_detail(
    runtime: &ZoteroRuntime,
    scope: &LibraryScopeArgs,
    item_key: &str,
    include_attachments: bool,
    include_fulltext_resolution: bool,
) -> Result<ZoteroItemDetail> {
    Ok(runtime
        .toolkit
        .zotero_get_item(ZoteroItemParams {
            item_key: item_key.to_string(),
            library_type: scope.library_type.clone(),
            library_id: scope.library_id.clone(),
            max_chars_per_item: None,
            include_attachments: Some(include_attachments),
            include_fulltext_resolution: Some(include_fulltext_resolution),
        })
        .await?)
}

fn score_item_detail(item: &ZoteroItemDetail) -> u32 {
    let mut score = 0;
    if item.document_resolution.is_some() {
        score += 200;
    }
    if item.url.is_some() {
        score += 50;
    }
    if item
        .attachments
        .as_ref()
        .is_some_and(|attachments| !attachments.is_empty())
    {
        score += 20;
    }
    score
}

fn score_matching_collections(
    collections: &[ZoteroCollection],
    query: &str,
    requested_scope: Option<&ZoteroScopeRef>,
    limit: usize,
) -> Vec<ZoteroCollectionMatch> {
    let normalized_query = query.trim().to_ascii_lowercase();
    let tokens = tokenize_query(normalized_query.as_str());
    let mut matches = collections
        .iter()
        .filter_map(|collection| {
            let name = collection.name.to_ascii_lowercase();
            let mut score = 0;
            if name == normalized_query {
                score += 500;
            }
            if name.contains(normalized_query.as_str()) {
                score += 220;
            }
            let mut matched_tokens = 0_u32;
            for token in &tokens {
                if name
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .any(|segment| segment == *token)
                {
                    score += 55;
                    matched_tokens += 1;
                } else if name.contains(token) {
                    score += 30;
                    matched_tokens += 1;
                }
            }
            if !tokens.is_empty() && matched_tokens == tokens.len() as u32 {
                score += 90;
            }
            (score > 0).then_some(ZoteroCollectionMatch {
                key: collection.key.clone(),
                name: collection.name.clone(),
                parent_collection: collection.parent_collection.clone(),
                scope: scope_from_collection(collection).or_else(|| requested_scope.cloned()),
                score,
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
    });
    matches.truncate(limit);
    matches
}

fn resolve_collection_reference(
    collections: &[ZoteroCollection],
    collection_ref: &str,
    requested_scope: Option<&ZoteroScopeRef>,
) -> Vec<ZoteroCollectionMatch> {
    if let Some(collection) = collections
        .iter()
        .find(|collection| collection.key == collection_ref)
    {
        return vec![ZoteroCollectionMatch {
            key: collection.key.clone(),
            name: collection.name.clone(),
            parent_collection: collection.parent_collection.clone(),
            scope: scope_from_collection(collection).or_else(|| requested_scope.cloned()),
            score: u32::MAX,
        }];
    }
    let exact_name_matches = collections
        .iter()
        .filter(|collection| collection.name.eq_ignore_ascii_case(collection_ref))
        .map(|collection| ZoteroCollectionMatch {
            key: collection.key.clone(),
            name: collection.name.clone(),
            parent_collection: collection.parent_collection.clone(),
            scope: scope_from_collection(collection).or_else(|| requested_scope.cloned()),
            score: u32::MAX - 1,
        })
        .collect::<Vec<_>>();
    if !exact_name_matches.is_empty() {
        return exact_name_matches;
    }
    score_matching_collections(collections, collection_ref, requested_scope, 5)
}

fn dedup_collection_matches(matches: Vec<ZoteroCollectionMatch>) -> Vec<ZoteroCollectionMatch> {
    let mut seen = BTreeSet::new();
    matches
        .into_iter()
        .filter(|collection| {
            let scope_key = collection
                .scope
                .as_ref()
                .map(scope_label)
                .unwrap_or_default();
            seen.insert(format!("{scope_key}:{}", collection.key))
        })
        .collect()
}

fn rank_candidate_items(items: Vec<ZoteroItem>) -> Vec<ZoteroItem> {
    let mut seen = BTreeSet::new();
    let mut ranked = items
        .into_iter()
        .filter(|item| seen.insert(item.key.clone()))
        .map(|item| {
            let mut score = 0_u32;
            let haystack = format!(
                "{} {} {}",
                item.title.to_ascii_lowercase(),
                item.item_type.to_ascii_lowercase(),
                item.abstract_snippet
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
            );
            if item.item_type.eq_ignore_ascii_case("webpage") {
                score += 200;
            }
            if haystack.contains("github") || haystack.contains("gitlab") {
                score += 150;
            }
            if haystack.contains("repo") || haystack.contains("code") {
                score += 80;
            }
            if !item.linked_items.is_empty() {
                score += 30;
            }
            (score, item)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.title.cmp(&right.1.title))
    });
    ranked.into_iter().map(|(_, item)| item).collect()
}

fn print_status_compact(status: &ZoteroStatusResult) {
    println!("Effective mode: {}", status.effective_mode);
    println!("Base URL: {}", status.zotero_base_url);
    println!(
        "API key configured: {}",
        if status.api_key_configured {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "Library scope: {}",
        status
            .library_type
            .as_deref()
            .zip(status.library_id.as_deref())
            .map_or_else(
                || "all accessible libraries".to_string(),
                |(library_type, library_id)| format!("{library_type}/{library_id}"),
            )
    );
    println!(
        "Default write scope: {}",
        status
            .default_write_library_type
            .as_deref()
            .zip(status.default_write_library_id.as_deref())
            .map_or_else(
                || "unconfigured".to_string(),
                |(library_type, library_id)| format!("{library_type}/{library_id}"),
            )
    );
    if let Some(alternate_mode) = status.alternate_mode {
        println!("Fallback mode: {alternate_mode}");
    }
    for warning in &status.warnings {
        println!("Note: {warning}");
    }
}

fn print_collection_matches_compact(collections: &[ZoteroCollectionMatch], warnings: &[String]) {
    if collections.is_empty() {
        println!("No matching collections.");
    } else {
        for collection in collections {
            let suffix = collection
                .scope
                .as_ref()
                .map(|scope| format!(" [{}]", scope_label(scope)))
                .unwrap_or_default();
            println!("[{}] {}{}", collection.key, collection.name, suffix);
        }
    }
    for warning in warnings {
        println!("Note: {warning}");
    }
}

fn print_items_compact(items: &[ZoteroItem]) {
    if items.is_empty() {
        println!("No items.");
        return;
    }
    for item in items {
        let mut suffix = String::new();
        if let Some(year) = item.year.as_deref() {
            suffix.push_str(&format!(" ({year})"));
        }
        if let Some(scope) = scope_from_item(item).as_ref() {
            suffix.push_str(&format!(" [{}]", scope_label(scope)));
        }
        println!(
            "[{}] [{}] {}{}",
            item.key, item.item_type, item.title, suffix
        );
    }
}

fn print_tags_compact(tags: &[String]) {
    if tags.is_empty() {
        println!("No tags.");
        return;
    }
    for tag in tags {
        println!("{tag}");
    }
}

fn print_groups_compact(groups: &[ZoteroGroup]) {
    if groups.is_empty() {
        println!("No groups.");
        return;
    }
    for group in groups {
        println!("[{}] {}", group.id, group.name);
    }
}

fn print_item_detail_compact(item: &ZoteroItemDetail) {
    println!("[{}] [{}] {}", item.key, item.item_type, item.title);
    if !item.authors.is_empty() {
        println!("Authors: {}", item.authors.join(", "));
    }
    if let Some(date) = item.date.as_deref() {
        println!("Date: {date}");
    }
    if let Some(doi) = item.doi.as_deref() {
        println!("DOI: {doi}");
    }
    if let Some(url) = item.url.as_deref() {
        println!("URL: {url}");
    }
    if !item.tags.is_empty() {
        println!("Tags: {}", item.tags.join(", "));
    }
    if let Some(resolution) = item.document_resolution.as_ref() {
        println!("Document source: {:?}", resolution.source_kind);
        if let Some(url) = resolution.preferred_url.as_deref() {
            println!("Preferred URL: {url}");
        }
        if let Some(path) = resolution.local_path.as_deref() {
            println!("Local path: {path}");
        }
    }
    if let Some(attachments) = item.attachments.as_ref()
        && !attachments.is_empty()
    {
        println!("Attachments:");
        for attachment in attachments {
            let label = attachment
                .title
                .as_deref()
                .or(attachment.filename.as_deref())
                .unwrap_or("attachment");
            if let Some(url) = attachment.url.as_deref() {
                println!("  - [{}] {}", attachment.key, label);
                println!("    {url}");
            } else {
                println!("  - [{}] {}", attachment.key, label);
            }
        }
    }
}

fn print_resolved_paper_compact(result: &ZoteroResolvedPaperResult) {
    println!("Mode: {}", result.effective_mode);
    if result.fallback_used {
        println!("Fallback used: yes");
    }
    if let Some(best_match) = result.best_match.as_ref() {
        println!("Best match:");
        print_item_detail_compact(best_match);
    } else {
        println!("No resolved paper match.");
    }
    if !result.candidates.is_empty() {
        println!("Candidates:");
        print_items_compact(&result.candidates);
    }
    for warning in &result.warnings {
        println!("Note: {warning}");
    }
}

fn print_repo_matches_compact(result: &ZoteroFindReposResult) {
    println!("Mode: {}", result.effective_mode);
    if result.fallback_used {
        println!("Fallback used: yes");
    }
    if !result.matched_collections.is_empty() {
        println!("Matched collections:");
        for collection in &result.matched_collections {
            println!("  - [{}] {}", collection.key, collection.name);
        }
    }
    if result.repos.is_empty() {
        println!("No repository URLs found.");
    } else {
        println!("Repository URLs:");
        for repo in &result.repos {
            println!("  - [{}] {}", repo.item_key, repo.title);
            println!("    {}", repo.repo_url);
            println!("    via {}", repo.discovered_via);
        }
    }
    for warning in &result.warnings {
        println!("Note: {warning}");
    }
}

fn print_add_paper_compact(result: &ZoteroAddPaperResult) {
    println!("Mode: {}", result.effective_mode);
    println!("Scope: {}", scope_label(&result.scope));
    let collection_scope = result
        .collection
        .scope
        .as_ref()
        .map(scope_label)
        .unwrap_or_else(|| scope_label(&result.scope));
    println!(
        "Collection: [{}] {} [{}]",
        result.collection.key, result.collection.name, collection_scope
    );
    println!(
        "Paper: [{}] {} ({})",
        result.paper_key,
        result.paper_title,
        if result.paper_created {
            "created"
        } else {
            "reused"
        }
    );
    println!("PDF: {}", result.pdf.status);
    if let Some(url) = result.pdf.url.as_deref() {
        println!("PDF URL: {url}");
    }
    if let Some(snapshot) = result.snapshot.as_ref() {
        println!("Snapshot: {}", snapshot.status);
    }
    if let Some(repo) = result.repo.as_ref() {
        println!("Repo: [{}] {} ({})", repo.item_key, repo.title, repo.status);
        println!("Repo URL: {}", repo.url);
    }
    for warning in &result.warnings {
        println!("Note: {warning}");
    }
    if let Some(warning) = result.pdf.warning.as_deref() {
        println!("Note: {warning}");
    }
    if let Some(snapshot) = result.snapshot.as_ref()
        && let Some(warning) = snapshot.warning.as_deref()
    {
        println!("Note: {warning}");
    }
}

fn parse_json_payload<T>(args: &JsonPayloadArgs, command_name: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let value = load_json_value(args)
        .with_context(|| format!("load JSON payload for `ata zotero {command_name}`"))?;
    serde_json::from_value(value)
        .with_context(|| format!("parse JSON payload for `ata zotero {command_name}`"))
}

fn load_json_value(args: &JsonPayloadArgs) -> Result<Value> {
    let raw = match (&args.json, &args.json_file) {
        (Some(json), None) => json.clone(),
        (None, Some(path)) => fs::read_to_string(path)
            .with_context(|| format!("read JSON payload from {}", path.display()))?,
        _ => unreachable!("clap ensures exactly one JSON payload source"),
    };
    serde_json::from_str(&raw).context("parse JSON payload")
}

fn parse_optional_enum<T>(value: Option<String>, field_name: &str) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    value
        .map(|raw| {
            serde_json::from_value(Value::String(raw))
                .with_context(|| format!("parse `{field_name}`"))
        })
        .transpose()
}

fn print_pretty_json<T>(value: &T) -> Result<()>
where
    T: Serialize,
{
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[derive(Debug, Clone)]
struct ZoteroCommandCatalogEntry {
    command: &'static str,
    description: &'static str,
    core_args: &'static [&'static str],
    aliases: &'static [&'static str],
    tags: &'static [&'static str],
    examples: &'static [&'static str],
}

fn search_command_catalog(query: &str, limit: usize) -> Vec<&'static ZoteroCommandCatalogEntry> {
    let normalized_query = query.trim().to_lowercase();
    let tokens = tokenize_query(&normalized_query);
    let mut matches = zotero_command_catalog()
        .iter()
        .filter_map(|entry| {
            let score = score_catalog_entry(entry, &normalized_query, &tokens);
            (score > 0).then_some((score, entry))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.command.cmp(right.1.command))
    });
    matches
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry)
        .collect()
}

fn tokenize_query(query: &str) -> Vec<&str> {
    query
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect()
}

fn score_catalog_entry(
    entry: &ZoteroCommandCatalogEntry,
    normalized_query: &str,
    tokens: &[&str],
) -> u32 {
    let command = entry.command.to_lowercase();
    let aliases = entry
        .aliases
        .iter()
        .map(|alias| alias.to_lowercase())
        .collect::<Vec<_>>();
    let tags = entry
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect::<Vec<_>>();
    let description = entry.description.to_lowercase();
    let examples = entry
        .examples
        .iter()
        .map(|example| example.to_lowercase())
        .collect::<Vec<_>>();
    let search_text = std::iter::once(command.as_str())
        .chain(aliases.iter().map(String::as_str))
        .chain(tags.iter().map(String::as_str))
        .chain(std::iter::once(description.as_str()))
        .chain(examples.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");

    let mut score = 0;
    if !normalized_query.is_empty() {
        if command == normalized_query {
            score += 500;
        }
        if command.contains(normalized_query) {
            score += 220;
        }
        if aliases.iter().any(|alias| alias == normalized_query) {
            score += 200;
        }
        if aliases.iter().any(|alias| alias.contains(normalized_query)) {
            score += 160;
        }
        if tags.iter().any(|tag| tag == normalized_query) {
            score += 140;
        }
        if description.contains(normalized_query) {
            score += 120;
        }
        if examples
            .iter()
            .any(|example| example.contains(normalized_query))
        {
            score += 100;
        }
    }

    let mut matched_tokens = 0_u32;
    for token in tokens {
        let mut token_score = 0;
        if command
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| word == *token)
        {
            token_score += 55;
        } else if command.contains(token) {
            token_score += 35;
        }
        if aliases.iter().any(|alias| {
            alias
                .split(|character: char| !character.is_ascii_alphanumeric())
                .any(|word| word == *token)
        }) {
            token_score += 45;
        } else if aliases.iter().any(|alias| alias.contains(token)) {
            token_score += 25;
        }
        if tags.iter().any(|tag| tag == token) {
            token_score += 35;
        } else if tags.iter().any(|tag| tag.contains(token)) {
            token_score += 20;
        }
        if description.contains(token) {
            token_score += 18;
        }
        if examples.iter().any(|example| example.contains(token)) {
            token_score += 12;
        }
        if token_score == 0 && search_text.contains(token) {
            token_score += 8;
        }
        if token_score > 0 {
            matched_tokens += 1;
            score += token_score;
        }
    }

    if !tokens.is_empty() && matched_tokens == tokens.len() as u32 {
        score += 90;
    }

    score
}

fn render_command_manual(entry: &ZoteroCommandCatalogEntry) -> Result<String> {
    let mut command = ZoteroCli::command()
        .name("zotero")
        .bin_name("ata zotero")
        .disable_help_subcommand(true);
    let mut full_command = String::from("ata zotero");
    for segment in entry.command.split(' ') {
        full_command.push(' ');
        full_command.push_str(segment);
        command = command
            .find_subcommand(segment)
            .cloned()
            .with_context(|| format!("find Zotero subcommand `{segment}` for `{full_command}`"))?;
    }
    command = command.bin_name(full_command);
    let arg_ids = command
        .get_arguments()
        .map(|arg| arg.get_id().to_string())
        .collect::<Vec<_>>();
    for arg_id in arg_ids {
        let keep = arg_id == "help" || entry.core_args.iter().any(|candidate| *candidate == arg_id);
        if !keep {
            command = command.mut_arg(&arg_id, |arg| arg.hide(true));
        }
    }
    let mut buffer = Vec::new();
    command.write_long_help(&mut buffer)?;
    let manual = String::from_utf8(buffer).context("convert rendered Zotero help to UTF-8")?;
    let normalize = |line: &str| {
        line.trim()
            .trim_end_matches(|character: char| ['.', ':'].contains(&character))
            .to_ascii_lowercase()
    };
    let mut lines = manual.lines().peekable();
    while lines.peek().is_some_and(|line| line.trim().is_empty()) {
        lines.next();
    }
    if let Some(line) = lines.peek().copied()
        && normalize(line) == normalize(entry.description)
    {
        lines.next();
        while lines.peek().is_some_and(|line| line.trim().is_empty()) {
            lines.next();
        }
    }
    let manual_body = lines.collect::<Vec<_>>().join("\n");
    Ok(format!(
        "Command: {}\n{}\n\n{}",
        entry.command, entry.description, manual_body
    ))
}

fn render_search_results(matches: &[&ZoteroCommandCatalogEntry], manual: &str) -> String {
    let shortlist = matches
        .iter()
        .enumerate()
        .map(|(index, entry)| format!("{}. {} — {}", index + 1, entry.command, entry.description))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Matches:\n{shortlist}\n\nBest match manual:\n\n{manual}")
}

fn zotero_command_catalog() -> &'static [ZoteroCommandCatalogEntry] {
    const CATALOG: &[ZoteroCommandCatalogEntry] = &[
        ZoteroCommandCatalogEntry {
            command: "status",
            description: "Show the effective Zotero mode, scope, and fallback path for this shell.",
            core_args: &[],
            aliases: &["mode", "diagnostics", "debug-config"],
            tags: &["status", "mode", "config", "fallback", "local", "remote"],
            examples: &["show zotero mode and config"],
        },
        ZoteroCommandCatalogEntry {
            command: "collections",
            description: "List collections in a Zotero library so you can identify the right collection key or source-repo bucket.",
            core_args: &[],
            aliases: &[
                "list-collections",
                "show-collections",
                "browse-collections",
                "find-collection",
            ],
            tags: &[
                "collections",
                "collection",
                "folders",
                "bucket",
                "list",
                "browse",
                "lookup",
                "source",
                "repos",
            ],
            examples: &[
                "find the source repos collection",
                "list collections to choose the right folder",
            ],
        },
        ZoteroCommandCatalogEntry {
            command: "resolve-paper",
            description: "Resolve one paper from Zotero and enrich it with document metadata.",
            core_args: &["query", "item_key"],
            aliases: &["resolve-item", "paper-resolution", "find-paper"],
            tags: &["paper", "resolve", "pdf", "attachments", "document"],
            examples: &[
                "resolve a paper from zotero",
                "find the pdf for a zotero paper",
            ],
        },
        ZoteroCommandCatalogEntry {
            command: "add-paper",
            description: "Add a paper to a Zotero collection, attach its PDF when possible, and link a source repo when available.",
            core_args: &["query", "doi", "arxiv", "url", "collection"],
            aliases: &["ingest-paper", "import-paper", "paper-add"],
            tags: &["paper", "add", "import", "pdf", "repo", "collection"],
            examples: &[
                "add a paper to the r&d agents collection",
                "import a paper and its pdf into zotero",
            ],
        },
        ZoteroCommandCatalogEntry {
            command: "find-repos",
            description: "Find repository URLs in Zotero items, collections, or linked records.",
            core_args: &["query", "collection"],
            aliases: &["find-source-repos", "github-discovery", "repo-discovery"],
            tags: &["repos", "github", "gitlab", "source", "implementations"],
            examples: &[
                "find github repo urls in zotero",
                "find source repos for multi agent systems",
            ],
        },
        ZoteroCommandCatalogEntry {
            command: "search",
            description: "Keyword search across Zotero items, including titles, creators, and tags.",
            core_args: &["query"],
            aliases: &["find-items", "lookup-items"],
            tags: &["search", "items", "papers", "metadata"],
            examples: &[
                "find papers about diffusion models",
                "search my Zotero library by title or author",
            ],
        },
        ZoteroCommandCatalogEntry {
            command: "tags",
            description: "List tags for autocomplete and filtering flows.",
            core_args: &[],
            aliases: &["list-tags"],
            tags: &["tags", "filters", "autocomplete"],
            examples: &["show my Zotero tags"],
        },
        ZoteroCommandCatalogEntry {
            command: "recent",
            description: "List recently added or modified items.",
            core_args: &[],
            aliases: &["recent-items", "latest"],
            tags: &["recent", "latest", "items"],
            examples: &["show recently added papers"],
        },
        ZoteroCommandCatalogEntry {
            command: "advanced-search",
            description: "Run multi-condition metadata and fulltext search from a JSON payload.",
            core_args: &["json", "json_file"],
            aliases: &["metadata-search", "compound-search"],
            tags: &["advanced", "metadata", "fulltext", "json"],
            examples: &["search by year and author with JSON filters"],
        },
        ZoteroCommandCatalogEntry {
            command: "grep-text",
            description: "Run bounded literal or regex matching across Zotero text surfaces.",
            core_args: &["json", "json_file"],
            aliases: &["regex-search", "literal-search"],
            tags: &["grep", "regex", "text", "annotations", "notes"],
            examples: &["grep note text for GRPO"],
        },
        ZoteroCommandCatalogEntry {
            command: "search-notes",
            description: "Search notes and annotation text.",
            core_args: &["query"],
            aliases: &["find-notes", "notes-search"],
            tags: &["notes", "annotations", "search"],
            examples: &["search annotations for reward hacking"],
        },
        ZoteroCommandCatalogEntry {
            command: "item get",
            description: "Get item metadata with optional attachment and fulltext resolution.",
            core_args: &["item_key"],
            aliases: &["get-item", "inspect-item"],
            tags: &["item", "metadata", "attachments", "fulltext", "pdf"],
            examples: &["inspect one paper and resolve its pdf"],
        },
        ZoteroCommandCatalogEntry {
            command: "item citation",
            description: "Generate a citation for an item.",
            core_args: &["item_key"],
            aliases: &["cite-item", "cite-paper", "bibtex", "apa-citation"],
            tags: &["citation", "bibtex", "apa", "csl"],
            examples: &["cite paper", "get a bibtex citation for this paper"],
        },
        ZoteroCommandCatalogEntry {
            command: "item fulltext",
            description: "Get indexed fulltext and document resolution for an item.",
            core_args: &["item_key"],
            aliases: &["get-fulltext", "paper-text"],
            tags: &["fulltext", "document", "pdf", "text"],
            examples: &["read the indexed fulltext for this item"],
        },
        ZoteroCommandCatalogEntry {
            command: "item notes",
            description: "Get notes attached to an item.",
            core_args: &["item_key"],
            aliases: &["get-notes", "item-notes"],
            tags: &["notes", "item", "annotations"],
            examples: &["show notes for this paper"],
        },
        ZoteroCommandCatalogEntry {
            command: "item annotations",
            description: "Get annotations for an item or library scope.",
            core_args: &["item_key"],
            aliases: &["get-annotations", "highlights"],
            tags: &["annotations", "highlights", "pdf"],
            examples: &["show pdf annotations for an item"],
        },
        ZoteroCommandCatalogEntry {
            command: "item attachments",
            description: "List attachment metadata for an item.",
            core_args: &["item_key"],
            aliases: &["get-attachments", "attachments"],
            tags: &["attachments", "pdf", "supplement"],
            examples: &["list attachments under this paper"],
        },
        ZoteroCommandCatalogEntry {
            command: "collection items",
            description: "List items in a specific collection.",
            core_args: &["collection_key"],
            aliases: &["collection-contents", "list-collection-items"],
            tags: &["collection", "items", "contents"],
            examples: &["list papers in the multi-agent collection"],
        },
        ZoteroCommandCatalogEntry {
            command: "collection create",
            description: "Create a collection.",
            core_args: &["name"],
            aliases: &["create-collection", "new-collection"],
            tags: &["collection", "create", "folder", "organization"],
            examples: &["create a collection for multi-agent systems"],
        },
        ZoteroCommandCatalogEntry {
            command: "collection find-or-create",
            description: "Find a collection by exact name or create it if missing.",
            core_args: &["name"],
            aliases: &["ensure-collection", "reuse-or-create-collection"],
            tags: &["collection", "find", "create", "idempotent"],
            examples: &["ensure a collection exists before importing papers"],
        },
        ZoteroCommandCatalogEntry {
            command: "collection add-items",
            description: "Add existing items to a collection without disturbing other memberships.",
            core_args: &["collection_key", "item_keys"],
            aliases: &["add-to-collection", "move-items-to-collection"],
            tags: &["collection", "add", "organize", "items"],
            examples: &["add these papers to a collection"],
        },
        ZoteroCommandCatalogEntry {
            command: "groups list",
            description: "List accessible Zotero groups.",
            core_args: &[],
            aliases: &["list-groups", "groups"],
            tags: &["groups", "libraries", "shared"],
            examples: &["show group libraries"],
        },
        ZoteroCommandCatalogEntry {
            command: "items create",
            description: "Create items from a JSON payload.",
            core_args: &["json", "json_file"],
            aliases: &["create-items", "bulk-create-items"],
            tags: &["items", "create", "bulk", "json", "import"],
            examples: &["bulk create paper records from JSON"],
        },
        ZoteroCommandCatalogEntry {
            command: "items update",
            description: "Update existing items from a JSON payload.",
            core_args: &["json", "json_file"],
            aliases: &["update-items", "bulk-update-items"],
            tags: &["items", "update", "bulk", "json", "edit"],
            examples: &["bulk update zotero metadata from JSON"],
        },
        ZoteroCommandCatalogEntry {
            command: "attachment create-link",
            description: "Create a linked attachment, such as a PDF or repository URL, under an existing item.",
            core_args: &["parent_item_key", "title", "url"],
            aliases: &["attach-link", "attach-pdf-url", "link-pdf", "add-pdf-link"],
            tags: &["attachment", "pdf", "url", "repo", "link"],
            examples: &[
                "attach pdf url to paper",
                "attach a pdf url to an existing paper",
                "add a github repo link under this item",
            ],
        },
        ZoteroCommandCatalogEntry {
            command: "attachment import-url",
            description: "Import a URL as a stored attachment under an existing item so Zotero can open the file locally.",
            core_args: &["parent_item_key", "title", "url"],
            aliases: &["import-pdf", "store-pdf", "upload-attachment-url"],
            tags: &["attachment", "pdf", "import", "stored", "viewer"],
            examples: &[
                "import a pdf url under this paper",
                "store a paper pdf in zotero instead of linking it",
            ],
        },
    ];

    CATALOG
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::FindReposArgs;
    use super::ItemCommand;
    use super::JsonOutputArgs;
    use super::LibraryScopeArgs;
    use super::SearchArgs;
    use super::ZoteroCli;
    use super::ZoteroCommand;
    use super::ZoteroQuickSearchMode;
    use super::paper_preferred_pdf_url;
    use super::parse_optional_enum;
    use super::render_command_manual;
    use super::render_search_results;
    use super::search_command_catalog;
    use clap::Parser;
    use codex_research_tools::config::RateLimitOverrides;
    use codex_research_tools::config::ResearchConfig;
    use codex_research_tools::rate_limiter::ApiRateLimit;
    use codex_research_tools::types::Paper;
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;
    use wiremock::matchers::path_regex;
    use wiremock::matchers::query_param;

    fn build_test_runtime(
        base_url: String,
        api_key: Option<&str>,
        mode: super::ZoteroMode,
    ) -> super::ZoteroRuntime {
        build_test_runtime_with_research_sources(
            base_url,
            "http://localhost:23119/api".to_string(),
            "http://localhost:23119".to_string(),
            "http://localhost:23119".to_string(),
            api_key,
            mode,
        )
    }

    fn build_test_runtime_with_research_sources(
        base_url: String,
        semantic_scholar_base_url: String,
        arxiv_base_url: String,
        openalex_base_url: String,
        api_key: Option<&str>,
        mode: super::ZoteroMode,
    ) -> super::ZoteroRuntime {
        let config = ResearchConfig {
            zotero_api_key: api_key.map(ToString::to_string),
            zotero_user_id: Some("123".to_string()),
            zotero_group_id: None,
            zotero_base_url: base_url,
            semantic_scholar_base_url,
            arxiv_base_url,
            openalex_base_url,
            rate_limit_overrides: permissive_rate_limit_overrides(),
            ..ResearchConfig::default()
        };
        super::ZoteroRuntime {
            toolkit: super::ResearchToolkit::from_config(config.clone()),
            config,
            mode,
        }
    }

    fn permissive_rate_limit_overrides() -> RateLimitOverrides {
        RateLimitOverrides {
            semantic_scholar: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
            arxiv: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
            openalex: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
            zotero: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
            github: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
            hackernews: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
            patents: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
        }
    }

    fn add_paper_args(
        query: Option<&str>,
        doi: Option<&str>,
        arxiv: Option<&str>,
        url: Option<&str>,
    ) -> super::AddPaperArgs {
        super::AddPaperArgs {
            query: query.map(ToString::to_string),
            doi: doi.map(ToString::to_string),
            arxiv: arxiv.map(ToString::to_string),
            url: url.map(ToString::to_string),
            collection: "R&D Agents".to_string(),
            repo_collection: "Source Repos".to_string(),
            scope: LibraryScopeArgs {
                library_type: None,
                library_id: None,
            },
            output: JsonOutputArgs { json: false },
        }
    }

    fn test_paper_with_pdf(pdf_url: Option<&str>, arxiv_id: Option<&str>) -> Paper {
        Paper {
            title: "Test Paper".to_string(),
            authors: "Test Author".to_string(),
            year: Some(2026),
            venue: None,
            citation_count: None,
            abstract_text: None,
            doi: None,
            arxiv_id: arxiv_id.map(ToString::to_string),
            s2_paper_id: None,
            openalex_id: None,
            url: None,
            pdf_url: pdf_url.map(ToString::to_string),
            code_url: None,
            source_meta: None,
        }
    }

    #[test]
    fn paper_preferred_pdf_url_ignores_blank_pdf_url() {
        let paper = test_paper_with_pdf(Some("   "), Some("2603.05708v1"));
        assert_eq!(
            paper_preferred_pdf_url(&paper).as_deref(),
            Some("https://arxiv.org/pdf/2603.05708v1.pdf")
        );
    }

    #[test]
    fn paper_preferred_pdf_url_rejects_doi_landing_url() {
        let paper = test_paper_with_pdf(Some("https://doi.org/10.3390/drones9010033"), None);
        assert_eq!(paper_preferred_pdf_url(&paper), None);
    }

    #[test]
    fn paper_preferred_pdf_url_accepts_pdf_like_url() {
        let paper = test_paper_with_pdf(Some("https://example.org/content/paper.pdf"), None);
        assert_eq!(
            paper_preferred_pdf_url(&paper).as_deref(),
            Some("https://example.org/content/paper.pdf")
        );
    }

    #[test]
    fn search_commands_prefers_collection_creation_for_create_collection_queries() {
        let result = search_command_catalog("create collection for agents", 3);
        assert_eq!(
            result.first().map(|entry| entry.command),
            Some("collection create")
        );
    }

    #[test]
    fn search_commands_prefers_attachment_links_for_pdf_queries() {
        let result = search_command_catalog("attach pdf url to paper", 3);
        assert_eq!(
            result.first().map(|entry| entry.command),
            Some("attachment create-link")
        );
    }

    #[test]
    fn search_commands_prefers_find_repos_for_repo_discovery_queries() {
        let result = search_command_catalog("find github repo urls in zotero", 3);
        assert_eq!(
            result.first().map(|entry| entry.command),
            Some("find-repos")
        );
    }

    #[test]
    fn search_commands_prefers_add_paper_for_ingestion_queries() {
        let result = search_command_catalog("add a paper to zotero with its pdf", 3);
        assert_eq!(result.first().map(|entry| entry.command), Some("add-paper"));
    }

    #[test]
    fn search_commands_prefers_collections_for_collection_lookup_queries() {
        let result = search_command_catalog("find the source repos collection", 3);
        assert_eq!(
            result.first().map(|entry| entry.command),
            Some("collections")
        );
    }

    #[test]
    fn boolean_presence_flags_default_to_true() {
        let cli = ZoteroCli::try_parse_from([
            "ata",
            "item",
            "get",
            "--item-key",
            "ABCD1234",
            "--include-attachments",
            "--include-fulltext-resolution",
        ])
        .expect("expected zotero cli to parse");

        let ZoteroCommand::Item(item_cli) = cli.command else {
            panic!("expected item command");
        };
        let ItemCommand::Get(args) = item_cli.command else {
            panic!("expected item get command");
        };

        assert_eq!(args.include_attachments, Some(true));
        assert_eq!(args.include_fulltext_resolution, Some(true));
    }

    #[test]
    fn search_command_parses_qmode() {
        let cli = ZoteroCli::try_parse_from([
            "ata",
            "search",
            "--query",
            "agent",
            "--qmode",
            "everything",
        ])
        .expect("expected zotero cli to parse");

        let ZoteroCommand::Search(SearchArgs { qmode, .. }) = cli.command else {
            panic!("expected search command");
        };

        assert_eq!(
            parse_optional_enum::<ZoteroQuickSearchMode>(qmode, "qmode").expect("parse qmode"),
            Some(ZoteroQuickSearchMode::Everything)
        );
    }

    #[test]
    fn render_command_manual_hides_shared_optional_flags() {
        let entry = search_command_catalog("cite paper", 1)
            .into_iter()
            .next()
            .expect("expected citation match");
        let manual = render_command_manual(entry).expect("expected manual to render");
        assert!(manual.contains("Command: item citation"));
        assert!(manual.contains("Generate a citation for an item"));
        assert_eq!(manual.matches("Generate a citation for an item").count(), 1);
        assert!(manual.contains("Usage: ata zotero item citation --item-key <ITEM_KEY>"));
        assert!(manual.contains("--item-key <ITEM_KEY>"));
        assert!(!manual.contains("--config"));
        assert!(!manual.contains("--library-type"));
        assert!(!manual.contains("--format"));
    }

    #[test]
    fn render_search_results_shows_shortlist_then_best_manual() {
        let matches = search_command_catalog("cite paper", 3);
        let manual = render_command_manual(matches[0]).expect("expected manual");
        let rendered = render_search_results(&matches, &manual);
        assert!(rendered.contains("Matches:"));
        assert!(rendered.contains("1. item citation — Generate a citation for an item."));
        assert!(rendered.contains("Best match manual:"));
        assert_eq!(rendered.matches("Usage: ata zotero").count(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn find_repos_falls_back_when_primary_mode_errors() {
        let primary_server = MockServer::start().await;
        let alternate_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/collections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&primary_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "agent"))
            .and(query_param("qmode", "everything"))
            .and(query_param("itemType", "webpage"))
            .respond_with(ResponseTemplate::new(400).set_body_string("primary search failed"))
            .mount(&primary_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/collections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&alternate_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "agent"))
            .and(query_param("qmode", "everything"))
            .and(query_param("itemType", "webpage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "webpage",
                        "title": "AgentNet",
                        "url": "https://github.com/acme/agentnet"
                    }
                }
            ])))
            .mount(&alternate_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "agent"))
            .and(query_param("qmode", "everything"))
            .and(query_param("itemType", "attachment"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&alternate_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "agent"))
            .and(query_param("qmode", "everything"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&alternate_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "ITEM1",
                "data": {
                    "itemType": "webpage",
                    "title": "AgentNet",
                    "url": "https://github.com/acme/agentnet"
                }
            })))
            .mount(&alternate_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "attachment"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&alternate_server)
            .await;

        let context = super::ZoteroCliContext {
            primary: build_test_runtime(
                primary_server.uri(),
                Some("test-key"),
                super::ZoteroMode::Remote,
            ),
            alternate: Some(build_test_runtime(
                alternate_server.uri(),
                None,
                super::ZoteroMode::Local,
            )),
        };
        let args = FindReposArgs {
            query: Some("agent".to_string()),
            collection: None,
            scope: LibraryScopeArgs {
                library_type: None,
                library_id: None,
            },
            limit: 10,
            inspect_limit: 20,
            output: JsonOutputArgs { json: false },
        };

        let result = super::find_repos_with_fallback(&context, &args)
            .await
            .expect("expected alternate zotero runtime to satisfy repo discovery");

        assert_eq!(result.effective_mode, super::ZoteroMode::Local);
        assert!(result.fallback_used);
        assert_eq!(result.repos.len(), 1);
        assert_eq!(result.repos[0].repo_url, "https://github.com/acme/agentnet");
        assert!(result.warnings[0].contains("remote mode failed with"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_target_collection_uses_explicit_scope_when_collection_metadata_omits_it() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/groups/456/collections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "COLL1",
                    "data": {
                        "name": "R&D Agents"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let config = ResearchConfig {
            zotero_api_key: Some("test-key".to_string()),
            zotero_user_id: Some("123".to_string()),
            zotero_group_id: Some("456".to_string()),
            zotero_base_url: server.uri(),
            ..ResearchConfig::default()
        };
        let runtime = super::ZoteroRuntime {
            toolkit: super::ResearchToolkit::from_config(config.clone()),
            config,
            mode: super::ZoteroMode::Remote,
        };

        let (collection, scope) = super::resolve_target_collection(
            &runtime,
            "R&D Agents",
            &LibraryScopeArgs {
                library_type: Some("group".to_string()),
                library_id: Some("456".to_string()),
            },
        )
        .await
        .expect("expected explicit scope to satisfy collection resolution");

        assert_eq!(collection.key, "COLL1");
        assert_eq!(scope.library_type, "group");
        assert_eq!(scope.library_id, "456");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_external_paper_uses_openalex_doi_match_without_semantic_fallback() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex(r"/graph/v1/paper/.*"))
            .respond_with(ResponseTemplate::new(500).set_body_string("unexpected fallback"))
            .expect(0)
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": {"count": 1},
                "results": [{
                    "id": "https://openalex.org/W123",
                    "display_name": "Exact DOI Match",
                    "publication_year": 2024,
                    "doi": "https://doi.org/10.1000/exact",
                    "ids": {
                        "openalex": "https://openalex.org/W123",
                        "doi": "https://doi.org/10.1000/exact"
                    },
                    "authorships": [{"author": {"display_name": "Alice"}}],
                    "primary_location": {
                        "landing_page_url": "https://example.org/exact",
                        "pdf_url": "https://example.org/exact.pdf",
                        "source": {"display_name": "ICLR"}
                    },
                    "best_oa_location": null
                }]
            })))
            .mount(&openalex_server)
            .await;

        let runtime = build_test_runtime_with_research_sources(
            "http://localhost:23119/api".to_string(),
            format!("{}/graph/v1", semantic_server.uri()),
            arxiv_server.uri(),
            openalex_server.uri(),
            Some("test-key"),
            super::ZoteroMode::Remote,
        );

        let resolved = super::resolve_external_paper(
            &runtime,
            &add_paper_args(None, Some("10.1000/exact"), None, None),
        )
        .await
        .expect("expected DOI resolution from OpenAlex search");

        assert_eq!(resolved.paper.title, "Exact DOI Match");
        assert_eq!(resolved.paper.doi.as_deref(), Some("10.1000/exact"));
        assert!(
            resolved.warnings.is_empty(),
            "warnings: {:?}",
            resolved.warnings
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_external_paper_doi_falls_back_to_semantic_metadata_without_references() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": {"count": 0},
                "results": []
            })))
            .mount(&openalex_server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(r"/graph/v1/paper/[^/]+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "paperId": "s2-doi",
                "title": "Fallback DOI Paper",
                "abstract": "fallback abstract",
                "year": 2021,
                "venue": "ICML",
                "url": "https://example.org/fallback",
                "externalIds": { "DOI": "10.1000/fallback" },
                "authors": [{"name": "Bob"}]
            })))
            .expect(1)
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(r"/graph/v1/paper/.*/references$"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("references should not be fetched"),
            )
            .expect(0)
            .mount(&semantic_server)
            .await;

        let runtime = build_test_runtime_with_research_sources(
            "http://localhost:23119/api".to_string(),
            format!("{}/graph/v1", semantic_server.uri()),
            arxiv_server.uri(),
            openalex_server.uri(),
            Some("test-key"),
            super::ZoteroMode::Remote,
        );

        let resolved = super::resolve_external_paper(
            &runtime,
            &add_paper_args(None, Some("10.1000/fallback"), None, None),
        )
        .await
        .expect("expected DOI resolution from semantic metadata fallback");

        assert_eq!(resolved.paper.title, "Fallback DOI Paper");
        assert_eq!(resolved.paper.doi.as_deref(), Some("10.1000/fallback"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_external_paper_query_uses_search_results_and_preserves_semantic_warning() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/search"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/query"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <feed xmlns="http://www.w3.org/2005/Atom"></feed>"#,
                "application/atom+xml",
            ))
            .mount(&arxiv_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": {"count": 1},
                "results": [{
                    "id": "https://openalex.org/W987",
                    "display_name": "Rate Limited Query Paper",
                    "publication_year": 2025,
                    "doi": "https://doi.org/10.1000/query",
                    "ids": {
                        "openalex": "https://openalex.org/W987",
                        "doi": "https://doi.org/10.1000/query"
                    },
                    "authorships": [{"author": {"display_name": "Carol"}}],
                    "primary_location": {
                        "landing_page_url": "https://example.org/query",
                        "pdf_url": "https://example.org/query.pdf",
                        "source": {"display_name": "NeurIPS"}
                    },
                    "best_oa_location": null
                }]
            })))
            .mount(&openalex_server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(r"/graph/v1/paper/.*"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("detail should not be fetched"),
            )
            .expect(0)
            .mount(&semantic_server)
            .await;

        let runtime = build_test_runtime_with_research_sources(
            "http://localhost:23119/api".to_string(),
            format!("{}/graph/v1", semantic_server.uri()),
            arxiv_server.uri(),
            openalex_server.uri(),
            Some("test-key"),
            super::ZoteroMode::Remote,
        );

        let resolved = super::resolve_external_paper(
            &runtime,
            &add_paper_args(Some("Rate Limited Query Paper"), None, None, None),
        )
        .await
        .expect("expected query resolution from search results");

        assert_eq!(resolved.paper.title, "Rate Limited Query Paper");
        assert!(
            resolved
                .warnings
                .iter()
                .any(|warning| warning.contains("semantic_scholar search failed")),
            "warnings: {:?}",
            resolved.warnings
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_external_paper_prefers_exact_arxiv_match_before_fallback() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/query"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <feed xmlns="http://www.w3.org/2005/Atom">
                  <entry>
                    <id>http://arxiv.org/abs/2401.00001v2</id>
                    <title>Exact arXiv Match</title>
                    <summary>summary</summary>
                    <published>2024-01-01T00:00:00Z</published>
                    <author><name>Dana</name></author>
                    <link title="pdf" href="http://arxiv.org/pdf/2401.00001v2"/>
                  </entry>
                </feed>"#,
                "application/atom+xml",
            ))
            .mount(&arxiv_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/works"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("openalex should not be queried"),
            )
            .expect(0)
            .mount(&openalex_server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(r"/graph/v1/paper/.*"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("semantic fallback should not be used"),
            )
            .expect(0)
            .mount(&semantic_server)
            .await;

        let runtime = build_test_runtime_with_research_sources(
            "http://localhost:23119/api".to_string(),
            format!("{}/graph/v1", semantic_server.uri()),
            arxiv_server.uri(),
            openalex_server.uri(),
            Some("test-key"),
            super::ZoteroMode::Remote,
        );

        let resolved = super::resolve_external_paper(
            &runtime,
            &add_paper_args(None, None, Some("2401.00001v2"), None),
        )
        .await
        .expect("expected exact arXiv resolution");

        assert_eq!(resolved.paper.title, "Exact arXiv Match");
        assert_eq!(resolved.paper.arxiv_id.as_deref(), Some("2401.00001v2"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_external_paper_url_requires_exact_match() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total": 1,
                "next": null,
                "data": [{
                    "paperId": "s2-url",
                    "title": "Different URL Paper",
                    "year": 2024,
                    "url": "https://example.org/different",
                    "authors": [{"name": "Erin"}]
                }]
            })))
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/query"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <feed xmlns="http://www.w3.org/2005/Atom"></feed>"#,
                "application/atom+xml",
            ))
            .mount(&arxiv_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": {"count": 0},
                "results": []
            })))
            .mount(&openalex_server)
            .await;

        let runtime = build_test_runtime_with_research_sources(
            "http://localhost:23119/api".to_string(),
            format!("{}/graph/v1", semantic_server.uri()),
            arxiv_server.uri(),
            openalex_server.uri(),
            Some("test-key"),
            super::ZoteroMode::Remote,
        );

        let err = super::resolve_external_paper(
            &runtime,
            &add_paper_args(None, None, None, Some("https://example.org/requested")),
        )
        .await
        .expect_err("expected unmatched URL to fail");

        assert!(
            err.to_string()
                .contains("matched URL `https://example.org/requested` exactly")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_external_paper_url_accepts_exact_pdf_url_match() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/search"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/query"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <feed xmlns="http://www.w3.org/2005/Atom"></feed>"#,
                "application/atom+xml",
            ))
            .mount(&arxiv_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": {"count": 1},
                "results": [{
                    "id": "https://openalex.org/W777",
                    "display_name": "Exact PDF URL Match",
                    "publication_year": 2024,
                    "ids": {
                        "openalex": "https://openalex.org/W777"
                    },
                    "authorships": [{"author": {"display_name": "Frank"}}],
                    "primary_location": {
                        "landing_page_url": "https://example.org/paper",
                        "pdf_url": "https://example.org/paper.pdf",
                        "source": {"display_name": "CVPR"}
                    },
                    "best_oa_location": null
                }]
            })))
            .mount(&openalex_server)
            .await;

        let runtime = build_test_runtime_with_research_sources(
            "http://localhost:23119/api".to_string(),
            format!("{}/graph/v1", semantic_server.uri()),
            arxiv_server.uri(),
            openalex_server.uri(),
            Some("test-key"),
            super::ZoteroMode::Remote,
        );

        let resolved = super::resolve_external_paper(
            &runtime,
            &add_paper_args(None, None, None, Some("https://example.org/paper.pdf")),
        )
        .await
        .expect("expected exact PDF URL match to resolve");

        assert_eq!(resolved.paper.title, "Exact PDF URL Match");
        assert!(
            resolved
                .warnings
                .iter()
                .any(|warning| warning.contains("semantic_scholar search failed")),
            "warnings: {:?}",
            resolved.warnings
        );
    }
}
