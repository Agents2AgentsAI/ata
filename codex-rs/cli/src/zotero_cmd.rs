use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::CommandFactory;
use clap::Parser;
use clap::Subcommand;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_core::research::build_research_config;
use codex_research_tools::ResearchToolkit;
use codex_research_tools::config::DEFAULT_LOCAL_ZOTERO_BASE_URL;
use codex_research_tools::config::ResearchConfig;
use codex_research_tools::types::ZoteroAddItemsToCollectionParams;
use codex_research_tools::types::ZoteroAnnotationsParams;
use codex_research_tools::types::ZoteroCitationParams;
use codex_research_tools::types::ZoteroCollection;
use codex_research_tools::types::ZoteroCollectionItemsParams;
use codex_research_tools::types::ZoteroCollectionsParams;
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
            let output = toolkit
                .zotero_get_collections(ZoteroCollectionsParams {
                    library_type: args.scope.library_type,
                    library_id: args.scope.library_id,
                    limit: args.limit,
                    offset: args.offset,
                })
                .await?;
            if args.output.compact {
                let matches = output
                    .collections
                    .into_iter()
                    .map(|collection| ZoteroCollectionMatch {
                        key: collection.key,
                        name: collection.name,
                        parent_collection: collection.parent_collection,
                        score: 0,
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
        },
    }
}

async fn load_research_config(config_overrides: &CliConfigOverrides) -> Result<ResearchConfig> {
    let cli_kv_overrides = config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    let config = Config::load_with_cli_overrides_and_harness_overrides(
        cli_kv_overrides,
        ConfigOverrides::default(),
    )
    .await?;
    let cwd = std::env::current_dir().context("resolve current directory")?;
    Ok(build_research_config(
        config.research.as_ref(),
        &config.codex_home,
        &cwd,
    ))
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

async fn find_repos_with_fallback(
    context: &ZoteroCliContext,
    args: &FindReposArgs,
) -> Result<ZoteroFindReposResult> {
    let primary = find_repos_once(&context.primary, args).await?;
    if !primary.repos.is_empty() || context.alternate.is_none() {
        return Ok(primary);
    }

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
        let resolved = resolve_collection_reference(collections.as_slice(), collection_ref);
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
        let collection_matches = score_matching_collections(collections.as_slice(), query, 3);
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
) -> Vec<ZoteroCollectionMatch> {
    if let Some(collection) = collections
        .iter()
        .find(|collection| collection.key == collection_ref)
    {
        return vec![ZoteroCollectionMatch {
            key: collection.key.clone(),
            name: collection.name.clone(),
            parent_collection: collection.parent_collection.clone(),
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
            score: u32::MAX - 1,
        })
        .collect::<Vec<_>>();
    if !exact_name_matches.is_empty() {
        return exact_name_matches;
    }
    score_matching_collections(collections, collection_ref, 5)
}

fn dedup_collection_matches(matches: Vec<ZoteroCollectionMatch>) -> Vec<ZoteroCollectionMatch> {
    let mut seen = BTreeSet::new();
    matches
        .into_iter()
        .filter(|collection| seen.insert(collection.key.clone()))
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
            println!("[{}] {}", collection.key, collection.name);
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
            aliases: &["attach-link", "link-pdf", "add-pdf-link"],
            tags: &["attachment", "pdf", "url", "repo", "link"],
            examples: &[
                "attach a pdf url to an existing paper",
                "add a github repo link under this item",
            ],
        },
    ];

    CATALOG
}

#[cfg(test)]
mod tests {
    use super::ItemCommand;
    use super::SearchArgs;
    use super::ZoteroCli;
    use super::ZoteroCommand;
    use super::ZoteroQuickSearchMode;
    use super::parse_optional_enum;
    use super::render_command_manual;
    use super::render_search_results;
    use super::search_command_catalog;
    use clap::Parser;
    use pretty_assertions::assert_eq;

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
}
