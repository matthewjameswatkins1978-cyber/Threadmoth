use schemars::schema_for;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    env,
    io::{self, BufRead},
};
use threadmoth::{
    pipeline::execute_request,
    protocol::{
        Assertion, Cardinality, EffectBudget, OperationPayload, PreparedPlan, Request,
        TransactionRequest, MAX_REQUEST_BYTES, PROTOCOL_VERSION,
    },
    workspace::Workspace,
};

use crate::cli::THREADMOTH_VERSION;

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct InspectToolArgs {
    #[serde(rename = "file_path", alias = "path")]
    file_path: String,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    max_bytes: Option<usize>,
    #[serde(default)]
    max_entries: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SuggestToolArgs {
    #[serde(rename = "file_path", alias = "path")]
    file_path: String,
    #[serde(default)]
    goal: Option<String>,
    #[serde(default)]
    #[serde(rename = "selector", alias = "at")]
    selector: Option<String>,
    #[serde(default = "default_suggestion_mode")]
    mode: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExplainToolArgs {
    code: String,
}

#[derive(Deserialize, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields)]
struct CapabilitiesToolArgs {
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    #[serde(rename = "file_path", alias = "for_path")]
    file_path: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExactReplaceToolArgs {
    #[serde(rename = "file_path", alias = "file")]
    file_path: String,
    old: String,
    new: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PrepareTextToolArgs {
    file_path: String,
    edits: Vec<threadmoth::source_prepare::TextReplacementIntent>,
    #[serde(default)]
    newline_policy: threadmoth::source_prepare::NewlinePolicy,
    #[serde(default)]
    #[serde(rename = "within", alias = "scope")]
    within: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetValueToolArgs {
    #[serde(rename = "file_path", alias = "file")]
    file_path: String,
    #[serde(rename = "selector", alias = "path")]
    selector: String,
    value: Value,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct OrientToolArgs {}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct StructuralToolArgs {
    pattern: String,
    rewrite: String,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    max_files: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConvertNewlinesToolArgs {
    #[serde(rename = "file_path", alias = "path", alias = "file")]
    file_path: String,
    to: String,
    #[serde(default)]
    preview: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditToolArgs {
    #[serde(default)]
    #[serde(rename = "file_path", alias = "path", alias = "target", alias = "file")]
    file_path: Option<String>,
    #[serde(default)]
    #[serde(rename = "search", alias = "old")]
    search: Option<String>,
    #[serde(default)]
    #[serde(rename = "search_file", alias = "old_file")]
    search_file: Option<String>,
    #[serde(default)]
    #[serde(rename = "replace", alias = "new")]
    replace: Option<String>,
    #[serde(default)]
    #[serde(rename = "replace_file", alias = "new_file")]
    replace_file: Option<String>,
    #[serde(default)]
    #[serde(rename = "within", alias = "scope", alias = "in")]
    within: Option<String>,
    #[serde(default)]
    #[serde(rename = "expect_count")]
    expect_count: Option<usize>,
    #[serde(default)]
    #[serde(rename = "all")]
    all: bool,
    #[serde(default)]
    #[serde(rename = "newline_policy")]
    newline_policy: Option<String>,
    #[serde(default)]
    #[serde(rename = "document", alias = "content")]
    document: Option<String>,
    #[serde(default)]
    #[serde(rename = "allow_newline_conversion")]
    allow_newline_conversion: bool,
    #[serde(default)]
    plan: bool,
}

fn default_suggestion_mode() -> String {
    "safe".into()
}

fn shorthand_request(file: &str, operation: OperationPayload, bytes: usize) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: format!("mcp-shorthand-{file}"),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: file.replace('\\', "/"),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
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

fn schema_error(error: serde_json::Error) -> String {
    serde_json::to_string(&threadmoth::protocol::schema_diagnostic(&error.to_string()))
        .unwrap_or_else(|_| error.to_string())
}

pub fn run_mcp() {
    let workspace = match env::current_dir() {
        Ok(path) => match Workspace::new(path) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("workspace initialization failed: {e}");
                return;
            }
        },
        Err(e) => {
            eprintln!("workspace initialization failed: {e}");
            return;
        }
    };
    let mut observation_cache = threadmoth::metadata::ObservationCache::default();
    let mut input = io::stdin().lock();
    loop {
        let line = match read_mcp_line(&mut input) {
            Ok(Some(Ok(line))) => line,
            Ok(Some(Err(actual))) => {
                println!(
                    "{}",
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": serde_json::Value::Null,
                        "error": {
                            "code": -32600,
                            "message": format!("request exceeds {MAX_REQUEST_BYTES} bytes (actual: {actual})")
                        }
                    })
                );
                continue;
            }
            Ok(None) => break,
            Err(error) => {
                eprintln!("MCP input read failed: {error}");
                break;
            }
        };
        let request: Value = match serde_json::from_str(&line) {
            Ok(x) => x,
            Err(error) => {
                println!("{}", json_rpc_error(Value::Null, -32700, error.to_string()));
                continue;
            }
        };
        if let Some(response) = handle_mcp_message(&workspace, request, &mut observation_cache) {
            println!("{}", response);
        }
    }
}

fn handle_mcp_message(
    workspace: &Workspace,
    request: Value,
    observation_cache: &mut threadmoth::metadata::ObservationCache,
) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(json_rpc_error(
            Value::Null,
            -32600,
            "invalid JSON-RPC request".into(),
        ));
    };
    let notification = !object.contains_key("id");
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let body = if object.get("jsonrpc") != Some(&Value::String("2.0".into())) {
        json_rpc_error(id.clone(), -32600, "invalid JSON-RPC request".into())
    } else {
        match object.get("method").and_then(Value::as_str) {
            Some("initialize") => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "threadmoth", "version": THREADMOTH_VERSION, "protocol_version": PROTOCOL_VERSION}}
            }),
            Some("tools/list") => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"tools": [
                    {"name": "threadmoth_edit", "description": "Apply or preview an exact verified edit in one invocation. For a single file, provide file_path, search (or search_file) and replace (or replace_file). Set plan: true to preview without mutating.", "inputSchema": schema_for!(EditToolArgs)},
                    {"name": "threadmoth_convert_newlines", "description": "Convert line endings across a workspace file with explicit conversion authority", "inputSchema": schema_for!(ConvertNewlinesToolArgs)},
                    {"name": "threadmoth_mutate", "description": "Apply one typed Threadmoth mutation and return its certificate; target specificity must come from the user or evidence", "inputSchema": schema_for!(Request)},
                    {"name": "threadmoth_preview", "description": "Preview one typed Threadmoth mutation without writing; if approved, call threadmoth_mutate with the same request; do not invent target specificity", "inputSchema": schema_for!(Request)},
                    {"name": "threadmoth_plan", "description": "Prepare a deterministic guarded plan without writing; preserve unresolved target ambiguity", "inputSchema": schema_for!(Request)},
                    {"name": "threadmoth_apply_plan", "description": "Apply an exact reviewed prepared plan after rechecking source identity, authority, budgets and assertions; return the mutation certificate", "inputSchema": schema_for!(PreparedPlan)},
                    {"name": "threadmoth_prepare_text", "description": "Prepare exact text edits from one observed source state; derives uniform LF/CRLF, refuses ambiguity and never writes. Preview plan.operations[0].request with threadmoth_preview, then pass the unchanged returned plan to threadmoth_apply_plan after review.", "inputSchema": schema_for!(PrepareTextToolArgs)},
                    {"name": "threadmoth_inspect", "description": "Read unchanged identity facts or a bounded deterministic outline/expansion; observation handles are stale-safe read identities and never mutation authority", "inputSchema": schema_for!(InspectToolArgs)},
                    {"name": "threadmoth_suggest", "description": "Return deterministic request suggestions and candidate evidence; never choose among unresolved candidates", "inputSchema": schema_for!(SuggestToolArgs)},
                    {"name": "threadmoth_explain", "description": "Return stable metadata for a refusal or failure reason", "inputSchema": schema_for!(ExplainToolArgs)},
                    {"name": "threadmoth_capabilities", "description": "Return Threadmoth capabilities, optionally scoped to a provider or path", "inputSchema": schema_for!(CapabilitiesToolArgs)},
                    {"name": "threadmoth_transact_preview", "description": "Preview a guarded transaction without writing", "inputSchema": schema_for!(TransactionRequest)},
                    {"name": "threadmoth_transact", "description": "Prepare and commit a guarded transaction", "inputSchema": schema_for!(TransactionRequest)},
                    {"name": "threadmoth_exact_replace", "description": "Safely replace one exact text occurrence through the canonical pipeline after the occurrence is uniquely established", "inputSchema": schema_for!(ExactReplaceToolArgs)},
                    {"name": "threadmoth_set_value", "description": "Safely set one selected JSON, JSONC, TOML, YAML, INI or dotenv value through the canonical registry and Core pipeline; do not guess among plausible targets", "inputSchema": schema_for!(SetValueToolArgs)},
                    {"name": "threadmoth_orient", "description": "Return workspace-bound discovery, the compact prepare-text → review → apply-plan CLI path, MCP preparation/application tools and exit-code map; does not select or change a workspace", "inputSchema": schema_for!(OrientToolArgs)},
                    {"name": "threadmoth_structural", "description": "Perform structural AST pattern search and rewrite across workspace files using the ast-grep toolbelt attachment, guarded by Threadmoth's transaction and rollback engine", "inputSchema": schema_for!(StructuralToolArgs)}
                ]}
            }),
            Some("tools/call") => {
                let Some(params) = object.get("params").and_then(Value::as_object) else {
                    return if notification {
                        None
                    } else {
                        Some(json_rpc_error(
                            id,
                            -32602,
                            "tools/call params must be an object".into(),
                        ))
                    };
                };
                let Some(name) = params.get("name").and_then(Value::as_str) else {
                    return if notification {
                        None
                    } else {
                        Some(json_rpc_error(
                            id,
                            -32602,
                            "tools/call requires a tool name".into(),
                        ))
                    };
                };
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let call_arguments = arguments.clone();
                let value = call_tool(workspace, name, arguments, observation_cache);
                let result = match value {
                    Ok(mut value) => {
                        if value["outcome"] == "REFUSED" {
                            attach_refusal_envelope(workspace, &mut value);
                        }
                        json!({"content": [{"type": "text", "text": serde_json::to_string(&value).unwrap()}], "structuredContent": value})
                    }
                    Err(error) => {
                        if error
                            .contains("requested path is outside the configured workspace root:")
                        {
                            let refusal =
                                workspace_root_refusal(workspace, &call_arguments, &error);
                            json!({"isError": true, "content": [{"type": "text", "text": serde_json::to_string(&refusal).unwrap()}], "structuredContent": refusal})
                        } else {
                            json!({"isError": true, "content": [{"type": "text", "text": error}]})
                        }
                    }
                };
                json!({"jsonrpc": "2.0", "id": id, "result": result})
            }
            Some(_) => json_rpc_error(id, -32601, "method not found".into()),
            None => json_rpc_error(id, -32600, "method is required".into()),
        }
    };
    if notification {
        None
    } else {
        Some(body)
    }
}

fn attach_refusal_envelope(workspace: &Workspace, certificate: &mut Value) {
    let code = certificate["reason_code"].as_str().unwrap_or("REFUSED");
    let reason = threadmoth::metadata::reason(code);
    let workspace_root = workspace.root().display().to_string();
    let requested_path = certificate["file_path"].as_str().unwrap_or("");
    let recovery = if code == "WORKSPACE_ROOT_MISMATCH" {
        json!([
            {"action": "use_checkout_local_threadmoth", "description": "Run Threadmoth from the intended authorized checkout, then preview again."},
            {"action": "inspect_workspace_configuration", "description": "Check the host-configured workspace root; a request cannot rebind it."}
        ])
    } else {
        certificate["recovery"]["remedies"]
            .as_array()
            .map(|remedies| {
                remedies
                    .iter()
                    .map(|remedy| {
                        json!({
                            "action": remedy["kind"],
                            "description": remedy["description"]
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into()
    };
    certificate["refusal"] = json!({
        "status": "refused",
        "code": code,
        "message": reason.as_ref().map(|item| item.meaning).unwrap_or("The request was refused by a safety guard."),
        "write_performed": false,
        "context": {"workspace_root": workspace_root, "requested_path": requested_path, "provider": certificate["provider"]},
        "recovery": recovery,
        "retryability": reason.as_ref().map(|item| if item.retry_unchanged { "safe_to_retry_unchanged" } else { "correct_request_before_retry" }).unwrap_or("correct_request_before_retry")
    });
}

fn workspace_root_refusal(workspace: &Workspace, arguments: &Value, message: &str) -> Value {
    let requested_path = ["file_path", "path", "file", "for_path"]
        .iter()
        .find_map(|field| arguments.get(*field).and_then(Value::as_str))
        .unwrap_or("");
    json!({
        "status": "refused",
        "code": "WORKSPACE_ROOT_MISMATCH",
        "message": "The requested path is outside the workspace root bound to this Threadmoth process.",
        "write_performed": false,
        "workspace_root": workspace.root().display().to_string(),
        "requested_path": requested_path,
        "reason": message.chars().take(512).collect::<String>(),
        "context": {"workspace_root": workspace.root().display().to_string(), "requested_path": requested_path},
        "recovery": [
            {"action": "use_checkout_local_threadmoth", "description": "Run Threadmoth from the intended authorized checkout, then preview again."},
            {"action": "inspect_workspace_configuration", "description": "Check the host-configured workspace root; a request cannot rebind it."}
        ],
        "retryability": "retry_after_caller_selects_and_starts_the_authorized_workspace"
    })
}

fn call_tool(
    workspace: &Workspace,
    name: &str,
    arguments: Value,
    observation_cache: &mut threadmoth::metadata::ObservationCache,
) -> Result<Value, String> {
    match name {
        "threadmoth_convert_newlines" => {
            let args: ConvertNewlinesToolArgs =
                serde_json::from_value(arguments).map_err(schema_error)?;
            let mut req = shorthand_request(
                &args.file_path,
                OperationPayload::Text(
                    threadmoth::provider::text::TextOperation::ConvertNewlines { to: args.to },
                ),
                1024 * 1024,
            );
            req.allow_newline_conversion = true;
            let cert = threadmoth::pipeline::execute_request(workspace, &req, args.preview);
            serde_json::to_value(&cert).map_err(|e| e.to_string())
        }
        "threadmoth_edit" => {
            let args = serde_json::from_value::<EditToolArgs>(arguments).map_err(schema_error)?;
            execute_mcp_edit(workspace, args)
        }
        "threadmoth_exact_replace" => {
            let args =
                serde_json::from_value::<ExactReplaceToolArgs>(arguments).map_err(schema_error)?;
            let request = shorthand_request(
                &args.file_path,
                OperationPayload::Text(threadmoth::provider::text::TextOperation::Replace {
                    target: args.old.clone(),
                    replacement: args.new.clone(),
                }),
                args.old.len().max(args.new.len()),
            );
            serde_json::to_value(execute_request(workspace, &request, false))
                .map_err(|error| error.to_string())
        }
        "threadmoth_prepare_text" => {
            let args =
                serde_json::from_value::<PrepareTextToolArgs>(arguments).map_err(schema_error)?;
            let request = threadmoth::source_prepare::SourceAwareTextRequest {
                file_path: args.file_path,
                edits: args.edits,
                newline_policy: args.newline_policy,
                within: args.within,
                cardinality: Cardinality::default(),
                allow_newline_conversion: false,
            };
            let result = threadmoth::source_prepare::prepare(workspace, &request);
            let output = match result {
                threadmoth::source_prepare::SourcePreparationResult::Prepared(prepared) => {
                    serde_json::to_value(prepared)
                }
                threadmoth::source_prepare::SourcePreparationResult::Refused { certificate } => {
                    serde_json::to_value(certificate)
                }
            };
            output.map_err(|error| error.to_string())
        }
        "threadmoth_set_value" => {
            let args =
                serde_json::from_value::<SetValueToolArgs>(arguments).map_err(schema_error)?;
            let bytes = serde_json::to_vec(&args.value)
                .map_err(|error| error.to_string())?
                .len();
            let operation = threadmoth::shorthand::set_value_operation(
                &args.file_path,
                &args.selector,
                args.value,
            )?;
            let request = shorthand_request(&args.file_path, operation, bytes);
            serde_json::to_value(execute_request(workspace, &request, false))
                .map_err(|error| error.to_string())
        }
        "threadmoth_capabilities" | "suture_capabilities" => {
            let args =
                serde_json::from_value::<CapabilitiesToolArgs>(arguments).map_err(schema_error)?;
            let output = if let Some(path) = args.file_path {
                let bytes = workspace.read_file(&path).ok();
                threadmoth::metadata::capabilities_for(&path, bytes.as_deref())
            } else {
                threadmoth::metadata::capability_view(args.selector.as_deref())
            };
            Ok(output)
        }
        "threadmoth_inspect" => {
            let args =
                serde_json::from_value::<InspectToolArgs>(arguments).map_err(schema_error)?;
            threadmoth::metadata::inspect_view(
                workspace,
                &args.file_path,
                args.view.as_deref(),
                args.handle.as_deref(),
                args.max_bytes,
                args.max_entries,
                Some(observation_cache),
            )
        }
        "threadmoth_suggest" => {
            let args =
                serde_json::from_value::<SuggestToolArgs>(arguments).map_err(schema_error)?;
            let bytes = workspace.read_file(&args.file_path).ok();
            serde_json::to_value(threadmoth::metadata::suggest(
                &args.file_path,
                args.goal.as_deref(),
                args.selector.as_deref(),
                &args.mode,
                bytes.as_deref(),
            ))
            .map_err(|e| e.to_string())
        }
        "threadmoth_explain" => {
            let args =
                serde_json::from_value::<ExplainToolArgs>(arguments).map_err(schema_error)?;
            let reason = threadmoth::metadata::reason(&args.code)
                .ok_or_else(|| format!("unknown reason code: {}", args.code))?;
            serde_json::to_value(reason).map_err(|e| e.to_string())
        }
        "threadmoth_orient" => {
            let _ = serde_json::from_value::<OrientToolArgs>(arguments).map_err(schema_error)?;
            Ok(threadmoth::metadata::orientation(
                &workspace.root().display().to_string(),
            ))
        }
        "threadmoth_mutate" | "suture_mutate" => {
            let request = serde_json::from_value::<Request>(arguments).map_err(schema_error)?;
            Ok(
                serde_json::to_value(execute_request(workspace, &request, false))
                    .map_err(|e| e.to_string())?,
            )
        }
        "threadmoth_preview" | "suture_preview" => {
            let request = serde_json::from_value::<Request>(arguments).map_err(schema_error)?;
            Ok(
                serde_json::to_value(execute_request(workspace, &request, true))
                    .map_err(|e| e.to_string())?,
            )
        }
        "threadmoth_plan" => {
            let mut value = arguments;
            let assertions = value
                .as_object_mut()
                .and_then(|object| object.remove("assertions"))
                .map(|value| serde_json::from_value::<Vec<Assertion>>(value).map_err(schema_error))
                .transpose()?
                .unwrap_or_default();
            if value.get("transaction_id").is_some() {
                let transaction =
                    serde_json::from_value::<TransactionRequest>(value).map_err(schema_error)?;
                let plan = threadmoth::pipeline::prepare_transaction_plan(
                    workspace,
                    &transaction,
                    assertions,
                )
                .map_err(|certificate| serde_json::to_string(&certificate).unwrap())?;
                serde_json::to_value(plan).map_err(|e| e.to_string())
            } else {
                let request = serde_json::from_value::<Request>(value).map_err(schema_error)?;
                let plan =
                    threadmoth::pipeline::prepare_request_plan(workspace, &request, assertions)
                        .map_err(|certificate| serde_json::to_string(&certificate).unwrap())?;
                serde_json::to_value(plan).map_err(|e| e.to_string())
            }
        }
        "threadmoth_apply_plan" => {
            let plan = serde_json::from_value::<PreparedPlan>(arguments).map_err(schema_error)?;
            serde_json::to_value(threadmoth::pipeline::apply_prepared_plan(workspace, &plan))
                .map_err(|e| e.to_string())
        }
        "threadmoth_transact" | "suture_transact" => {
            let transaction =
                serde_json::from_value::<TransactionRequest>(arguments).map_err(schema_error)?;
            Ok(
                serde_json::to_value(threadmoth::pipeline::execute_transaction(
                    workspace,
                    &transaction,
                    false,
                ))
                .map_err(|e| e.to_string())?,
            )
        }
        "threadmoth_transact_preview" => {
            let transaction =
                serde_json::from_value::<TransactionRequest>(arguments).map_err(schema_error)?;
            Ok(
                serde_json::to_value(threadmoth::pipeline::execute_transaction(
                    workspace,
                    &transaction,
                    true,
                ))
                .map_err(|e| e.to_string())?,
            )
        }
        "threadmoth_structural" => {
            let args: StructuralToolArgs = serde_json::from_value(arguments)
                .map_err(|e| format!("Invalid structural arguments: {e}"))?;
            let req = threadmoth::toolbelt::structural::StructuralRequest {
                pattern: args.pattern,
                rewrite: args.rewrite,
                lang: args.lang,
                paths: args.paths,
                max_files: args.max_files,
            };
            match threadmoth::toolbelt::structural::execute_structural(
                workspace,
                &req,
                None,
                args.dry_run,
            ) {
                Ok(threadmoth::toolbelt::structural::StructuralOutcome::NoChange { message }) => {
                    Ok(serde_json::json!({
                        "outcome": "no_change",
                        "message": message,
                    }))
                }
                Ok(threadmoth::toolbelt::structural::StructuralOutcome::Preview {
                    certificate,
                })
                | Ok(threadmoth::toolbelt::structural::StructuralOutcome::Applied {
                    certificate,
                }) => serde_json::to_value(certificate).map_err(|e| e.to_string()),
                Ok(threadmoth::toolbelt::structural::StructuralOutcome::PlanPrepared {
                    ..
                }) => {
                    unreachable!()
                }
                Err(e) => Err(e.to_string()),
            }
        }
        _ => Err("unknown Threadmoth tool".into()),
    }
}

fn execute_mcp_edit(workspace: &Workspace, args: EditToolArgs) -> Result<Value, String> {
    let has_search = args.search.is_some() || args.search_file.is_some();
    let has_replace = args.replace.is_some() || args.replace_file.is_some();
    let has_document = args.document.is_some();
    let has_file = args.file_path.is_some();

    if args.search.is_some() && args.search_file.is_some() {
        return Err("contradictory parameters: cannot combine `search` with `search_file`".into());
    }
    if args.replace.is_some() && args.replace_file.is_some() {
        return Err(
            "contradictory parameters: cannot combine `replace` with `replace_file`".into(),
        );
    }
    if args.expect_count.is_some() && args.all {
        return Err("contradictory parameters: cannot combine `expect_count` with `all`".into());
    }
    if let Some(0) = args.expect_count {
        return Err(
            "`expect_count` must be at least 1; to ensure absence use an edit document".into(),
        );
    }

    if has_document && (has_search || has_replace) {
        return Err(
            "contradictory parameters: cannot combine multi-file `document` with inline `search`/`replace`"
                .into(),
        );
    }

    if has_document && has_file {
        return Err(
            "contradictory parameters: provide either `document` content or a `file_path` to an edit document, not both"
                .into(),
        );
    }

    if has_search != has_replace {
        if has_search {
            if args.search_file.is_some() {
                return Err("incomplete inline edit: `search_file` was provided without `replace` or `replace_file`".into());
            } else {
                return Err(
                    "incomplete inline edit: `search` was provided without `replace`".into(),
                );
            }
        } else if args.replace_file.is_some() {
            return Err("incomplete inline edit: `replace_file` was provided without `search` or `search_file`".into());
        } else {
            return Err("incomplete inline edit: `replace` was provided without `search`".into());
        }
    }

    if args.within.is_some() && (has_document || !has_search || !has_replace) {
        return Err(
            "multi-file edit document mode does not accept `within`; scope anchors are only valid for single-file inline edits with search and replace"
                .into(),
        );
    }

    if (args.expect_count.is_some() || args.all) && !has_search {
        return Err(
            "`expect_count` and `all` are only valid for single-file edits with search and replace"
                .into(),
        );
    }

    if has_search && has_replace {
        let Some(file_path) = args.file_path else {
            return Err(
                "single-file inline edit requires `file_path` alongside `search` and `replace`"
                    .into(),
            );
        };

        let search = if let Some(s) = args.search {
            s
        } else if let Some(path) = &args.search_file {
            let resolved = workspace
                .resolve_namespaced_path(path, &threadmoth::path::PathNamespace::Native)
                .map_err(|e| format!("search_file path refused: {e}"))?;
            let bytes = workspace
                .read_file(&resolved)
                .map_err(|e| format!("cannot read search_file {resolved}: {e}"))?;
            if bytes.len() > MAX_REQUEST_BYTES {
                return Err(format!("search_file exceeds {MAX_REQUEST_BYTES} bytes"));
            }
            String::from_utf8(bytes).map_err(|_| "search_file must be valid UTF-8".to_string())?
        } else {
            unreachable!()
        };

        let replace = if let Some(r) = args.replace {
            r
        } else if let Some(path) = &args.replace_file {
            let resolved = workspace
                .resolve_namespaced_path(path, &threadmoth::path::PathNamespace::Native)
                .map_err(|e| format!("replace_file path refused: {e}"))?;
            let bytes = workspace
                .read_file(&resolved)
                .map_err(|e| format!("cannot read replace_file {resolved}: {e}"))?;
            if bytes.len() > MAX_REQUEST_BYTES {
                return Err(format!("replace_file exceeds {MAX_REQUEST_BYTES} bytes"));
            }
            String::from_utf8(bytes).map_err(|_| "replace_file must be valid UTF-8".to_string())?
        } else {
            unreachable!()
        };

        let newline_policy = match args.newline_policy.as_deref().unwrap_or("source") {
            "source" => threadmoth::source_prepare::NewlinePolicy::Source,
            "exact" => threadmoth::source_prepare::NewlinePolicy::Exact,
            other => {
                return Err(format!(
                    "invalid newline policy '{other}': expected 'source' or 'exact'"
                ))
            }
        };

        let cardinality = if let Some(n) = args.expect_count {
            if n == 1 {
                Cardinality::ExactlyOne
            } else {
                Cardinality::Exactly(n)
            }
        } else if args.all {
            Cardinality::All
        } else {
            Cardinality::ExactlyOne
        };

        let request = threadmoth::source_prepare::SourceAwareTextRequest {
            file_path,
            edits: vec![threadmoth::source_prepare::TextReplacementIntent {
                old: search,
                new: replace,
            }],
            newline_policy,
            within: args.within,
            cardinality,
            allow_newline_conversion: args.allow_newline_conversion,
        };
        return match threadmoth::source_prepare::prepare(workspace, &request) {
            threadmoth::source_prepare::SourcePreparationResult::Prepared(prepared) => {
                if args.plan {
                    serde_json::to_value(&prepared.plan).map_err(|e| e.to_string())
                } else {
                    let result =
                        threadmoth::pipeline::apply_prepared_plan(workspace, &prepared.plan);
                    match result {
                        threadmoth::protocol::PlanApplyResult::Certificate(certificate) => {
                            serde_json::to_value(certificate).map_err(|e| e.to_string())
                        }
                        threadmoth::protocol::PlanApplyResult::Transaction(certificate) => {
                            serde_json::to_value(certificate).map_err(|e| e.to_string())
                        }
                    }
                }
            }
            threadmoth::source_prepare::SourcePreparationResult::Refused { certificate } => {
                serde_json::to_value(certificate).map_err(|e| e.to_string())
            }
        };
    }

    if let Some(document) = args.document {
        return match threadmoth::edit_document::prepare(workspace, &document) {
            Ok(prepared) => {
                if args.plan {
                    serde_json::to_value(&prepared.plan).map_err(|e| e.to_string())
                } else {
                    let result =
                        threadmoth::pipeline::apply_prepared_plan(workspace, &prepared.plan);
                    match result {
                        threadmoth::protocol::PlanApplyResult::Certificate(certificate) => {
                            serde_json::to_value(certificate).map_err(|e| e.to_string())
                        }
                        threadmoth::protocol::PlanApplyResult::Transaction(certificate) => {
                            serde_json::to_value(certificate).map_err(|e| e.to_string())
                        }
                    }
                }
            }
            Err(threadmoth::edit_document::EditDocumentFailure::Invalid(diagnostic)) => {
                Ok(serde_json::json!({
                    "outcome": "REFUSED",
                    "reason_code": diagnostic.reason_code,
                    "line": diagnostic.line,
                    "message": diagnostic.message
                }))
            }
            Err(threadmoth::edit_document::EditDocumentFailure::Refused(certificate)) => {
                serde_json::to_value(certificate).map_err(|e| e.to_string())
            }
        };
    }

    if let Some(doc_path) = args.file_path {
        let resolved = workspace
            .resolve_namespaced_path(&doc_path, &threadmoth::path::PathNamespace::Native)
            .map_err(|e| e.to_string())?;
        let bytes = workspace.read_file(&resolved).map_err(|e| e.to_string())?;
        let document = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        if !document.lines().any(|line| line.starts_with("*** File: ")) {
            return Err(format!(
                "single-file editing requires `search` and `replace`. File `{doc_path}` does not contain edit document `*** File: ` headers"
            ));
        }
        return match threadmoth::edit_document::prepare(workspace, &document) {
            Ok(prepared) => {
                if args.plan {
                    serde_json::to_value(&prepared.plan).map_err(|e| e.to_string())
                } else {
                    let result =
                        threadmoth::pipeline::apply_prepared_plan(workspace, &prepared.plan);
                    match result {
                        threadmoth::protocol::PlanApplyResult::Certificate(certificate) => {
                            serde_json::to_value(certificate).map_err(|e| e.to_string())
                        }
                        threadmoth::protocol::PlanApplyResult::Transaction(certificate) => {
                            serde_json::to_value(certificate).map_err(|e| e.to_string())
                        }
                    }
                }
            }
            Err(threadmoth::edit_document::EditDocumentFailure::Invalid(diagnostic)) => {
                Ok(serde_json::json!({
                    "outcome": "REFUSED",
                    "reason_code": diagnostic.reason_code,
                    "line": diagnostic.line,
                    "message": diagnostic.message
                }))
            }
            Err(threadmoth::edit_document::EditDocumentFailure::Refused(certificate)) => {
                serde_json::to_value(certificate).map_err(|e| e.to_string())
            }
        };
    }

    Err(
        "empty edit request: provide `file_path` with `search` and `replace`, or a multi-file `document`"
            .into(),
    )
}

fn json_rpc_error(id: Value, code: i32, message: String) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn read_mcp_line(reader: &mut impl BufRead) -> io::Result<Option<Result<String, usize>>> {
    let mut bytes = Vec::new();
    let mut actual = 0usize;
    loop {
        let (content_len, available_len) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                if actual == 0 && bytes.is_empty() {
                    return Ok(None);
                }
                break;
            }
            let content_len = available
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(available.len());
            actual = actual.saturating_add(content_len);
            if bytes.len() <= MAX_REQUEST_BYTES {
                let room = MAX_REQUEST_BYTES
                    .saturating_add(1)
                    .saturating_sub(bytes.len());
                bytes.extend_from_slice(&available[..content_len.min(room)]);
            }
            (content_len, available.len())
        };
        let consumed = if content_len < available_len {
            content_len + 1
        } else {
            content_len
        };
        reader.consume(consumed);
        if content_len < available_len {
            break;
        }
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
        actual = actual.saturating_sub(1);
    }
    if actual > MAX_REQUEST_BYTES {
        return Ok(Some(Err(actual)));
    }
    String::from_utf8(bytes)
        .map(|line| Some(Ok(line)))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
