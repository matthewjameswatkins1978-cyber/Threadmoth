use clap::{Args, Parser, Subcommand, ValueEnum, ValueHint};

pub const THREADMOTH_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(
    name = "threadmoth",
    version = THREADMOTH_VERSION,
    about = "Fast, deterministic structural search and rewrite for AI agents.",
    long_about = "Threadmoth changes exactly the state a request authorizes, refuses ambiguity, and returns a certificate describing what was observed and what actually changed.",
    after_help = "Edit a file: threadmoth edit PATH --search OLD --replace NEW\nFor several files: threadmoth edit changes.tm\nReview before applying: threadmoth edit PATH --search OLD --replace NEW --plan\nStart a text edit: threadmoth prepare-text PATH --edit OLD NEW --output plan.json --summary\nReview it: threadmoth explain --plan plan.json --format diff\nApply it: threadmoth apply-plan --plan plan.json --summary\nDiscover structured edits: threadmoth suggest PATH\nInspect capabilities: threadmoth capabilities\nGenerate shell completion: threadmoth completions <shell>\nRun checks: threadmoth benchmark --tough",
    propagate_version = true,
    disable_help_subcommand = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Edit one file or apply an edit document in one verified atomic invocation.
    #[command(
        after_help = "Single file: threadmoth edit file.txt --search OLD --replace NEW\nChanges document: threadmoth edit changes.tm\nFrom stdin: cat changes.tm | threadmoth edit -\nPreview only: threadmoth edit file.txt --search OLD --replace NEW --plan\nMachine output: threadmoth edit file.txt --search OLD --replace NEW --json"
    )]
    Edit(EditArgs),

    /// Replace exactly one literal occurrence using the canonical text pipeline.
    ReplaceExact(ReplaceExactArgs),

    /// Set one structured value using the canonical provider pipeline.
    SetValue(SetValueArgs),

    /// Set one structured value as an explicit UTF-8 string.
    SetString(SetStringArgs),

    /// Create one missing file without overwriting an existing destination.
    CreateFile(CreateFileArgs),

    /// Apply one verified mutation.
    #[command(
        alias = "apply",
        after_help = "Example: threadmoth mutate --request request.json\nHuman summary: threadmoth mutate --request request.json --summary"
    )]
    Mutate(RequestArgs),

    /// Preview a mutation without writing.
    #[command(
        alias = "dry-run",
        after_help = "Machine certificate: threadmoth preview --request request.json\nCompact human view: threadmoth preview --request request.json --summary"
    )]
    Preview(RequestArgs),

    /// Apply or preview a guarded multi-file transaction.
    #[command(
        after_help = "Preview: threadmoth transact --request transaction.json --preview\nCompact human view: threadmoth transact --request transaction.json --preview --summary"
    )]
    Transact(TransactionArgs),

    /// Prepare a deterministic mutation plan without writing.
    #[command(after_help = "Example: threadmoth plan --request request.json --output plan.json")]
    Plan(PlanArgs),

    /// Apply a previously prepared exact plan.
    #[command(after_help = "Example: threadmoth apply-plan --plan plan.json --summary")]
    ApplyPlan(ApplyPlanArgs),

    /// Prepare exact text replacements from the observed source without changing it.
    #[command(
        after_help = "Example: threadmoth prepare-text settings.conf --edit OLD NEW --output plan.json --summary\nFor multiline text: threadmoth prepare-text settings.conf --edit-files old.txt new.txt --output plan.json --summary\nThen review: threadmoth explain --plan plan.json --format diff\nThen apply: threadmoth apply-plan --plan plan.json --summary\nUse --request for the complete source-aware JSON request."
    )]
    PrepareText(PrepareTextArgs),

    /// Prepare strict exact edits across several workspace files without writing them.
    #[command(
        after_help = "Review during preparation: threadmoth prepare-edits changes.tm --output plan.json --review\nCompact commands only: threadmoth prepare-edits changes.tm --output plan.json --summary\nFull review if truncated: threadmoth explain --plan plan.json --format diff --full\nApply separately: threadmoth apply-plan --plan plan.json --summary\nUse one *** File: PATH section per workspace-relative file and ordered SEARCH/REPLACE blocks within it."
    )]
    PrepareEdits(PrepareEditsArgs),

    /// Perform structural AST pattern search and rewrite using the Toolbelt ast-grep attachment.
    #[command(
        name = "structural",
        after_help = "Examples:\n  threadmoth structural --pattern 'old($A)' --rewrite 'new($A)' --lang python src/\n  threadmoth structural --pattern 'foo($A)' --rewrite 'bar($A)' src/ --preview\n  threadmoth structural --pattern 'foo($A)' --rewrite 'bar($A)' src/ --plan plan.json\nThen review: threadmoth explain --plan plan.json --format diff\nThen apply: threadmoth apply-plan --plan plan.json --summary"
    )]
    Structural(StructuralArgs),

    /// Legacy alias for `transact --preview`.
    #[command(hide = true)]
    TransactionPreview(RequestArgs),

    /// Recover interrupted transaction state where possible.
    Recover(RecoverArgs),

    /// Show machine-readable capabilities, optionally for one file.
    #[command(
        after_help = "Examples:\n  threadmoth capabilities\n  threadmoth capabilities code\n  threadmoth capabilities --for src/main.rs\n  threadmoth capabilities --json --all"
    )]
    Capabilities(CapabilitiesArgs),

    /// Show request examples.
    #[command(after_help = "Example: threadmoth examples source-text")]
    Examples {
        /// Optional example topic.
        topic: Option<String>,
    },

    /// Run correctness-checked performance and safety checks.
    #[command(
        after_help = "Canonical forms:\n  threadmoth benchmark\n  threadmoth benchmark --quick\n  threadmoth benchmark --tough\n  threadmoth benchmark --torture\nAdd --json for machine output."
    )]
    Benchmark(BenchmarkArgs),

    /// Legacy alias for `benchmark --torture`.
    #[command(hide = true)]
    Torture {
        /// Emit machine-readable JSON.
        #[arg(short = 'j', long)]
        json: bool,
    },

    /// Search or show detailed command help.
    Help(HelpArgs),

    /// Orient an agent to the current workspace and safe discovery workflow.
    Orient {
        /// Emit machine-readable orientation data.
        #[arg(long)]
        json: bool,
    },

    /// Explain a refusal or failure reason code.
    Explain {
        /// Reason code such as TARGET_AMBIGUOUS.
        #[arg(value_name = "CODE", required_unless_present = "plan")]
        code: Option<String>,
        /// Explain a prepared plan without applying it.
        #[arg(long, value_hint = ValueHint::FilePath, conflicts_with = "code")]
        plan: Option<std::path::PathBuf>,
        /// Emit machine-readable JSON.
        #[arg(short = 'j', long)]
        json: bool,
        /// Render a prepared plan as a bounded diff or Markdown review.
        #[arg(long, value_enum)]
        format: Option<ExplainFormat>,
        /// Show the complete diff when a bounded review is truncated (may be large).
        #[arg(long, requires_all = ["plan", "format"], conflicts_with = "json")]
        full: bool,
    },

    /// Suggest a safe request shape for a workspace file or refusal.
    #[command(
        after_help = "Examples:\n  threadmoth suggest Cargo.toml --goal set-value --selector package.name\n  threadmoth suggest --from-refusal refusal.json"
    )]
    Suggest(SuggestArgs),

    /// Inspect identity, a compact structural outline, or one bounded region.
    #[command(
        after_help = "Examples:\n  threadmoth inspect PATH\n  threadmoth inspect PATH --json\n  threadmoth inspect PATH --outline\n  threadmoth inspect PATH --expand HANDLE"
    )]
    Inspect(InspectArgs),

    /// Print protocol or request schemas.
    #[command(
        after_help = "Examples: threadmoth schema request --json  |  threadmoth schema --json"
    )]
    Schema(SchemaArgs),

    /// Check runtime and CLI installation health.
    Doctor(DoctorArgs),

    /// Check for and explicitly install a stable official Threadmoth release.
    #[command(
        disable_version_flag = true,
        after_help = "Examples:\n  threadmoth update --check\n  threadmoth update\n  threadmoth update --yes --json"
    )]
    Update(UpdateArgs),

    /// Generate shell completion for Threadmoth.
    #[command(
        after_help = "Current-session examples:\n  PowerShell: threadmoth completions powershell | Out-String | Invoke-Expression\n  Bash: source <(threadmoth completions bash)"
    )]
    Completions {
        /// Shell to generate completion for.
        #[arg(value_enum)]
        shell: CompletionShell,
    },

    /// Generate a roff man page to stdout or a file.
    Manpage {
        /// Optional output path. Defaults to stdout.
        #[arg(short, long, value_hint = ValueHint::FilePath)]
        output: Option<std::path::PathBuf>,
    },

    /// Run the MCP stdio server.
    Mcp,
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Emit stable machine-readable health data without network access.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct InspectArgs {
    /// Workspace-relative file path.
    #[arg(value_name = "FILE_PATH", value_hint = ValueHint::FilePath)]
    pub path: std::path::PathBuf,

    /// Return a bounded deterministic structural outline.
    #[arg(long, conflicts_with = "expand")]
    pub outline: bool,

    /// Expand one exact handle returned by --outline.
    #[arg(long, value_name = "HANDLE", conflicts_with = "outline")]
    pub expand: Option<String>,

    /// Maximum bytes returned by an expansion.
    #[arg(long, default_value_t = 8192)]
    pub max_bytes: usize,

    /// Maximum outline entries returned.
    #[arg(long, default_value_t = 64)]
    pub max_entries: usize,

    /// Compatibility alias; inspect already emits JSON by default.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ReplaceExactArgs {
    #[arg(value_name = "FILE_PATH", value_hint = ValueHint::FilePath)]
    pub file: std::path::PathBuf,
    pub old: String,
    pub new: String,
}

#[derive(Args, Debug)]
pub struct SetValueArgs {
    #[arg(value_name = "FILE_PATH", value_hint = ValueHint::FilePath)]
    pub file: std::path::PathBuf,
    #[arg(value_name = "SELECTOR")]
    pub path: String,
    /// JSON spelling of the value, for example 8080, true, or "enabled".
    pub value: String,
    /// Treat VALUE as a literal UTF-8 string instead of parsing it as JSON.
    #[arg(long)]
    pub string: bool,
}

#[derive(Args, Debug)]
pub struct SetStringArgs {
    #[arg(value_name = "FILE_PATH", value_hint = ValueHint::FilePath)]
    pub file: std::path::PathBuf,
    #[arg(value_name = "SELECTOR")]
    pub path: String,
    /// Literal UTF-8 string value.
    pub value: String,
}

#[derive(Args, Debug)]
pub struct CreateFileArgs {
    #[arg(value_name = "FILE_PATH", value_hint = ValueHint::FilePath)]
    pub file: std::path::PathBuf,
    pub content: String,
}

#[derive(Args, Debug)]
pub struct RequestArgs {
    /// Read the JSON request from a file instead of stdin.
    #[arg(short = 'r', long, value_hint = ValueHint::FilePath)]
    pub request: Option<std::path::PathBuf>,

    /// Print a compact human summary instead of the full JSON certificate.
    #[arg(long)]
    pub summary: bool,
}

#[derive(Args, Debug)]
pub struct RecoverArgs {
    /// List internally discovered journals without changing files.
    #[arg(long, conflicts_with_all = ["inspect", "transaction"])]
    pub list: bool,

    /// Inspect one internally discovered transaction without changing files.
    #[arg(long, value_name = "TRANSACTION_ID", conflicts_with_all = ["list", "transaction"])]
    pub inspect: Option<String>,

    /// Recover one internally discovered transaction by ID.
    #[arg(long, value_name = "TRANSACTION_ID", conflicts_with_all = ["list", "inspect"])]
    pub transaction: Option<String>,
}

#[derive(Args, Debug)]
pub struct TransactionArgs {
    /// Read the transaction JSON request from a file instead of stdin.
    #[arg(short = 'r', long, value_hint = ValueHint::FilePath)]
    pub request: Option<std::path::PathBuf>,

    /// Preview the transaction without committing it.
    #[arg(short = 'n', long)]
    pub preview: bool,

    /// Print a compact human summary instead of the full JSON certificate.
    #[arg(long)]
    pub summary: bool,
}

#[derive(Args, Debug)]
pub struct PlanArgs {
    /// Read a request or transaction JSON file instead of stdin.
    #[arg(short = 'r', long, value_hint = ValueHint::FilePath)]
    pub request: Option<std::path::PathBuf>,

    /// Write the prepared plan to this file as well as stdout.
    #[arg(short, long, value_hint = ValueHint::FilePath)]
    pub output: Option<std::path::PathBuf>,

    /// Print a compact human summary instead of the full plan.
    #[arg(long)]
    pub summary: bool,
}

#[derive(Args, Debug)]
pub struct ApplyPlanArgs {
    /// Read the prepared plan from this file instead of stdin.
    #[arg(short, long, value_hint = ValueHint::FilePath)]
    pub plan: std::path::PathBuf,

    /// Print a compact human summary instead of the full certificate.
    #[arg(long)]
    pub summary: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum PrepareTextNewlinePolicy {
    #[default]
    Source,
    Exact,
}

#[derive(Args, Debug)]
pub struct PrepareTextArgs {
    /// Workspace-relative source file for compact exact text edits.
    #[arg(value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub file: Option<std::path::PathBuf>,

    /// Exact text to replace and its replacement; repeat for sequential edits.
    #[arg(long = "edit", num_args = 2, action = clap::ArgAction::Append, value_names = ["OLD", "NEW"])]
    pub edits: Vec<String>,

    /// Read exact UTF-8 OLD and NEW text from files; repeat for sequential edits.
    #[arg(long = "edit-files", num_args = 2, action = clap::ArgAction::Append, value_names = ["OLD_FILE", "NEW_FILE"], value_hint = ValueHint::FilePath)]
    pub edit_files: Vec<std::path::PathBuf>,

    /// Derive uniform LF/CRLF from source, or require supplied bytes exactly for mixed endings.
    #[arg(long, value_enum)]
    pub newline_policy: Option<PrepareTextNewlinePolicy>,

    /// Scope disambiguation anchor (structural scope or section boundary).
    #[arg(long, alias = "scope", value_name = "ANCHOR")]
    pub within: Option<String>,

    /// Write the guarded prepared plan to this file for review and apply.
    #[arg(short, long, value_hint = ValueHint::FilePath)]
    pub output: Option<std::path::PathBuf>,

    /// Print concise next steps instead of the full preparation response (requires --output).
    #[arg(long)]
    pub summary: bool,

    /// Read the complete source-aware request from this JSON file, or stdin when omitted.
    #[arg(short = 'r', long, value_hint = ValueHint::FilePath)]
    pub request: Option<std::path::PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub struct StructuralArgs {
    /// AST pattern to search (e.g. 'foo($A, $B)')
    #[arg(short, long)]
    pub pattern: String,

    /// Replacement template using matched metavariables (e.g. 'bar($B, $A)')
    #[arg(short, long)]
    pub rewrite: String,

    /// Language of the pattern and files (e.g. python, rust, typescript). Inferred from file extension if omitted.
    #[arg(short, long)]
    pub lang: Option<String>,

    /// Target files or directories within the workspace
    #[arg(default_value = ".")]
    pub paths: Vec<String>,

    /// Save the prepared plan to this file for review and execution
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub plan: Option<std::path::PathBuf>,

    /// Preview the prospective diff without writing to disk
    #[arg(long)]
    pub preview: bool,

    /// Maximum number of files permitted to be modified
    #[arg(long)]
    pub max_files: Option<usize>,

    /// Output full JSON certificate or summary
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct PrepareEditsArgs {
    /// Workspace-relative UTF-8 changes.tm document.
    #[arg(value_name = "DOCUMENT", value_hint = ValueHint::FilePath)]
    pub document: std::path::PathBuf,

    /// Write the guarded multi-file plan to this new file.
    #[arg(short, long, value_hint = ValueHint::FilePath)]
    pub output: Option<std::path::PathBuf>,

    /// Print compact review/apply commands instead of the complete plan (requires --output).
    #[arg(long, conflicts_with = "review")]
    pub summary: bool,

    /// Show the bounded source diff after preparing the plan; applying remains separate.
    #[arg(long, requires = "output", conflicts_with = "summary")]
    pub review: bool,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    /// Workspace-relative target file or changes document (or '-' for stdin).
    #[arg(value_name = "TARGET", value_hint = ValueHint::FilePath)]
    pub target: std::path::PathBuf,

    /// Exact text to search for (used with --replace for single-file editing).
    #[arg(
        short = 's',
        long,
        alias = "old",
        value_name = "OLD",
        requires = "replace"
    )]
    pub search: Option<String>,

    /// Replacement text (used with --search for single-file editing).
    #[arg(
        short = 'r',
        long,
        alias = "new",
        value_name = "NEW",
        requires = "search"
    )]
    pub replace: Option<String>,

    /// Scope disambiguation anchor (structural scope or section boundary).
    #[arg(long, alias = "scope", value_name = "ANCHOR")]
    pub within: Option<String>,

    /// Prepare and review the plan without mutating sources.
    #[arg(long)]
    pub plan: bool,

    /// Emit machine-readable JSON output instead of human-friendly presentation.
    #[arg(short = 'j', long)]
    pub json: bool,

    /// Write prepared plan to this file (used with --plan).
    #[arg(short = 'o', long, value_name = "PLAN_PATH", requires = "plan", value_hint = ValueHint::FilePath)]
    pub output: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Check for an update without replacing the current executable.
    #[arg(long)]
    pub check: bool,

    /// Do not ask for confirmation before replacing the current executable.
    #[arg(long)]
    pub yes: bool,

    /// Emit one machine-readable status object.
    #[arg(long)]
    pub json: bool,

    /// Check for this stable release, refusing downgrades.
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,
}

#[derive(Args, Debug)]
pub struct CapabilitiesArgs {
    /// Optional capability/provider selector.
    pub selector: Option<String>,

    /// Evaluate capabilities for this workspace-relative file.
    #[arg(long = "for", value_hint = ValueHint::FilePath)]
    pub for_path: Option<std::path::PathBuf>,

    /// Emit compact machine-readable JSON.
    #[arg(short = 'j', long)]
    pub json: bool,

    /// Force pretty JSON output.
    #[arg(long)]
    pub pretty: bool,

    /// Compatibility flag accepted by existing automation.
    #[arg(long)]
    pub all: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum BenchmarkProfile {
    Quick,
    Standard,
    Tough,
}

#[derive(Args, Debug)]
pub struct BenchmarkArgs {
    /// Legacy positional profile: quick, standard or tough.
    #[arg(value_enum, hide = true)]
    pub profile: Option<BenchmarkProfile>,

    /// Run the quick benchmark profile.
    #[arg(short = 'q', long, conflicts_with_all = ["tough", "torture"])]
    pub quick: bool,

    /// Run the tough benchmark profile.
    #[arg(short = 't', long, conflicts_with_all = ["quick", "torture"])]
    pub tough: bool,

    /// Run the deterministic safety torture suite instead of timing benchmarks.
    #[arg(short = 'x', long, conflicts_with_all = ["quick", "tough", "profile"])]
    pub torture: bool,

    /// Emit machine-readable JSON.
    #[arg(short = 'j', long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct HelpArgs {
    /// Command to show detailed help for.
    pub command: Option<String>,

    /// Search Threadmoth help metadata.
    #[arg(long)]
    pub find: Option<String>,

    /// Emit machine-readable targeted help.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SuggestArgs {
    /// Workspace-relative file to inspect.
    #[arg(value_name = "FILE_PATH", value_hint = ValueHint::FilePath, required_unless_present = "from_refusal")]
    pub path: Option<std::path::PathBuf>,

    /// Read an existing refusal certificate from this file, or `-` for stdin.
    #[arg(long, value_hint = ValueHint::FilePath, conflicts_with = "path")]
    pub from_refusal: Option<String>,

    /// High-level desired operation, such as set-value.
    #[arg(long)]
    pub goal: Option<String>,

    /// Structural location, such as package.name.
    #[arg(long = "selector", alias = "at")]
    pub at: Option<String>,

    /// Suggestion mode. Defaults to safe.
    #[arg(long, default_value = "safe")]
    pub mode: String,
}

#[derive(Args, Debug)]
pub struct SchemaArgs {
    /// Schema scope: all, request, response, certificate, transaction_request, transaction_certificate, or metadata.
    pub scope: Option<String>,

    /// Emit compact machine-readable JSON.
    #[arg(short = 'j', long)]
    pub json: bool,

    /// Force pretty JSON output.
    #[arg(long)]
    pub pretty: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum ExplainFormat {
    Diff,
    Markdown,
}

impl From<CompletionShell> for clap_complete::Shell {
    fn from(value: CompletionShell) -> Self {
        match value {
            CompletionShell::Bash => clap_complete::Shell::Bash,
            CompletionShell::Zsh => clap_complete::Shell::Zsh,
            CompletionShell::Fish => clap_complete::Shell::Fish,
            CompletionShell::Powershell => clap_complete::Shell::PowerShell,
        }
    }
}
