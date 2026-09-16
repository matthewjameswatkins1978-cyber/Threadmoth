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
    path: String,
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
    path: String,
    #[serde(default)]
    goal: Option<String>,
    #[serde(default)]
    at: Option<String>,
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
    for_path: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExactReplaceToolArgs {
    file: String,
    old: String,
    new: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetValueToolArgs {
    file: String,
    path: String,
    value: Value,
}

fn default_suggestion_mode() -> String {
    "safe".into()
}

fn shorthand_request(file: &str, operation: OperationPayload, bytes: usize) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: format!("mcp-shorthand-{file}"),
        allow_generated: false,
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
                    {"name": "threadmoth_mutate", "description": "Apply one typed Threadmoth mutation and return its certificate; target specificity must come from the user or evidence", "inputSchema": schema_for!(Request)},
                    {"name": "threadmoth_preview", "description": "Preview one typed Threadmoth mutation without writing; do not invent target specificity", "inputSchema": schema_for!(Request)},
                    {"name": "threadmoth_plan", "description": "Prepare a deterministic guarded plan without writing; preserve unresolved target ambiguity", "inputSchema": schema_for!(Request)},
                    {"name": "threadmoth_apply_plan", "description": "Apply an exact prepared plan after rechecking identity and assertions", "inputSchema": schema_for!(PreparedPlan)},
                    {"name": "threadmoth_inspect", "description": "Read unchanged identity facts or a bounded deterministic outline/expansion; observation handles are stale-safe read identities and never mutation authority", "inputSchema": schema_for!(InspectToolArgs)},
                    {"name": "threadmoth_suggest", "description": "Return deterministic request suggestions and candidate evidence; never choose among unresolved candidates", "inputSchema": schema_for!(SuggestToolArgs)},
                    {"name": "threadmoth_explain", "description": "Return stable metadata for a refusal or failure reason", "inputSchema": schema_for!(ExplainToolArgs)},
                    {"name": "threadmoth_capabilities", "description": "Return Threadmoth capabilities, optionally scoped to a provider or path", "inputSchema": schema_for!(CapabilitiesToolArgs)},
                    {"name": "threadmoth_transact_preview", "description": "Preview a guarded transaction without writing", "inputSchema": schema_for!(TransactionRequest)},
                    {"name": "threadmoth_transact", "description": "Prepare and commit a guarded transaction", "inputSchema": schema_for!(TransactionRequest)},
                    {"name": "threadmoth_exact_replace", "description": "Safely replace one exact text occurrence through the canonical pipeline after the occurrence is uniquely established", "inputSchema": schema_for!(ExactReplaceToolArgs)},
                    {"name": "threadmoth_set_value", "description": "Safely set one selected JSON, JSONC, TOML, YAML, INI or dotenv value through the canonical registry and Core pipeline; do not guess among plausible targets", "inputSchema": schema_for!(SetValueToolArgs)}
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
                let value = call_tool(workspace, name, arguments, observation_cache);
                let result = match value {
                    Ok(value) => {
                        json!({"content": [{"type": "text", "text": serde_json::to_string(&value).unwrap()}], "structuredContent": value})
                    }
                    Err(error) => {
                        json!({"isError": true, "content": [{"type": "text", "text": error}]})
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

fn call_tool(
    workspace: &Workspace,
    name: &str,
    arguments: Value,
    observation_cache: &mut threadmoth::metadata::ObservationCache,
) -> Result<Value, String> {
    match name {
        "threadmoth_exact_replace" => {
            let args =
                serde_json::from_value::<ExactReplaceToolArgs>(arguments).map_err(schema_error)?;
            let request = shorthand_request(
                &args.file,
                OperationPayload::Text(threadmoth::provider::text::TextOperation::Replace {
                    target: args.old.clone(),
                    replacement: args.new.clone(),
                }),
                args.old.len().max(args.new.len()),
            );
            serde_json::to_value(execute_request(workspace, &request, false))
                .map_err(|error| error.to_string())
        }
        "threadmoth_set_value" => {
            let args =
                serde_json::from_value::<SetValueToolArgs>(arguments).map_err(schema_error)?;
            let bytes = serde_json::to_vec(&args.value)
                .map_err(|error| error.to_string())?
                .len();
            let operation =
                threadmoth::shorthand::set_value_operation(&args.file, &args.path, args.value)?;
            let request = shorthand_request(&args.file, operation, bytes);
            serde_json::to_value(execute_request(workspace, &request, false))
                .map_err(|error| error.to_string())
        }
        "threadmoth_capabilities" | "suture_capabilities" => {
            let args =
                serde_json::from_value::<CapabilitiesToolArgs>(arguments).map_err(schema_error)?;
            let output = if let Some(path) = args.for_path {
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
                &args.path,
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
            let bytes = workspace.read_file(&args.path).ok();
            serde_json::to_value(threadmoth::metadata::suggest(
                &args.path,
                args.goal.as_deref(),
                args.at.as_deref(),
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
        _ => Err("unknown Threadmoth tool".into()),
    }
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
