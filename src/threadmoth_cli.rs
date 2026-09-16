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
    CompletionShell, CreateFileArgs, DoctorArgs, ExplainFormat, HelpArgs, InspectArgs, PlanArgs,
    RecoverArgs, ReplaceExactArgs, SchemaArgs, SetStringArgs, SetValueArgs, SuggestArgs,
    UpdateArgs, THREADMOTH_VERSION,
};

fn main() {
    let cli = Cli::parse();
    match cli.command {
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
        Command::TransactionPreview(args) => {
            run_transaction(args.request.as_deref(), true, args.summary)
        }
        Command::Recover(args) => run_recover(args),
        Command::Capabilities(args) => run_capabilities(args),
        Command::Examples { topic } => print_examples(topic.as_deref()),
        Command::Benchmark(args) => run_benchmark(args),
        Command::Torture { json } => std::process::exit(threadmoth::torture::run(json)),
        Command::Help(args) => run_help(args),
        Command::Explain {
            code,
            plan,
            json,
            format,
        } => {
            if let Some(plan) = plan {
                run_explain_plan(&plan, json, format);
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

fn run_explain_plan(path: &Path, json: bool, format: Option<ExplainFormat>) {
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
    let safe = refusal.is_none();
    if !json {
        if let Some(format) = format {
            render_plan_review(&plan, workspace.as_ref(), format);
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
    }
}

fn render_plan_review(plan: &PreparedPlan, workspace: Option<&Workspace>, format: ExplainFormat) {
    let title = match format {
        ExplainFormat::Diff => "Threadmoth plan diff",
        ExplainFormat::Markdown => "# Threadmoth plan review",
    };
    println!(
        "{title}\nPlan: {}\nProtocol: {}",
        plan.plan_id, plan.protocol_version
    );
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
                bounded_review(&diff),
            ),
            ExplainFormat::Markdown => println!(
                "\n## {}\n\n- Provider: `{}`\n- State: **{}**\n- Pre-image: `{}`\n- Prospective: `{}`\n- Edits: `{}`\n\n```diff\n{}\n```",
                operation.file_path,
                operation.provider,
                state,
                operation.pre_hash,
                operation.prospective_hash,
                operation.edits.len(),
                bounded_review(&diff),
            ),
        }
    }
}

fn bounded_review(value: &str) -> String {
    let mut result = value.chars().take(4096).collect::<String>();
    if value.chars().count() > 4096 {
        result.push_str("\n... review output truncated ...");
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
        if let Some(subcommand) = root.find_subcommand_mut(&command) {
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

    #[test]
    fn outcome_names_match_protocol_vocabulary() {
        assert_eq!(outcome_name(&Outcome::Applied), "APPLIED");
        assert_eq!(outcome_name(&Outcome::NoChange), "NO_CHANGE");
        assert_eq!(outcome_name(&Outcome::Refused), "REFUSED");
        assert_eq!(outcome_name(&Outcome::Failed), "FAILED");
    }
}
