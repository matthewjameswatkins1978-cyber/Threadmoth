#![forbid(unsafe_code)]

mod cli;
mod cli_mcp;

use clap::{CommandFactory, Parser};
use clap_complete::generate;
use std::{
    env, fs,
    io::{self, Read, Write},
    path::Path,
};
use threadmoth::{
    pipeline::execute_request,
    protocol::{
        Assertion, Certificate, CommitGuarantee, EffectBudget, EffectUsage, Outcome,
        PlanApplyResult, PreparedPlan, PreservationFacts, RefusalReason, Request,
        StructuralValidation, TransactionCertificate, TransactionRequest, MAX_PLAN_BYTES,
        MAX_REQUEST_BYTES, PROTOCOL_VERSION,
    },
    workspace::Workspace,
};

use cli::{
    ApplyPlanArgs, BenchmarkArgs, BenchmarkProfile, CapabilitiesArgs, Cli, Command,
    CompletionShell, CreateFileArgs, DoctorArgs, EditArgs, ExplainFormat, HelpArgs, InspectArgs,
    PlanArgs, PrepareEditsArgs, PrepareTextArgs, RecoverArgs, ReplaceExactArgs, SchemaArgs,
    SetStringArgs, SetValueArgs, SuggestArgs, UpdateArgs, THREADMOTH_VERSION,
};

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Edit(args) => run_edit(args),
        Command::ReplaceExact(args) => run_replace_exact(args),
        Command::SetValue(args) => run_set_value(args),
        Command::SetString(args) => run_set_string(args),
        Command::CreateFile(args) => run_create_file(args),
        Command::Mutate(args) => run_request(args.request.as_deref(), false, args.summary),
        Command::Preview(args) => run_request(args.request.as_deref(), true, args.summary),
        Command::Transact(args) => {
            run_transaction(args.request.as_deref(), args.preview, args.summary)
        }
        Command::Plan(args) => run_plan(args),
        Command::ApplyPlan(args) => run_apply_plan(args),
        Command::PrepareText(args) => run_prepare_text(args),
        Command::PrepareEdits(args) => run_prepare_edits(args),
        Command::TransactionPreview(args) => {
            run_transaction(args.request.as_deref(), true, args.summary)
        }
        Command::Recover(args) => run_recover(args),
        Command::Capabilities(args) => run_capabilities(args),
        Command::Examples { topic } => print_examples(topic.as_deref()),
        Command::Benchmark(args) => run_benchmark(args),
        Command::Torture { json } => std::process::exit(threadmoth::torture::run(json)),
        Command::Help(args) => run_help(args),
        Command::Orient { json } => run_orient(json),
        Command::Explain {
            code,
            plan,
            json,
            format,
            full,
        } => {
            if let Some(plan) = plan {
                run_explain_plan(&plan, json, format, full);
            } else if let Some(code) = code {
                print_explain(&code, json);
            }
        }
        Command::Suggest(args) => run_suggest(args),
        Command::Inspect(args) => run_inspect(args),
        Command::Schema(args) => run_schema(args),
        Command::Doctor(args) => run_doctor(args),
        Command::Update(args) => run_update(args),
        Command::Completions { shell } => run_completions(shell),
        Command::Manpage { output } => run_manpage(output.as_deref()),
        Command::Mcp => cli_mcp::run_mcp(),
    }
}

fn shorthand_request(
    file: &Path,
    operation: threadmoth::protocol::OperationPayload,
    bytes: usize,
) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: format!("shorthand-{}", file.display()),
        allow_generated: false,
        file_path: file.to_string_lossy().replace('\\', "/"),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: threadmoth::protocol::Cardinality::ExactlyOne,
        budget: EffectBudget {
            max_files: Some(1),
            max_matches: Some(1),
            max_changed_regions: Some(1),
            max_changed_lines: None,
            max_changed_bytes: Some(bytes.saturating_add(64).max(1)),
            allowed_path_prefixes: Vec::new(),
        },
        operation,
    }
}

fn run_shorthand(request: Request) {
    let workspace = Workspace::new(env::current_dir().unwrap_or_else(|_| ".".into()))
        .unwrap_or_else(|error| {
            eprintln!("workspace initialization failed: {error}");
            std::process::exit(3)
        });
    let certificate = execute_request(&workspace, &request, false);
    emit_certificate(&certificate, false, false);
    exit_for_outcome(certificate.outcome);
}

fn run_replace_exact(args: ReplaceExactArgs) {
    let bytes = args.old.len().max(args.new.len());
    run_shorthand(shorthand_request(
        &args.file,
        threadmoth::protocol::OperationPayload::Text(
            threadmoth::provider::text::TextOperation::Replace {
                target: args.old,
                replacement: args.new,
            },
        ),
        bytes,
    ));
}

fn run_set_value(args: SetValueArgs) {
    let value = if args.string {
        serde_json::Value::String(args.value.clone())
    } else {
        match serde_json::from_str(&args.value) {
            Ok(value) => value,
            Err(error) => {
                let mut certificate = empty_cert(RefusalReason::MalformedInput {
                    details: format!("set-value expects a JSON value: {error}"),
                });
                certificate.schema_diagnostic =
                    Some(threadmoth::protocol::schema_diagnostic(&error.to_string()));
                emit_certificate(&certificate, false, false);
                std::process::exit(2);
            }
        }
    };
    let operation = match threadmoth::shorthand::set_value_operation(
        &args.file.to_string_lossy(),
        &args.path,
        value,
    ) {
        Ok(operation) => operation,
        Err(error) => {
            eprintln!("set-value refused: {error}");
            std::process::exit(2);
        }
    };
    run_shorthand(shorthand_request(&args.file, operation, args.value.len()));
}

fn run_set_string(args: SetStringArgs) {
    let operation = match threadmoth::shorthand::set_value_operation(
        &args.file.to_string_lossy(),
        &args.path,
        serde_json::Value::String(args.value.clone()),
    ) {
        Ok(operation) => operation,
        Err(error) => {
            eprintln!("set-string refused: {error}");
            std::process::exit(2);
        }
    };
    run_shorthand(shorthand_request(&args.file, operation, args.value.len()));
}

fn run_create_file(args: CreateFileArgs) {
    let content = args.content.into_bytes();
    let bytes = content.len().max(1);
    run_shorthand(shorthand_request(
        &args.file,
        threadmoth::protocol::OperationPayload::File(
            threadmoth::lifecycle::FileOperation::CreateFile {
                expected_absent: true,
                content,
            },
        ),
        bytes,
    ));
}

fn run_update(args: UpdateArgs) {
    let installation = threadmoth::updater::installation_kind();
    if !installation.is_standalone() {
        let report = threadmoth::updater::UpdateReport::refused(
            installation,
            threadmoth::updater::UpdateErrorKind::UnsupportedInstall,
            "self-update is disabled for package-managed installations",
        );
        print_update_report(&report, args.json);
        std::process::exit(2);
    }

    let info = match threadmoth::updater::discover(args.version.as_deref()) {
        Ok(info) => info,
        Err(error) => {
            let report = threadmoth::updater::UpdateReport::from_error(error);
            print_update_report(&report, args.json);
            std::process::exit(report.exit_code());
        }
    };
    if info.is_current() {
        let report = info.into_report("up_to_date");
        print_update_report(&report, args.json);
        return;
    }

    let available = info.available_version.clone().unwrap_or_default();
    if args.check {
        let report = info.into_report("update_available");
        print_update_report(&report, args.json);
        return;
    }

    if !args.yes {
        print!("Update {} → {}? [Y/n] ", info.current_version, available);
        let _ = io::stdout().flush();
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer).is_err()
            || matches!(answer.trim().to_ascii_lowercase().as_str(), "n" | "no")
        {
            let report = info.into_report("refused");
            print_update_report(&report, args.json);
            std::process::exit(2);
        }
    }

    match threadmoth::updater::install(&info) {
        Ok(report) => print_update_report(&report, args.json),
        Err(error) => {
            let report = threadmoth::updater::UpdateReport::from_error(error);
            print_update_report(&report, args.json);
            std::process::exit(report.exit_code());
        }
    }
}

fn print_update_report(report: &threadmoth::updater::UpdateReport, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(report).unwrap());
        return;
    }
    match report.status.as_str() {
        "up_to_date" => println!("Threadmoth {} is already current.", report.current_version),
        "update_available" => println!(
            "Threadmoth {} is available.\nPlatform: {}\nVerification: {}",
            report.available_version.as_deref().unwrap_or("unknown"),
            report.platform,
            report.verification
        ),
        "updated" => println!(
            "Updated Threadmoth {} → {}",
            report.current_version,
            report.available_version.as_deref().unwrap_or("unknown")
        ),
        "refused" | "failed" => eprintln!(
            "Update {}: {}",
            report.status,
            report.error.as_deref().unwrap_or("unknown error")
        ),
        _ => println!("Update status: {}", report.status),
    }
}

fn run_plan(args: PlanArgs) {
    let input = match read_request_input(args.request.as_deref()) {
        Ok(input) => input,
        Err(RequestInputError::TooLarge(actual)) => {
            eprintln!("plan input exceeds {MAX_REQUEST_BYTES} bytes (actual: {actual})");
            std::process::exit(2)
        }
        Err(RequestInputError::Io(error)) => {
            eprintln!("plan input read failed: {error}");
            std::process::exit(3)
        }
    };
    let kind = match parse_plan_input(&input) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("plan input refused: {error}");
            std::process::exit(2)
        }
    };
    let root = env::current_dir().unwrap_or_else(|_| ".".into());
    let workspace = match Workspace::new(root) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("workspace initialization failed: {error}");
            std::process::exit(3)
        }
    };
    let plan = match kind {
        PlanInput::Request {
            request,
            assertions,
        } => match threadmoth::pipeline::prepare_request_plan(&workspace, &request, assertions) {
            Ok(plan) => plan,
            Err(certificate) => {
                emit_certificate(&certificate, true, args.summary);
                std::process::exit(2)
            }
        },
        PlanInput::Transaction {
            transaction,
            assertions,
        } => match threadmoth::pipeline::prepare_transaction_plan(
            &workspace,
            &transaction,
            assertions,
        ) {
            Ok(plan) => plan,
            Err(certificate) => {
                emit_transaction_certificate(&certificate, true, args.summary);
                std::process::exit(2)
            }
        },
    };
    let rendered = serde_json::to_string_pretty(&plan).expect("prepared plan serialises");
    if let Some(output) = args.output {
        if let Err(error) = fs::write(&output, format!("{rendered}\n")) {
            eprintln!("cannot write plan {}: {error}", output.display());
            std::process::exit(3)
        }
    }
    if args.summary {
        println!("THREADMOTH PLAN");
        println!("Plan ID      {}", plan.plan_id);
        println!("Operations   {}", plan.operations.len());
        println!("Assertions   {}", plan.assertions.len());
        println!("Safety       READY TO APPLY AGAINST EXACT PRE-IMAGES");
    } else {
        println!("{rendered}");
    }
}

#[cfg(windows)]
fn quote_powershell_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(unix)]
fn quote_posix_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(windows)]
const PLAN_COMMAND_SHELL: &str = "PowerShell";
#[cfg(unix)]
const PLAN_COMMAND_SHELL: &str = "POSIX shell";
#[cfg(not(any(windows, unix)))]
const PLAN_COMMAND_SHELL: &str = "supported shell";

fn quote_plan_output_path(output: &Path) -> Option<String> {
    let value = output.to_str()?;
    if value
        .chars()
        .any(|character| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
    {
        return None;
    }
    #[cfg(windows)]
    {
        Some(quote_powershell_argument(value))
    }
    #[cfg(unix)]
    {
        Some(quote_posix_argument(value))
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

fn run_prepare_text(args: PrepareTextArgs) {
    if args.summary && args.output.is_none() {
        eprintln!("--summary requires --output so the prepared plan can be reviewed and applied");
        std::process::exit(2);
    }
    let has_inline_edits = !args.edits.is_empty();
    let has_file_edits = !args.edit_files.is_empty();
    let has_compact_intent = args.file.is_some() || has_inline_edits || has_file_edits;
    if has_inline_edits && has_file_edits {
        eprintln!(
            "--edit and --edit-files cannot be mixed; use one input mode to preserve edit order"
        );
        std::process::exit(2);
    }
    if !has_compact_intent && args.newline_policy.is_some() {
        eprintln!("--newline-policy applies to compact PATH/--edit/--edit-files input; set newline_policy in the JSON request");
        std::process::exit(2);
    }
    if args.request.is_some() && has_compact_intent {
        eprintln!("--request cannot be combined with PATH, --edit, or --edit-files");
        std::process::exit(2);
    }
    let workspace = Workspace::new(env::current_dir().unwrap_or_else(|_| ".".into()))
        .unwrap_or_else(|error| {
            eprintln!("workspace initialization failed: {error}");
            std::process::exit(3);
        });
    let request = if !has_compact_intent {
        let input = match read_request_input(args.request.as_deref()) {
            Ok(input) => input,
            Err(RequestInputError::TooLarge(actual)) => {
                eprintln!(
                    "source-aware request exceeds {MAX_REQUEST_BYTES} bytes (actual: {actual})"
                );
                std::process::exit(2);
            }
            Err(RequestInputError::Io(error)) => {
                eprintln!("source-aware request read failed: {error}");
                std::process::exit(3);
            }
        };
        serde_json::from_str::<threadmoth::source_prepare::SourceAwareTextRequest>(&input)
            .unwrap_or_else(|error| {
                eprintln!("invalid source-aware request: {error}");
                std::process::exit(2);
            })
    } else {
        let Some(file) = args.file else {
            eprintln!("provide PATH and at least one --edit OLD NEW or --edit-files OLD_FILE NEW_FILE, or use --request");
            std::process::exit(2);
        };
        let edit_values = if has_file_edits {
            read_edit_files(&workspace, &args.edit_files)
        } else {
            args.edits
        };
        if edit_values.is_empty() || edit_values.len() % 2 != 0 {
            eprintln!("provide at least one complete --edit or --edit-files pair");
            std::process::exit(2);
        }
        let edits = edit_values
            .chunks_exact(2)
            .map(|pair| threadmoth::source_prepare::TextReplacementIntent {
                old: pair[0].clone(),
                new: pair[1].clone(),
            })
            .collect();
        threadmoth::source_prepare::SourceAwareTextRequest {
            file_path: file.to_string_lossy().into_owned(),
            edits,
            newline_policy: match args.newline_policy.unwrap_or_default() {
                cli::PrepareTextNewlinePolicy::Source => {
                    threadmoth::source_prepare::NewlinePolicy::Source
                }
                cli::PrepareTextNewlinePolicy::Exact => {
                    threadmoth::source_prepare::NewlinePolicy::Exact
                }
            },
            within: args.within,
        }
    };
    match threadmoth::source_prepare::prepare(&workspace, &request) {
        threadmoth::source_prepare::SourcePreparationResult::Prepared(prepared) => {
            if let Some(output) = args.output.as_deref() {
                if output_resolves_to_source(output, &request.file_path, &workspace) {
                    eprintln!("plan output must resolve safely and must not overwrite the edited source file");
                    std::process::exit(2);
                }
                let plan =
                    serde_json::to_vec_pretty(&prepared.plan).expect("prepared plan serialises");
                let mut file = match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output)
                {
                    Ok(file) => file,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        eprintln!(
                            "plan output already exists; choose a new path: {}",
                            output.display()
                        );
                        std::process::exit(2);
                    }
                    Err(error) => {
                        eprintln!("cannot create plan {}: {error}", output.display());
                        std::process::exit(3);
                    }
                };
                if let Err(error) = file.write_all(&plan) {
                    eprintln!("cannot write plan {}: {error}", output.display());
                    std::process::exit(3);
                }
            }
            if args.summary {
                let output = args.output.as_deref().expect("summary requires output");
                println!("THREADMOTH TEXT PREPARATION");
                println!("Source SHA-256 {}", prepared.source_sha256);
                println!("Newlines       {}", prepared.source_newline_profile);
                println!("Exact edits    {}", prepared.resolved_edits);
                println!("Plan ID        {}", prepared.plan.plan_id);
                if let Some(quoted_plan_path) = quote_plan_output_path(output) {
                    println!(
                        "Review ({PLAN_COMMAND_SHELL}): threadmoth explain --plan {quoted_plan_path} --format diff"
                    );
                    println!(
                        "Apply ({PLAN_COMMAND_SHELL}): threadmoth apply-plan --plan {quoted_plan_path}"
                    );
                } else {
                    println!(
                        "Review ({PLAN_COMMAND_SHELL}): unavailable (not executable; plan path cannot be represented faithfully as a safe single-line command)"
                    );
                    println!(
                        "Apply ({PLAN_COMMAND_SHELL}): unavailable (not executable; plan path cannot be represented faithfully as a safe single-line command)"
                    );
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&prepared).unwrap());
            }
        }
        threadmoth::source_prepare::SourcePreparationResult::Refused { certificate } => {
            if args.summary {
                print_prepare_refusal_summary(&certificate);
            } else {
                println!("{}", serde_json::to_string_pretty(&certificate).unwrap());
            }
            exit_for_outcome(certificate.outcome);
        }
    }
}

fn run_prepare_edits(args: PrepareEditsArgs) {
    if args.summary && args.output.is_none() {
        eprintln!("--summary requires --output so the prepared plan can be reviewed and applied");
        std::process::exit(2);
    }
    if args.document.is_absolute()
        || args.document.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        eprintln!("changes document must be named by a workspace-relative path without `..`");
        std::process::exit(2);
    }
    let workspace = Workspace::new(env::current_dir().unwrap_or_else(|_| ".".into()))
        .unwrap_or_else(|error| {
            eprintln!("workspace initialization failed: {error}");
            std::process::exit(3)
        });
    let document_path = args.document.to_string_lossy().replace('\\', "/");
    let document_path = match workspace
        .resolve_namespaced_path(&document_path, &threadmoth::path::PathNamespace::Native)
    {
        Ok(path) => path,
        Err(error) => {
            eprintln!("changes document path refused: {error}");
            std::process::exit(2)
        }
    };
    let bytes = workspace.read_file(&document_path).unwrap_or_else(|error| {
        eprintln!("cannot read changes document {document_path}: {error}");
        std::process::exit(3)
    });
    if bytes.len() > threadmoth::protocol::MAX_REQUEST_BYTES {
        eprintln!(
            "changes document exceeds {} bytes (actual: {})",
            threadmoth::protocol::MAX_REQUEST_BYTES,
            bytes.len()
        );
        std::process::exit(2);
    }
    let document = String::from_utf8(bytes).unwrap_or_else(|_| {
        eprintln!("changes document must be valid UTF-8");
        std::process::exit(2)
    });
    let prepared = match threadmoth::edit_document::prepare(&workspace, &document) {
        Ok(prepared) => prepared,
        Err(threadmoth::edit_document::EditDocumentFailure::Invalid(diagnostic)) => {
            if args.summary {
                println!("THREADMOTH EDIT PREPARATION");
                println!("Result       REFUSED");
                println!("Reason       {}", diagnostic.reason_code);
                println!("Line         {}", diagnostic.line);
                println!("Next step    {}", diagnostic.message);
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "outcome": "REFUSED",
                        "reason_code": diagnostic.reason_code,
                        "line": diagnostic.line,
                        "message": diagnostic.message
                    }))
                    .unwrap()
                );
            }
            std::process::exit(2)
        }
        Err(threadmoth::edit_document::EditDocumentFailure::Refused(certificate)) => {
            emit_transaction_certificate(&certificate, true, args.summary);
            exit_for_outcome(certificate.outcome.clone());
            unreachable!()
        }
    };

    let serialized_plan_bytes = serde_json::to_vec_pretty(&prepared.plan)
        .expect("prepared plan serialises")
        .len();
    if serialized_plan_bytes > threadmoth::protocol::MAX_PLAN_BYTES {
        eprintln!(
            "prepared plan exceeds {} bytes (actual: {})",
            threadmoth::protocol::MAX_PLAN_BYTES,
            serialized_plan_bytes
        );
        std::process::exit(2);
    }

    if let Some(output) = args.output.as_deref() {
        if output_resolves_to_source(output, &document_path, &workspace)
            || prepared.plan.operations.iter().any(|operation| {
                output_resolves_to_source(output, &operation.file_path, &workspace)
            })
        {
            eprintln!(
                "plan output must not resolve to the changes document or any edited source file"
            );
            std::process::exit(2);
        }
        let plan = serde_json::to_vec_pretty(&prepared.plan).expect("prepared plan serialises");
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                eprintln!(
                    "plan output already exists; choose a new path: {}",
                    output.display()
                );
                std::process::exit(2);
            }
            Err(error) => {
                eprintln!("cannot create plan {}: {error}", output.display());
                std::process::exit(3);
            }
        };
        if let Err(error) = file.write_all(&plan) {
            eprintln!("cannot write plan {}: {error}", output.display());
            std::process::exit(3);
        }
        if args.review {
            println!("THREADMOTH EDIT PREPARATION");
            println!("Files          {}", prepared.file_count);
            println!("Exact edits    {}", prepared.edit_count);
            println!("Input bytes    {}", prepared.input_bytes);
            println!("Plan ID        {}", prepared.plan.plan_id);
            let truncated =
                render_plan_review(&prepared.plan, Some(&workspace), ExplainFormat::Diff, false);
            if truncated {
                print_review_command(output, true);
            }
            print_apply_command(output);
        } else if args.summary {
            println!("THREADMOTH EDIT PREPARATION");
            println!("Files          {}", prepared.file_count);
            println!("Exact edits    {}", prepared.edit_count);
            println!("Input bytes    {}", prepared.input_bytes);
            println!("Plan ID        {}", prepared.plan.plan_id);
            print_edit_plan_commands(output);
        } else {
            println!("{}", serde_json::to_string_pretty(&prepared.plan).unwrap());
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&prepared.plan).unwrap());
    }
}

fn run_edit(args: EditArgs) {
    let workspace = Workspace::new(env::current_dir().unwrap_or_else(|_| ".".into()))
        .unwrap_or_else(|error| {
            eprintln!("workspace initialization failed: {error}");
            std::process::exit(3)
        });

    let target_str = args.target.to_string_lossy().into_owned();
    let is_stdin = target_str == "-";

    if args.search.is_some() != args.replace.is_some() {
        if args.search.is_some() {
            eprintln!("single-file editing with --search requires --replace");
        } else {
            eprintln!("single-file editing with --replace requires --search");
        }
        std::process::exit(2);
    }

    if let (Some(search), Some(replace)) = (&args.search, &args.replace) {
        if is_stdin {
            eprintln!(
                "single-file editing with --search and --replace requires a target file, not '-'"
            );
            std::process::exit(2);
        }
        let request = threadmoth::source_prepare::SourceAwareTextRequest {
            file_path: target_str.clone(),
            edits: vec![threadmoth::source_prepare::TextReplacementIntent {
                old: search.clone(),
                new: replace.clone(),
            }],
            newline_policy: threadmoth::source_prepare::NewlinePolicy::Source,
            within: args.within,
        };

        match threadmoth::source_prepare::prepare(&workspace, &request) {
            threadmoth::source_prepare::SourcePreparationResult::Prepared(prepared) => {
                if args.plan {
                    handle_edit_plan_output(
                        &prepared.plan,
                        args.output.as_deref(),
                        args.json,
                        &workspace,
                    );
                } else {
                    let apply_result =
                        threadmoth::pipeline::apply_prepared_plan(&workspace, &prepared.plan);
                    match apply_result {
                        PlanApplyResult::Certificate(certificate) => {
                            if args.json {
                                println!("{}", serde_json::to_string_pretty(&certificate).unwrap());
                            } else {
                                print_edit_certificate(
                                    &certificate,
                                    &workspace,
                                    &target_str,
                                    Some(search),
                                );
                            }
                            exit_for_outcome(certificate.outcome);
                        }
                        PlanApplyResult::Transaction(certificate) => {
                            if args.json {
                                println!("{}", serde_json::to_string_pretty(&certificate).unwrap());
                            } else {
                                print_edit_transaction(&certificate);
                            }
                            exit_for_outcome(certificate.outcome);
                        }
                    }
                }
            }
            threadmoth::source_prepare::SourcePreparationResult::Refused { certificate } => {
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&certificate).unwrap());
                } else {
                    print_edit_certificate(&certificate, &workspace, &target_str, Some(search));
                }
                exit_for_outcome(certificate.outcome);
            }
        }
    } else {
        if args.within.is_some() {
            eprintln!(
                "multi-file edit document mode does not accept --within; scope anchors are only valid for single-file inline edits with --search and --replace"
            );
            std::process::exit(2);
        }
        let document = if is_stdin {
            let mut buffer = Vec::new();
            io::stdin()
                .take((MAX_REQUEST_BYTES as u64).saturating_add(1))
                .read_to_end(&mut buffer)
                .unwrap_or_else(|error| {
                    eprintln!("cannot read edit document from stdin: {error}");
                    std::process::exit(3);
                });
            if buffer.len() > MAX_REQUEST_BYTES {
                eprintln!("stdin edit document exceeds {MAX_REQUEST_BYTES} bytes");
                std::process::exit(2);
            }
            String::from_utf8(buffer).unwrap_or_else(|_| {
                eprintln!("stdin edit document must be valid UTF-8");
                std::process::exit(2);
            })
        } else {
            let resolved = match workspace
                .resolve_namespaced_path(&target_str, &threadmoth::path::PathNamespace::Native)
            {
                Ok(path) => path,
                Err(error) => {
                    eprintln!("target path refused: {error}");
                    std::process::exit(2);
                }
            };
            let bytes = match workspace.read_file(&resolved) {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprintln!("cannot read target file {resolved}: {error}");
                    std::process::exit(3);
                }
            };
            if bytes.len() > MAX_REQUEST_BYTES {
                eprintln!("target file exceeds {MAX_REQUEST_BYTES} bytes");
                std::process::exit(2);
            }
            let text = String::from_utf8(bytes).unwrap_or_else(|_| {
                eprintln!("target file must be valid UTF-8");
                std::process::exit(2);
            });
            if !text.lines().any(|line| line.starts_with("*** File: ")) {
                eprintln!(
                    "single-file editing requires --search OLD and --replace NEW\nExample: threadmoth edit {} --search OLD --replace NEW\nFor multi-file edits, use an edit document with `*** File: PATH` headers.",
                    target_str
                );
                std::process::exit(2);
            }
            text
        };

        match threadmoth::edit_document::prepare(&workspace, &document) {
            Ok(prepared) => {
                if args.plan {
                    handle_edit_plan_output(
                        &prepared.plan,
                        args.output.as_deref(),
                        args.json,
                        &workspace,
                    );
                } else {
                    let apply_result =
                        threadmoth::pipeline::apply_prepared_plan(&workspace, &prepared.plan);
                    match apply_result {
                        PlanApplyResult::Transaction(certificate) => {
                            if args.json {
                                println!("{}", serde_json::to_string_pretty(&certificate).unwrap());
                            } else {
                                print_edit_transaction(&certificate);
                            }
                            exit_for_outcome(certificate.outcome);
                        }
                        PlanApplyResult::Certificate(certificate) => {
                            if args.json {
                                println!("{}", serde_json::to_string_pretty(&certificate).unwrap());
                            } else {
                                print_edit_certificate(&certificate, &workspace, &target_str, None);
                            }
                            exit_for_outcome(certificate.outcome);
                        }
                    }
                }
            }
            Err(threadmoth::edit_document::EditDocumentFailure::Invalid(diagnostic)) => {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "outcome": "REFUSED",
                            "reason_code": diagnostic.reason_code,
                            "line": diagnostic.line,
                            "message": diagnostic.message
                        }))
                        .unwrap()
                    );
                } else {
                    println!("THREADMOTH EDIT REFUSED");
                    println!("Result       REFUSED");
                    println!("Reason       {}", diagnostic.reason_code);
                    println!("Line         {}", diagnostic.line);
                    println!("Details      {}", diagnostic.message);
                    println!(
                        "Next step    Correct the edit document formatting at line {}",
                        diagnostic.line
                    );
                    println!("Source       unchanged");
                }
                std::process::exit(2);
            }
            Err(threadmoth::edit_document::EditDocumentFailure::Refused(certificate)) => {
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&certificate).unwrap());
                } else {
                    print_edit_transaction(&certificate);
                }
                exit_for_outcome(certificate.outcome);
            }
        }
    }
}

fn handle_edit_plan_output(
    plan: &PreparedPlan,
    output: Option<&Path>,
    json_mode: bool,
    workspace: &Workspace,
) {
    if let Some(output) = output {
        if plan
            .operations
            .iter()
            .any(|op| output_resolves_to_source(output, &op.file_path, workspace))
        {
            eprintln!("plan output must not overwrite any edited source file");
            std::process::exit(2);
        }
        let plan_bytes = serde_json::to_vec_pretty(plan).expect("prepared plan serialises");
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                eprintln!(
                    "plan output already exists; choose a new path: {}",
                    output.display()
                );
                std::process::exit(2);
            }
            Err(error) => {
                eprintln!("cannot create plan {}: {error}", output.display());
                std::process::exit(3);
            }
        };
        if let Err(error) = file.write_all(&plan_bytes) {
            eprintln!("cannot write plan {}: {error}", output.display());
            std::process::exit(3);
        }
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(plan).unwrap());
    } else {
        println!("THREADMOTH EDIT PLAN");
        println!("Operations   {}", plan.operations.len());
        println!("Plan ID      {}", plan.plan_id);
        let _ = render_plan_review(plan, Some(workspace), ExplainFormat::Diff, false);
        if let Some(output) = output {
            print_apply_command(output);
        } else {
            println!("Note         This plan was previewed in memory and not saved to disk.");
            println!("To apply this edit, run without `--plan`, or pass `--output plan.json` to persist.");
        }
    }
}

fn print_edit_certificate(
    certificate: &Certificate,
    workspace: &Workspace,
    file_path: &str,
    search_term: Option<&str>,
) {
    match certificate.outcome {
        Outcome::Applied => {
            println!("THREADMOTH EDIT COMMITTED");
            println!("File         {}", certificate.file_path);
            println!("Result       APPLIED");
            println!(
                "Change       {} line(s), {} byte(s), {} match(es)",
                certificate.effect.changed_lines,
                certificate.effect.changed_bytes,
                certificate.effect.matches
            );
            println!("Guarantee    {}", certificate.commit.mode);
        }
        Outcome::NoChange => {
            println!("THREADMOTH EDIT NO CHANGE");
            println!("File         {}", certificate.file_path);
            println!("Result       NO_CHANGE");
            println!("Details      The target was matched but the replacement produces no change.");
            println!("Guarantee    {}", certificate.commit.mode);
            println!("Source       unchanged");
        }
        Outcome::Refused => {
            print_edit_refusal(certificate, workspace, file_path, search_term);
        }
        Outcome::Failed => {
            print_edit_failure(certificate, workspace, file_path);
        }
    }
}

fn print_edit_transaction(certificate: &TransactionCertificate) {
    match certificate.outcome {
        Outcome::Applied => {
            println!("THREADMOTH TRANSACTION COMMITTED");
            println!("Files        {}", certificate.certificates.len());
            println!("Guarantee    {}", certificate.transaction_guarantee);
            println!("Result       APPLIED");
        }
        Outcome::NoChange => {
            println!("THREADMOTH TRANSACTION NO CHANGE");
            println!("Files        {}", certificate.certificates.len());
            println!("Guarantee    {}", certificate.transaction_guarantee);
            println!("Result       NO_CHANGE");
            println!("Source       unchanged");
        }
        Outcome::Refused => {
            print_transaction_summary(certificate, false);
        }
        Outcome::Failed => {
            print_transaction_failure(certificate);
        }
    }
}

fn print_edit_failure(certificate: &Certificate, _workspace: &Workspace, file_path: &str) {
    println!("THREADMOTH EDIT FAILED");
    println!("File         {}", file_path);
    println!("Result       FAILED");
    let code = certificate.reason_code.as_deref().unwrap_or("FAILED");
    println!("Reason       {}", code);
    if let Some(details) = &certificate.failure_reason {
        println!("Details      {details:?}");
    }
    println!("Recovery     {}", certificate.recovery_state);

    if certificate.recovery_state == "manual_recovery_required" {
        println!("WARNING      Rollback failed or incomplete! Run `threadmoth recover` to inspect or restore journaled files.");
        println!("Source       DIRTY (manual recovery required)");
    } else if certificate.recovery_state == "rolled_back" {
        println!("Source       unchanged (rolled back to original state)");
    } else if certificate.recovery_state == "not_started" {
        println!("Source       unchanged (failed before modifying source)");
    } else {
        println!("Source       unknown (verify with `threadmoth recover` or inspect file)");
    }
}

fn print_transaction_failure(certificate: &TransactionCertificate) {
    println!("THREADMOTH TRANSACTION FAILED");
    println!("Result       FAILED");
    if let Some(code) = certificate.reason_code.as_deref() {
        println!("Reason       {code}");
    }
    if let Some(details) = &certificate.failure_reason {
        println!("Details      {details:?}");
    }
    println!("Recovery     {}", certificate.rollback_state);

    if certificate.rollback_state == "manual_recovery_required" {
        println!("WARNING      Rollback failed or incomplete! Run `threadmoth recover` to inspect or restore journaled files.");
        println!("Source       DIRTY (manual recovery required)");
    } else if certificate.rollback_state == "rolled_back" {
        println!("Source       unchanged (all modified files rolled back)");
    } else if certificate.rollback_state == "not_started" {
        println!("Source       unchanged (failed before modifying source)");
    } else {
        println!("Source       unknown (verify with `threadmoth recover`)");
    }
}

fn print_edit_refusal(
    certificate: &Certificate,
    workspace: &Workspace,
    file_path: &str,
    search_term: Option<&str>,
) {
    println!("THREADMOTH EDIT REFUSED");
    println!("File         {}", file_path);
    let code = certificate.reason_code.as_deref().unwrap_or("REFUSED");
    println!("Reason       {}", code);

    if code == "TARGET_AMBIGUOUS" {
        let mut candidates_shown = false;
        if let Some(RefusalReason::DuplicateTarget {
            count, candidates, ..
        }) = &certificate.refusal_reason
        {
            if !candidates.is_empty() {
                println!("Candidates   Found {count} occurrences in {file_path}:");
                for c in candidates {
                    let single_line = c.context.replace(['\r', '\n'], " ");
                    let trimmed = single_line.trim();
                    println!(
                        "               Line {}: {}",
                        c.line,
                        truncate_chars(trimmed, 60)
                    );
                }
                candidates_shown = true;
            } else {
                println!("Candidates   Found {count} occurrences in {file_path}");
            }
        } else if certificate.effect.matches > 1 {
            println!(
                "Candidates   Found {} occurrences in {file_path}",
                certificate.effect.matches
            );
        }
        if let Some(diff) = certificate.diff_summary.as_deref() {
            println!("Details      {diff}");
        }
        if candidates_shown {
            println!(
                "Next step    Provide additional surrounding context in --search to uniquely establish the target, or use `threadmoth edit changes.tm` with ordered blocks."
            );
        } else {
            println!(
                "Next step    Inspect the file with `threadmoth inspect {file_path}` to find line numbers and context, or provide more surrounding lines in --search."
            );
        }
    } else if code == "TARGET_NOT_FOUND" {
        println!("Details      The search target was not found in the source file.");
        if let Some(term) = search_term {
            if term.contains('\n')
                || term.contains('"')
                || term.contains('\'')
                || term.contains('`')
                || term.contains('$')
            {
                println!(
                    "Next step    If the search target contains newlines or shell-sensitive characters, provide it via stdin (`threadmoth edit -`) or a changes document (`threadmoth edit changes.tm`)."
                );
            } else {
                println!(
                    "Next step    Check the exact target text against the current file. For multiline edits, use `threadmoth edit -` or `threadmoth edit changes.tm`."
                );
            }
        } else {
            println!(
                "Next step    Check the exact target text against the current file. For multiline edits, use `threadmoth edit -` or `threadmoth edit changes.tm`."
            );
        }
    } else if code == "PLAN_STALE" || code == "STALE_IDENTITY" {
        println!("Details      The source file was modified concurrently or changed on disk since observation.");
        println!("Next step    Re-run the edit command against the latest source state.");
    } else if code == "WORKSPACE_ESCAPE" || code == "WORKSPACE_ROOT_MISMATCH" {
        println!("Details      The target path is outside the authorized workspace root.");
        println!(
            "Next step    Target a workspace-relative file path within {}",
            workspace.root().display()
        );
    } else if code == "EFFECT_BUDGET_EXCEEDED" {
        println!("Details      The requested edit exceeds the safety effect budget.");
        let suggestions = budget_suggestions(certificate);
        if !suggestions.is_empty() {
            println!("Next step    Required budget: {}", suggestions.join(", "));
        }
    } else {
        if let Some(details) = &certificate.failure_reason {
            println!("Details      {details:?}");
        }
        let ext = Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if matches!(ext, "json" | "jsonc" | "toml" | "yaml" | "yml" | "env") {
            println!("Next step    Inspect targets with `threadmoth suggest {file_path}` or inspect capabilities with `threadmoth capabilities --for {file_path}`.");
        } else {
            println!("Next step    Inspect capabilities with `threadmoth capabilities --for {file_path}` or get guidance with `threadmoth explain {code}`.");
        }
    }

    let ext = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if matches!(ext, "json" | "jsonc" | "toml" | "yaml" | "yml" | "env")
        && (code == "TARGET_AMBIGUOUS" || code == "TARGET_NOT_FOUND")
    {
        println!("Hint         This is a structured format ({ext}). You can inspect targets with `threadmoth suggest {file_path}`.");
    }

    println!("Source       unchanged");
}

fn print_edit_plan_commands(output: &Path) {
    print_review_command(output, false);
    print_apply_command(output);
}

fn print_review_command(output: &Path, full: bool) {
    let label = if full { "Full review" } else { "Review" };
    let full_option = if full { " --full" } else { "" };
    if let Some(quoted_plan_path) = quote_plan_output_path(output) {
        println!(
            "{label} ({PLAN_COMMAND_SHELL}): threadmoth explain --plan {quoted_plan_path} --format diff{full_option}"
        );
    } else {
        println!(
            "{label} ({PLAN_COMMAND_SHELL}): unavailable (not executable; plan path cannot be represented faithfully as a safe single-line command)"
        );
    }
}

fn print_apply_command(output: &Path) {
    if let Some(quoted_plan_path) = quote_plan_output_path(output) {
        println!("Apply ({PLAN_COMMAND_SHELL}): threadmoth apply-plan --plan {quoted_plan_path}");
    } else {
        println!(
            "Apply ({PLAN_COMMAND_SHELL}): unavailable (not executable; plan path cannot be represented faithfully as a safe single-line command)"
        );
    }
}

fn print_prepare_refusal_summary(certificate: &Certificate) {
    println!("THREADMOTH TEXT PREPARATION");
    println!("Result       REFUSED");
    if let Some(code) = certificate.reason_code.as_deref() {
        println!("Reason       {code}");
        if let Some(reason) = threadmoth::metadata::reason(code) {
            println!("Recovery     {}", reason.why_refused);
            if !reason.relevant_commands.is_empty() {
                println!("Options      {}", reason.relevant_commands.join(", "));
            }
        }
    }
    if let Some(evidence) = certificate
        .diff_summary
        .as_deref()
        .filter(|summary| summary.contains("line ending discrepancy"))
    {
        println!("Evidence     {evidence}");
    }
    if let Some(remedy) = certificate.recovery.as_ref().and_then(|recovery| {
        recovery
            .remedies
            .iter()
            .find(|remedy| remedy.kind == "use_file_based_text_input")
    }) {
        println!("Next step    {}", remedy.description);
    }
    println!("Source       unchanged");
}

fn read_edit_files(workspace: &Workspace, paths: &[std::path::PathBuf]) -> Vec<String> {
    let mut total_bytes = 0usize;
    paths
        .iter()
        .map(|input_path| {
            let input_display = input_path.to_str().unwrap_or_else(|| {
                eprintln!(
                    "--edit-files input path is not valid Unicode: {}",
                    input_path.display()
                );
                std::process::exit(2);
            });
            let relative = workspace
                .resolve_namespaced_path(input_display, &threadmoth::path::PathNamespace::Native)
                .unwrap_or_else(|error| {
                    eprintln!("--edit-files input path refused: {error}");
                    std::process::exit(2);
                });
            let path = workspace.resolve_path(&relative).unwrap_or_else(|error| {
                eprintln!("--edit-files input path refused: {error}");
                std::process::exit(2);
            });
            if !path.is_file() {
                eprintln!("--edit-files input is not a file: {}", path.display());
                std::process::exit(2);
            }
            let remaining = MAX_REQUEST_BYTES.saturating_sub(total_bytes);
            let mut bytes = Vec::new();
            fs::File::open(&path)
                .unwrap_or_else(|error| {
                    eprintln!("cannot read --edit-files input {}: {error}", path.display());
                    std::process::exit(3);
                })
                .take((remaining as u64).saturating_add(1))
                .read_to_end(&mut bytes)
                .unwrap_or_else(|error| {
                    eprintln!("cannot read --edit-files input {}: {error}", path.display());
                    std::process::exit(3);
                });
            if bytes.len() > remaining {
                eprintln!("combined --edit-files input exceeds {MAX_REQUEST_BYTES} bytes");
                std::process::exit(2);
            }
            total_bytes += bytes.len();
            String::from_utf8(bytes).unwrap_or_else(|_| {
                eprintln!("--edit-files input is not valid UTF-8: {}", path.display());
                std::process::exit(2);
            })
        })
        .collect()
}

fn output_resolves_to_source(output: &Path, source: &str, workspace: &Workspace) -> bool {
    let Some(source_path) = workspace
        .resolve_namespaced_path(source, &threadmoth::path::PathNamespace::Native)
        .ok()
        .and_then(|path| fs::canonicalize(path).ok())
    else {
        return true;
    };
    let output_path = if output.is_absolute() {
        output.to_path_buf()
    } else {
        let Ok(current_dir) = env::current_dir() else {
            return true;
        };
        current_dir.join(output)
    };
    let canonical_output = if output_path.exists() {
        fs::canonicalize(&output_path).ok()
    } else {
        output_path
            .parent()
            .and_then(|parent| fs::canonicalize(parent).ok())
            .and_then(|parent| output_path.file_name().map(|name| parent.join(name)))
    };
    let Some(canonical_output) = canonical_output else {
        return true;
    };
    #[cfg(windows)]
    {
        source_path
            .to_string_lossy()
            .eq_ignore_ascii_case(&canonical_output.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        source_path == canonical_output
    }
}

fn run_apply_plan(args: ApplyPlanArgs) {
    let input = match fs::read(&args.plan) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("plan read failed: {error}");
            std::process::exit(3)
        }
    };
    if input.len() > MAX_PLAN_BYTES {
        eprintln!(
            "plan exceeds {MAX_PLAN_BYTES} bytes (actual: {})",
            input.len()
        );
        std::process::exit(2)
    }
    let plan: PreparedPlan = match serde_json::from_slice(&input) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("plan refused: {error}");
            std::process::exit(2)
        }
    };
    let root = env::current_dir().unwrap_or_else(|_| ".".into());
    let workspace = match Workspace::new(root) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("workspace initialization failed: {error}");
            std::process::exit(3)
        }
    };
    let result = threadmoth::pipeline::apply_prepared_plan(&workspace, &plan);
    let outcome = match &result {
        PlanApplyResult::Certificate(certificate) => {
            if args.summary {
                print_certificate_summary(certificate, false);
            } else {
                println!("{}", serde_json::to_string_pretty(certificate).unwrap());
            }
            certificate.outcome.clone()
        }
        PlanApplyResult::Transaction(certificate) => {
            if args.summary {
                print_transaction_summary(certificate, false);
            } else {
                println!("{}", serde_json::to_string_pretty(certificate).unwrap());
            }
            certificate.outcome.clone()
        }
    };
    exit_for_outcome(outcome);
}

fn run_explain_plan(path: &Path, json: bool, format: Option<ExplainFormat>, full: bool) {
    let input = match fs::read(path) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("plan read failed: {error}");
            std::process::exit(3)
        }
    };
    if input.len() > MAX_PLAN_BYTES {
        eprintln!(
            "plan exceeds {MAX_PLAN_BYTES} bytes (actual: {})",
            input.len()
        );
        std::process::exit(2)
    }
    let plan: PreparedPlan = match serde_json::from_slice(&input) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("plan refused: {error}");
            std::process::exit(2)
        }
    };
    let workspace = env::current_dir()
        .ok()
        .and_then(|root| Workspace::new(root).ok());
    let refusal = workspace
        .as_ref()
        .and_then(|workspace| threadmoth::pipeline::check_prepared_plan(workspace, &plan).err());
    let safe = workspace.is_some() && refusal.is_none();
    if !json {
        if let Some(format) = format {
            let truncated = render_plan_review(&plan, workspace.as_ref(), format, full);
            if truncated && !full {
                print_review_command(path, true);
            }
            println!("Next: {}", plan_next_action(path, safe));
            return;
        }
    }
    if json {
        println!(
            "{}",
            serde_json::json!({
                "plan_id": plan.plan_id,
                "protocol_version": plan.protocol_version,
                "operations": plan.operations.len(),
                "assertions": plan.assertions.len(),
                "safe_to_apply": safe,
                "next_action": plan_next_action(path, safe),
                "refusal_code": refusal.as_ref().map(|reason| reason.code()),
                "refusal": refusal
            })
        );
    } else {
        println!("Plan {}", plan.plan_id);
        println!("Protocol     {}", plan.protocol_version);
        println!("Operations   {}", plan.operations.len());
        for operation in &plan.operations {
            println!(
                "File         {}\nProvider     {}\nPre-image    {}\nEdits        {}\nProspective  {}",
                operation.file_path,
                operation.provider,
                operation.pre_hash,
                operation.edits.len(),
                operation.prospective_hash
            );
        }
        println!("Assertions   {}", plan.assertions.len());
        println!(
            "Safe to apply: {}",
            if safe {
                "YES (pre-images rechecked at apply)"
            } else {
                "NO"
            }
        );
        if let Some(reason) = refusal {
            println!("Reason: {} ({reason:?})", reason.code());
        }
        println!("Next: {}", plan_next_action(path, safe));
    }
}

fn plan_next_action(path: &Path, safe: bool) -> String {
    if safe {
        format!("threadmoth apply-plan --plan \"{}\"", path.display())
    } else {
        "prepare a new plan from the current source using the same intent, then review it".into()
    }
}

fn render_plan_review(
    plan: &PreparedPlan,
    workspace: Option<&Workspace>,
    format: ExplainFormat,
    full: bool,
) -> bool {
    let title = match format {
        ExplainFormat::Diff => "Threadmoth plan diff",
        ExplainFormat::Markdown => "# Threadmoth plan review",
    };
    println!(
        "{title}\nPlan: {}\nProtocol: {}",
        plan.plan_id, plan.protocol_version
    );
    let mut truncated = false;
    for operation in &plan.operations {
        let Some(workspace) = workspace else {
            println!("\n{}\nstate: UNAVAILABLE", operation.file_path);
            continue;
        };
        let current = workspace
            .resolve_namespaced_path(&operation.file_path, &operation.request.namespace)
            .ok()
            .and_then(|path| workspace.read_file(path).ok());
        let Some(original) = current else {
            println!("\n{}\nstate: UNAVAILABLE", operation.file_path);
            continue;
        };
        let current_hash = threadmoth::engine::compute_sha256(&original);
        let state = if current_hash == operation.pre_hash {
            "FRESH"
        } else {
            "STALE"
        };
        let edits = operation
            .edits
            .iter()
            .map(|edit| threadmoth::engine::ByteEdit {
                start: edit.offset,
                end: edit.offset.saturating_add(edit.delete_len),
                replacement: edit.replacement.clone(),
            })
            .collect::<Vec<_>>();
        let candidate = threadmoth::engine::apply_byte_edits(&original, &edits).ok();
        let diff = candidate
            .as_ref()
            .map(|candidate| {
                similar::TextDiff::from_lines(
                    &String::from_utf8_lossy(&original),
                    &String::from_utf8_lossy(candidate),
                )
                .unified_diff()
                .context_radius(3)
                .header("before", "after")
                .to_string()
            })
            .unwrap_or_else(|| "stored edits are invalid".into());
        match format {
            ExplainFormat::Diff => println!(
                "\n--- {}\n+++ {}\nprovider: {}\nstate: {}\npre_hash: {}\nprospective_hash: {}\n{}",
                operation.file_path,
                operation.file_path,
                operation.provider,
                state,
                operation.pre_hash,
                operation.prospective_hash,
                bounded_review(&diff, full),
            ),
            ExplainFormat::Markdown => println!(
                "\n## {}\n\n- Provider: `{}`\n- State: **{}**\n- Pre-image: `{}`\n- Prospective: `{}`\n- Edits: `{}`\n\n```diff\n{}\n```",
                operation.file_path,
                operation.provider,
                state,
                operation.pre_hash,
                operation.prospective_hash,
                operation.edits.len(),
                bounded_review(&diff, full),
            ),
        }
        truncated |= !full && diff.chars().count() > 4096;
    }
    truncated
}

fn bounded_review(value: &str, full: bool) -> String {
    if full {
        return value.to_string();
    }
    let mut result = value.chars().take(4096).collect::<String>();
    if value.chars().count() > 4096 {
        result.push_str("\n... review output truncated; use the Full review command ...");
    }
    result
}

#[allow(clippy::large_enum_variant)]
enum PlanInput {
    Request {
        request: Request,
        assertions: Vec<Assertion>,
    },
    Transaction {
        transaction: TransactionRequest,
        assertions: Vec<Assertion>,
    },
}

fn parse_plan_input(input: &str) -> Result<PlanInput, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(input).map_err(|error| error.to_string())?;
    let assertions = value
        .as_object_mut()
        .and_then(|object| object.remove("assertions"))
        .map(|value| {
            serde_json::from_value::<Vec<Assertion>>(value).map_err(|error| error.to_string())
        })
        .transpose()?
        .unwrap_or_default();
    if let Some(envelope) = value
        .as_object_mut()
        .and_then(|object| object.remove("request"))
    {
        value = envelope;
    }
    if value.get("transaction_id").is_some() {
        let transaction = serde_json::from_value::<TransactionRequest>(value)
            .map_err(|error| error.to_string())?;
        Ok(PlanInput::Transaction {
            transaction,
            assertions,
        })
    } else {
        let request =
            serde_json::from_value::<Request>(value).map_err(|error| error.to_string())?;
        Ok(PlanInput::Request {
            request,
            assertions,
        })
    }
}

fn run_request(request_path: Option<&Path>, dry: bool, summary: bool) {
    let input = match read_request_input(request_path) {
        Ok(s) => s,
        Err(RequestInputError::TooLarge(actual)) => {
            let certificate = empty_cert(RefusalReason::ResourceLimitExceeded {
                dimension: "max_request_bytes".into(),
                limit: MAX_REQUEST_BYTES,
                actual,
            });
            emit_certificate(&certificate, dry, summary);
            std::process::exit(2)
        }
        Err(RequestInputError::Io(error)) => {
            eprintln!("request read failed: {error}");
            std::process::exit(3)
        }
    };
    let req: Request = match serde_json::from_str(&input) {
        Ok(r) => r,
        Err(e) => {
            let mut certificate = empty_cert(RefusalReason::MalformedInput {
                details: e.to_string(),
            });
            certificate.schema_diagnostic =
                Some(threadmoth::protocol::schema_diagnostic(&e.to_string()));
            emit_certificate(&certificate, dry, summary);
            std::process::exit(2)
        }
    };
    let root = match env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cannot determine workspace: {e}");
            std::process::exit(3)
        }
    };
    let ws = match Workspace::new(root) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("workspace initialization failed: {e}");
            std::process::exit(3)
        }
    };
    let cert = execute_request(&ws, &req, dry);
    emit_certificate(&cert, dry, summary);
    exit_for_outcome(cert.outcome);
}

fn run_transaction(request_path: Option<&Path>, dry: bool, summary: bool) {
    let input = match read_request_input(request_path) {
        Ok(s) => s,
        Err(RequestInputError::TooLarge(actual)) => {
            let certificate = empty_transaction_certificate(RefusalReason::ResourceLimitExceeded {
                dimension: "max_request_bytes".into(),
                limit: MAX_REQUEST_BYTES,
                actual,
            });
            emit_transaction_certificate(&certificate, dry, summary);
            std::process::exit(2)
        }
        Err(RequestInputError::Io(error)) => {
            eprintln!("request read failed: {error}");
            std::process::exit(3);
        }
    };
    let transaction: TransactionRequest = match serde_json::from_str(&input) {
        Ok(x) => x,
        Err(e) => {
            let mut certificate = empty_transaction_certificate(RefusalReason::MalformedInput {
                details: format!("transaction request parse failed: {e}"),
            });
            certificate.schema_diagnostic =
                Some(threadmoth::protocol::schema_diagnostic(&e.to_string()));
            emit_transaction_certificate(&certificate, dry, summary);
            std::process::exit(2);
        }
    };
    let root = env::current_dir().unwrap_or_else(|_| ".".into());
    let ws = match Workspace::new(root) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("workspace initialization failed: {e}");
            std::process::exit(3)
        }
    };
    let certificate = threadmoth::pipeline::execute_transaction(&ws, &transaction, dry);
    emit_transaction_certificate(&certificate, dry, summary);
    exit_for_outcome(certificate.outcome);
}

fn emit_certificate(certificate: &Certificate, dry: bool, summary: bool) {
    if summary {
        print_certificate_summary(certificate, dry);
    } else {
        println!("{}", serde_json::to_string_pretty(certificate).unwrap());
    }
}

fn emit_transaction_certificate(certificate: &TransactionCertificate, dry: bool, summary: bool) {
    if summary {
        print_transaction_summary(certificate, dry);
    } else {
        println!("{}", serde_json::to_string_pretty(certificate).unwrap());
    }
}

fn print_certificate_summary(certificate: &Certificate, dry: bool) {
    println!("THREADMOTH {}", if dry { "PREVIEW" } else { "MUTATION" });
    println!("{}", "─".repeat(72));
    println!("Result       {}", outcome_name(&certificate.outcome));
    if !certificate.file_path.is_empty() {
        println!("File         {}", certificate.file_path);
    }
    println!(
        "Provider     {} ({})",
        certificate.provider, certificate.provider_version
    );
    if let Some(reason_code) = certificate.reason_code.as_deref() {
        println!("Reason       {reason_code}");
    }
    println!(
        "Effect       {} file(s), {} match(es), {} region(s), {} line(s), {} byte(s)",
        certificate.effect.files,
        certificate.effect.matches,
        certificate.effect.changed_regions,
        certificate.effect.changed_lines,
        certificate.effect.changed_bytes,
    );
    println!(
        "Budget       {}",
        if certificate.effect.passed {
            "PASS"
        } else {
            "REFUSED"
        }
    );
    let suggestions = budget_suggestions(certificate);
    if !suggestions.is_empty() {
        println!("Minimum      {}", suggestions.join(", "));
    }
    if certificate.preservation.original_newline_profile != "unknown"
        || certificate.preservation.result_newline_profile != "unknown"
    {
        println!(
            "Newlines     {} -> {}",
            certificate.preservation.original_newline_profile,
            certificate.preservation.result_newline_profile
        );
    }
    println!(
        "Preservation unrelated_bytes_changed={} bom_changed={} final_newline_changed={}",
        certificate.preservation.unrelated_bytes_changed,
        certificate.preservation.bom_changed,
        certificate.preservation.final_newline_changed,
    );
    if !certificate.pre_hash.is_empty() {
        println!("Pre SHA-256  {}", certificate.pre_hash);
    }
    if let Some(post_hash) = certificate.post_hash.as_deref() {
        println!("Post SHA-256 {post_hash}");
    }
    println!("Commit       {}", certificate.commit.mode);
    if certificate.diff_summary.is_some() {
        println!(
            "Diff         available in full JSON certificate{}",
            if certificate.diff_truncated {
                " (bounded/truncated)"
            } else {
                ""
            }
        );
    }
    if !certificate.diagnostics.is_empty() {
        println!("Diagnostics  {}", certificate.diagnostics.join(" | "));
    }
}

fn print_transaction_summary(certificate: &TransactionCertificate, dry: bool) {
    println!(
        "THREADMOTH TRANSACTION {}",
        if dry { "PREVIEW" } else { "RESULT" }
    );
    println!("{}", "─".repeat(72));
    println!("Result       {}", outcome_name(&certificate.outcome));
    if !certificate.transaction_id.is_empty() {
        println!("Transaction  {}", certificate.transaction_id);
    }
    println!("Members      {}", certificate.certificates.len());
    println!("Guarantee    {}", certificate.transaction_guarantee);
    println!("Recovery     {}", certificate.rollback_state);
    if let Some(reason_code) = certificate.reason_code.as_deref() {
        println!("Reason       {reason_code}");
    }
    if !certificate.certificates.is_empty() {
        println!();
        println!("{:<36} {:>10} {:>9}", "File", "Result", "Regions");
        println!("{}", "─".repeat(72));
        for member in &certificate.certificates {
            println!(
                "{:<36} {:>10} {:>9}",
                truncate_chars(&member.file_path, 36),
                outcome_name(&member.outcome),
                member.effect.changed_regions,
            );
        }
    }
}

fn budget_suggestions(certificate: &Certificate) -> Vec<String> {
    let budget = &certificate.budget;
    let effect = &certificate.effect;
    let mut values = Vec::new();
    push_budget_suggestion(&mut values, "max_files", budget.max_files, effect.files);
    push_budget_suggestion(
        &mut values,
        "max_matches",
        budget.max_matches,
        effect.matches,
    );
    push_budget_suggestion(
        &mut values,
        "max_changed_regions",
        budget.max_changed_regions,
        effect.changed_regions,
    );
    push_budget_suggestion(
        &mut values,
        "max_changed_lines",
        budget.max_changed_lines,
        effect.changed_lines,
    );
    push_budget_suggestion(
        &mut values,
        "max_changed_bytes",
        budget.max_changed_bytes,
        effect.changed_bytes,
    );
    values
}

fn push_budget_suggestion(
    values: &mut Vec<String>,
    name: &str,
    limit: Option<usize>,
    actual: usize,
) {
    if limit.is_some_and(|limit| actual > limit) {
        values.push(format!("{name}={actual}"));
    }
}

fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Applied => "APPLIED",
        Outcome::NoChange => "NO_CHANGE",
        Outcome::Refused => "REFUSED",
        Outcome::Failed => "FAILED",
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.into();
    }
    let mut output = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

fn exit_for_outcome(outcome: Outcome) {
    match outcome {
        Outcome::Refused => std::process::exit(2),
        Outcome::Failed => std::process::exit(3),
        Outcome::Applied | Outcome::NoChange => {}
    }
}

fn run_recover(args: RecoverArgs) {
    let root = env::current_dir().unwrap_or_else(|_| ".".into());
    let ws = match Workspace::new(root) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("workspace initialization failed: {e}");
            std::process::exit(3)
        }
    };
    let output = if args.list {
        serde_json::to_value(threadmoth::recovery::list(&ws)).unwrap()
    } else if let Some(transaction_id) = args.inspect {
        serde_json::to_value(threadmoth::recovery::inspect(&ws, &transaction_id)).unwrap()
    } else if let Some(transaction_id) = args.transaction {
        serde_json::to_value(threadmoth::recovery::recover_transaction(
            &ws,
            &transaction_id,
        ))
        .unwrap()
    } else {
        serde_json::to_value(threadmoth::recovery::recover_all(&ws)).unwrap()
    };
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

fn run_capabilities(args: CapabilitiesArgs) {
    let _ = args.all;
    let mut output = threadmoth::metadata::capability_view(args.selector.as_deref());
    if let Some(path) = args.for_path {
        let root = env::current_dir().unwrap_or_else(|_| ".".into());
        let ws = Workspace::new(root).unwrap_or_else(|e| {
            eprintln!("workspace initialization failed: {e}");
            std::process::exit(3)
        });
        let display = path.to_string_lossy();
        let bytes = ws.read_file(display.as_ref()).ok();
        output = threadmoth::metadata::capabilities_for(display.as_ref(), bytes.as_deref());
    }
    let rendered = if args.json && !args.pretty {
        serde_json::to_string(&output).unwrap()
    } else {
        serde_json::to_string_pretty(&output).unwrap()
    };
    println!("{rendered}");
}

fn print_examples(topic: Option<&str>) {
    if topic.is_some_and(|topic| topic == "source-text") {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "topic": "source-text",
            "intent": "Replace exact text using source-aware preparation.",
            "prepare": "threadmoth prepare-text PATH --edit OLD NEW --output PLAN --summary",
            "prepare_from_files": "threadmoth prepare-text PATH --edit-files OLD_FILE NEW_FILE --output PLAN --summary",
            "prepare_multiple_files": "threadmoth prepare-edits CHANGES --output PLAN --review",
            "review": "prepare-edits --review shows the bounded diff; use explain --plan PLAN --format diff --full if truncated",
            "apply": "threadmoth apply-plan --plan PLAN --summary",
            "recovery": "If review says stale, prepare again from current source with the same intent.",
            "guarantees": ["exact unique targets", "source-derived uniform LF/CRLF", "source-read-only preparation", "stale-plan revalidation", "certificate on apply"]
        })).unwrap());
        return;
    }
    let examples = threadmoth::metadata::examples(topic);
    if examples.is_empty() {
        eprintln!("no example topic matched");
        std::process::exit(1);
    }
    println!("{}", serde_json::to_string_pretty(&examples).unwrap());
}

fn run_benchmark(args: BenchmarkArgs) {
    if args.torture {
        std::process::exit(threadmoth::torture::run(args.json));
    }
    let profile = if args.quick {
        BenchmarkProfile::Quick
    } else if args.tough {
        BenchmarkProfile::Tough
    } else {
        args.profile.unwrap_or(BenchmarkProfile::Standard)
    };
    let profile = match profile {
        BenchmarkProfile::Quick => threadmoth::benchmark::Profile::Quick,
        BenchmarkProfile::Standard => threadmoth::benchmark::Profile::Standard,
        BenchmarkProfile::Tough => threadmoth::benchmark::Profile::Tough,
    };
    std::process::exit(threadmoth::benchmark::run(profile, args.json));
}

fn run_help(args: HelpArgs) {
    if let Some(term) = args.find {
        let matches = threadmoth::metadata::find_help(&term);
        if matches.is_empty() {
            eprintln!("no help matched '{term}'");
            std::process::exit(1);
        }
        for (name, description) in matches {
            println!("{name}: {description}");
        }
        return;
    }

    let mut root = Cli::command();
    if let Some(command) = args.command {
        if let Some((provider, operation)) = command.split_once(':') {
            let output = threadmoth::metadata::operation_help(provider, operation);
            match output {
                Some(value) if args.json => {
                    println!("{}", serde_json::to_string_pretty(&value).unwrap())
                }
                Some(value) => println!("{}", threadmoth::metadata::render_operation_help(&value)),
                None => {
                    eprintln!("unknown provider operation: {command}");
                    std::process::exit(1);
                }
            }
            return;
        }
        if let Some(value) = threadmoth::metadata::provider_help(&command) {
            if args.json {
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                println!("{}", threadmoth::metadata::render_provider_help(&value));
            }
            return;
        }
        if let Some(subcommand) = root.find_subcommand_mut(&command) {
            if args.json {
                let value = serde_json::json!({"command": command, "help": subcommand.render_long_help().to_string()});
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
                return;
            }
            subcommand.print_long_help().unwrap();
            println!();
            return;
        }
        if let Some(text) = threadmoth::metadata::command_help(&command) {
            println!("threadmoth help {command}\n\n{text}");
            return;
        }
        eprintln!("unknown command: {command}");
        std::process::exit(1);
    }
    root.print_long_help().unwrap();
    println!();
}

fn run_orient(json_output: bool) {
    let workspace = env::current_dir()
        .map(|path| path.canonicalize().unwrap_or(path))
        .unwrap_or_else(|_| ".".into());
    let value = threadmoth::metadata::orientation(&workspace.to_string_lossy());
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!(
            "Threadmoth {} · workspace {}",
            THREADMOTH_VERSION,
            workspace.display()
        );
        println!("Workflow: preview → mutate → verify certificate");
        println!("Quick edit: threadmoth edit PATH --search OLD --replace NEW (verifies and applies in one call)");
        println!("Multi-file: threadmoth edit changes.tm (or stdin: threadmoth edit -; use --plan to preview diff)");
        println!("Text edits: prepare-text PATH --edit OLD NEW --output PLAN --summary → review with explain --plan PLAN --format diff → apply-plan --plan PLAN");
        println!("Multiline intent: use --edit-files OLD_FILE NEW_FILE for faithful UTF-8 input.");
        println!("Multi-file intent: prepare-edits CHANGES --output PLAN --review; exact SEARCH/REPLACE blocks remain guarded.");
        println!("Discover: capabilities · suggest PATH · help TARGET");
        println!("Confinement: workspace and effect budget; refusals do not write");
        println!("Exit codes: success 0 · refusal 2 · runtime failure 3");
    }
}

fn print_explain(code: &str, json_output: bool) {
    let Some(reason) = threadmoth::metadata::reason(code) else {
        eprintln!("unknown reason code: {code}");
        std::process::exit(1);
    };
    if json_output {
        println!("{}", serde_json::to_string_pretty(&reason).unwrap());
    } else {
        println!(
            "{} — {}\nWhy: {}\nRecovery: {}\nRetry unchanged: {}\nCommands: {}",
            reason.code,
            reason.meaning,
            reason.why_refused,
            reason.recovery_category,
            reason.retry_unchanged,
            reason.relevant_commands.join(", ")
        );
    }
}

fn run_suggest(args: SuggestArgs) {
    if let Some(source) = args.from_refusal {
        let input = if source == "-" {
            read_json_argument(None)
        } else {
            match read_request_input(Some(Path::new(&source))) {
                Ok(input) => input,
                Err(RequestInputError::TooLarge(actual)) => {
                    eprintln!(
                        "refusal certificate exceeds {MAX_REQUEST_BYTES} bytes (actual: {actual})"
                    );
                    std::process::exit(2);
                }
                Err(RequestInputError::Io(error)) => {
                    eprintln!("refusal certificate read failed: {error}");
                    std::process::exit(3);
                }
            }
        };
        let certificate: Certificate = serde_json::from_str(&input).unwrap_or_else(|error| {
            eprintln!("invalid refusal certificate: {error}");
            std::process::exit(2);
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&threadmoth::metadata::refusal_recovery(&certificate))
                .unwrap()
        );
        return;
    }

    let path = args.path.expect("clap requires path unless --from-refusal");
    let path = path.to_string_lossy();
    let root = env::current_dir().unwrap_or_else(|_| ".".into());
    let ws = Workspace::new(root).unwrap_or_else(|e| {
        eprintln!("workspace initialization failed: {e}");
        std::process::exit(3)
    });
    let bytes = ws.read_file(path.as_ref()).ok();
    let suggestion = threadmoth::metadata::suggest(
        path.as_ref(),
        args.goal.as_deref(),
        args.at.as_deref(),
        &args.mode,
        bytes.as_deref(),
    );
    println!("{}", serde_json::to_string_pretty(&suggestion).unwrap());
}

fn run_inspect(args: InspectArgs) {
    let _ = args.json;
    let path = args.path.to_string_lossy();
    let root = env::current_dir().unwrap_or_else(|_| ".".into());
    let ws = match Workspace::new(root) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("workspace initialization failed: {e}");
            std::process::exit(3)
        }
    };
    let view = if args.expand.is_some() {
        Some("expand")
    } else if args.outline {
        Some("outline")
    } else {
        None
    };
    match threadmoth::metadata::inspect_view(
        &ws,
        path.as_ref(),
        view,
        args.expand.as_deref(),
        Some(args.max_bytes),
        Some(args.max_entries),
        None,
    ) {
        Ok(output) => println!("{}", serde_json::to_string_pretty(&output).unwrap()),
        Err(error) => {
            eprintln!("inspect failed: {error}");
            std::process::exit(2);
        }
    }
}

fn run_schema(args: SchemaArgs) {
    if let Some(scope) = args.scope.as_deref() {
        if !threadmoth::metadata::is_schema_scope(scope) {
            eprintln!(
                "unknown schema scope '{scope}'; choose all, request, response, certificate, transaction_request, transaction_certificate, or metadata"
            );
            std::process::exit(2);
        }
    }
    let out = threadmoth::metadata::schema(args.scope.as_deref());
    if args.json && !args.pretty {
        println!("{}", serde_json::to_string(&out).unwrap());
    } else {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    }
}

fn run_doctor(args: DoctorArgs) {
    let root = env::current_dir().unwrap_or_else(|_| ".".into());
    let workspace = if Workspace::new(root).is_ok() {
        "ready"
    } else {
        "unavailable"
    };
    let providers = threadmoth::metadata::provider_metadata()
        .into_iter()
        .map(|provider| provider.name)
        .collect::<Vec<_>>()
        .join(" ");
    let shell = detected_shell();
    let path_status = current_exe_on_path();
    let installation = threadmoth::updater::installation_kind();
    let executable = env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let self_update = if installation.is_standalone() {
        "available (explicit: threadmoth update)"
    } else {
        "disabled (package-managed or ambiguous installation)"
    };
    if args.json {
        let recovery = Workspace::new(env::current_dir().unwrap_or_else(|_| ".".into()))
            .ok()
            .and_then(|workspace| {
                serde_json::to_value(threadmoth::recovery::list(&workspace)).ok()
            });
        println!(
            "{}",
            serde_json::json!({
                "version": THREADMOTH_VERSION,
                "protocol": PROTOCOL_VERSION,
                "platform": {"os": env::consts::OS, "arch": env::consts::ARCH},
                "installation_kind": installation.label(),
                "executable": executable,
                "workspace_readiness": workspace,
                "recovery_journals": recovery,
                "self_update_eligible": installation.is_standalone(),
                "path_configured": path_status,
                "shell": shell,
                "build": threadmoth::build_info::current(),
            })
        );
        return;
    }
    println!(
        "threadmoth doctor\nversion: {THREADMOTH_VERSION}\nos: {}\narch: {}\nworkspace: {workspace}\nprotocol: {PROTOCOL_VERSION}\nproviders: {providers}\nbuild: {} / {} / {}\ntransport: stdin/stdout mcp/stdio\ncommit: staged atomic replacement; recovery journal available\ninstallation: {}\nexecutable: {executable}\nself-update: {self_update}\nshell: {shell}\npath: {}\ncompletion: available (threadmoth completions {shell})\nmanpage: available (threadmoth manpage)",
        env::consts::OS,
        env::consts::ARCH,
        threadmoth::build_info::current().build_flavor,
        threadmoth::build_info::current().cpu_baseline,
        threadmoth::build_info::current().optimization_profile,
        installation.label(),
        if path_status {
            "configured"
        } else {
            "current executable directory not detected on PATH"
        },
    );
}

fn detected_shell() -> &'static str {
    if env::var_os("PSModulePath").is_some() && env::consts::OS == "windows" {
        return "powershell";
    }
    let shell = env::var("SHELL").unwrap_or_default().to_ascii_lowercase();
    if shell.contains("zsh") {
        "zsh"
    } else if shell.contains("fish") {
        "fish"
    } else {
        "bash"
    }
}

fn current_exe_on_path() -> bool {
    let Ok(exe) = env::current_exe() else {
        return false;
    };
    let Some(parent) = exe.parent() else {
        return false;
    };
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|entry| entry == parent))
        .unwrap_or(false)
}

fn run_completions(shell: CompletionShell) {
    let mut command = Cli::command();
    generate::<clap_complete::Shell, _>(
        shell.into(),
        &mut command,
        "threadmoth",
        &mut io::stdout(),
    );
}

fn run_manpage(output: Option<&Path>) {
    let man = clap_mangen::Man::new(Cli::command());
    match output {
        Some(path) => {
            let mut file = fs::File::create(path).unwrap_or_else(|e| {
                eprintln!("cannot create manpage {}: {e}", path.display());
                std::process::exit(3)
            });
            man.render(&mut file).unwrap_or_else(|e| {
                eprintln!("cannot render manpage: {e}");
                std::process::exit(3)
            });
        }
        None => {
            let mut stdout = io::stdout();
            man.render(&mut stdout).unwrap_or_else(|e| {
                eprintln!("cannot render manpage: {e}");
                std::process::exit(3)
            });
            stdout.flush().ok();
        }
    }
}

fn read_json_argument(path: Option<&Path>) -> String {
    match read_request_input(path) {
        Ok(value) => value,
        Err(RequestInputError::TooLarge(actual)) => {
            eprintln!("input exceeds {MAX_REQUEST_BYTES} bytes (actual: {actual})");
            std::process::exit(2);
        }
        Err(RequestInputError::Io(error)) => {
            eprintln!("request read failed: {error}");
            std::process::exit(3);
        }
    }
}

enum RequestInputError {
    Io(io::Error),
    TooLarge(usize),
}

fn read_request_input(path: Option<&Path>) -> Result<String, RequestInputError> {
    let mut bytes = Vec::new();
    let reader: Box<dyn Read> = match path {
        Some(path) => Box::new(fs::File::open(path).map_err(RequestInputError::Io)?),
        None => Box::new(io::stdin()),
    };
    reader
        .take((MAX_REQUEST_BYTES as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(RequestInputError::Io)?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(RequestInputError::TooLarge(bytes.len()));
    }
    String::from_utf8(bytes)
        .map_err(|error| RequestInputError::Io(io::Error::new(io::ErrorKind::InvalidData, error)))
}

fn empty_cert(reason: RefusalReason) -> Certificate {
    let reason_code = reason.code().into();
    Certificate {
        protocol_version: PROTOCOL_VERSION.into(),
        request_id: String::new(),
        outcome: Outcome::Refused,
        file_path: String::new(),
        provider: "request".into(),
        provider_version: "parser".into(),
        expected_cardinality: Default::default(),
        observed_cardinality: None,
        pre_hash: String::new(),
        post_hash: None,
        changed_ranges: Vec::new(),
        changed_line_ranges: Vec::new(),
        diff_summary: None,
        diff_truncated: false,
        structural_validation: StructuralValidation::NotApplicable,
        preservation: PreservationFacts::default(),
        commit: CommitGuarantee::default(),
        refusal_reason: Some(reason),
        failure_reason: None,
        reason_code: Some(reason_code),
        recovery: None,
        schema_diagnostic: None,
        diagnostics: Vec::new(),
        budget: EffectBudget::default(),
        effect: EffectUsage {
            files: 0,
            matches: 0,
            changed_regions: 0,
            changed_lines: 0,
            changed_bytes: 0,
            passed: true,
        },
        transaction_guarantee: "not_committed".into(),
        recovery_state: "not_required".into(),
        desired_state: None,
    }
}

fn empty_transaction_certificate(reason: RefusalReason) -> TransactionCertificate {
    TransactionCertificate {
        protocol_version: PROTOCOL_VERSION.into(),
        transaction_id: String::new(),
        outcome: Outcome::Refused,
        certificates: Vec::new(),
        rollback_state: "not_started".into(),
        transaction_guarantee: "not_committed".into(),
        refusal_reason: Some(reason.clone()),
        failure_reason: None,
        reason_code: Some(reason.code().into()),
        recovery: None,
        schema_diagnostic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_summary_reports_all_undersized_dimensions() {
        let mut certificate = empty_cert(RefusalReason::EffectBudgetExceeded {
            dimension: "max_changed_regions".into(),
            limit: 1,
            actual: 3,
        });
        certificate.budget.max_changed_regions = Some(1);
        certificate.budget.max_changed_lines = Some(2);
        certificate.effect.changed_regions = 3;
        certificate.effect.changed_lines = 7;
        certificate.effect.passed = false;
        assert_eq!(
            budget_suggestions(&certificate),
            vec!["max_changed_regions=3", "max_changed_lines=7"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_argument_uses_literal_single_quoted_syntax() {
        let value = r#"path "double" 'single' $env:TEMP $(Get-Date) `tick`"#;
        assert_eq!(
            quote_powershell_argument(value),
            r#"'path "double" ''single'' $env:TEMP $(Get-Date) `tick`'"#
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_command_parser_round_trips_quotes_and_interpolation_text() {
        let value = r#"C:\Temp\plan "double" 'single' $env:TEMP $(throw 'executed') `tick`"#;
        let script = format!(
            "$value = {}; [Console]::Out.Write($value)",
            quote_powershell_argument(value)
        );
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(script)
            .output()
            .expect("PowerShell is available on Windows");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, value.as_bytes());
    }

    #[test]
    fn outcome_names_match_protocol_vocabulary() {
        assert_eq!(outcome_name(&Outcome::Applied), "APPLIED");
        assert_eq!(outcome_name(&Outcome::NoChange), "NO_CHANGE");
        assert_eq!(outcome_name(&Outcome::Refused), "REFUSED");
        assert_eq!(outcome_name(&Outcome::Failed), "FAILED");
    }
}
