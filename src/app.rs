use std::{fs, io, path::PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::CommandFactory;
use clap_complete::generate;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    cli::{Cli, Command, EndpointFormat, ExplainTopic, ProfileCommand, ServerCommand, ToolCommand},
    config::{
        HydianConfig, WriteOutcome, atomic_write_with_backup, load_mcp_config, write_mcp_config,
    },
    diagnostics::{DiagnosticStatus, run as run_doctor},
    import::{apply_import, plan_import, read_import},
    model::McpConfig,
    output::Printer,
    paths::HydianPaths,
    profiles::{list_profiles, show_profile, use_profile},
    secrets::redact_json,
};

#[allow(clippy::unused_async)]
pub async fn run(cli: Cli) -> Result<()> {
    let printer = Printer::from_cli(&cli);
    let paths = HydianPaths::resolve(
        cli.home.as_deref(),
        cli.config.as_deref(),
        cli.mcp_config.as_deref(),
    )?;
    let profile_override = cli.profile.clone();

    match cli.command {
        None => print_help(),
        Some(Command::Init(arguments)) => {
            run_init(&printer, &paths, arguments.dry_run, arguments.force)
        }
        Some(Command::Import(arguments)) => {
            let imported = read_import(&arguments.path, arguments.format)?;
            let planned = plan_import(
                &arguments.path,
                &paths.mcp_config,
                imported,
                arguments.on_conflict,
                &arguments.rename_suffix,
            )?;
            let mut human = render_import_plan(&planned.plan);
            let outcome = if arguments.apply {
                paths.create_directories()?;
                let outcome = apply_import(&planned, &paths.backups)?;
                human.extend(render_write_outcome(&outcome));
                Some(outcome)
            } else {
                human.push("PREVIEW: no files were changed; pass --apply to import.".into());
                None
            };
            printer.success(
                "import",
                &json!({"plan": planned.plan, "applied": arguments.apply, "write": outcome}),
                &human,
            );
            Ok(())
        }
        Some(Command::Doctor(arguments)) => {
            let report = run_doctor(&paths, arguments.strict);
            let human = render_doctor(&report);
            if !report.ready {
                let reason = if report.has_failures() {
                    "doctor found failed checks"
                } else {
                    "doctor --strict treats warnings as failures"
                };
                printer.success("doctor", &report, &human);
                bail!("{reason}");
            }
            printer.success("doctor", &report, &human);
            Ok(())
        }
        Some(Command::Endpoint(arguments)) => {
            let config = HydianConfig::load(&paths.config)?;
            let endpoint = config.endpoint();
            let human = match arguments.format {
                EndpointFormat::Url | EndpointFormat::Json => vec![endpoint.clone()],
                EndpointFormat::Openai => vec![
                    endpoint.clone(),
                    "Use this local MCP endpoint as the upstream for OpenAI tunnel-client.".into(),
                    "Hydian does not emit an undocumented tunnel-client command.".into(),
                ],
            };
            printer.success(
                "endpoint",
                &json!({"url": endpoint, "format": format!("{:?}", arguments.format).to_lowercase()}),
                &human,
            );
            Ok(())
        }
        Some(Command::Profiles(arguments)) => run_profiles(
            &printer,
            &paths,
            profile_override.as_deref(),
            arguments.command,
        ),
        Some(Command::Servers(arguments)) => run_servers(&printer, &paths, arguments.command),
        Some(Command::Tools(arguments)) => run_tools(&printer, &paths, arguments.command),
        Some(Command::Status) => run_status(&printer, &paths),
        Some(Command::Explain(arguments)) => {
            let text = explain(arguments.topic);
            printer.success(
                "explain",
                &json!({"topic": format!("{:?}", arguments.topic), "text": text}),
                &text.lines().map(ToOwned::to_owned).collect::<Vec<_>>(),
            );
            Ok(())
        }
        Some(Command::Completion(arguments)) => {
            if printer.is_json() {
                bail!("completion scripts are text and cannot be emitted with --json");
            }
            let mut command = Cli::command();
            let binary_name = command.get_name().to_owned();
            generate(
                arguments.shell,
                &mut command,
                binary_name,
                &mut io::stdout(),
            );
            Ok(())
        }
        Some(Command::Serve(_)) => {
            bail!("the gateway runtime is not available in this build milestone")
        }
        Some(Command::Stdio) => {
            bail!("the MCP stdio frontend is not available in this build milestone")
        }
        Some(Command::Tui) => {
            bail!("the terminal interface is not available in this build milestone")
        }
        Some(Command::Service(_)) => {
            bail!("service management is not available in this build milestone")
        }
        Some(Command::Expose(_)) => {
            bail!("exposure providers are not available in this build milestone")
        }
    }
}

fn print_help() -> Result<()> {
    Cli::command().print_long_help()?;
    println!();
    Ok(())
}

#[derive(Debug, Serialize)]
struct InitReport {
    home: PathBuf,
    files: Vec<PathBuf>,
    dry_run: bool,
    writes: Vec<WriteOutcome>,
}

fn run_init(printer: &Printer, paths: &HydianPaths, dry_run: bool, force: bool) -> Result<()> {
    let schema_path = paths.home.join("config.schema.json");
    let files = vec![
        paths.config.clone(),
        paths.mcp_config.clone(),
        schema_path.clone(),
    ];
    let existing = files
        .iter()
        .filter(|path| path.exists())
        .cloned()
        .collect::<Vec<_>>();
    if !dry_run && !force && !existing.is_empty() {
        bail!(
            "Hydian configuration already exists: {}; use --dry-run to inspect or --force to replace it with backups",
            existing
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let mut writes = Vec::new();
    if !dry_run {
        paths.create_directories()?;
        writes.push(HydianConfig::default().write(&paths.config, &paths.backups)?);
        writes.push(write_mcp_config(
            &McpConfig::default(),
            &paths.mcp_config,
            &paths.backups,
        )?);
        let schema = serde_json::to_vec_pretty(&HydianConfig::json_schema()?)?;
        writes.push(atomic_write_with_backup(
            &schema_path,
            &schema,
            &paths.backups,
        )?);
    }

    let report = InitReport {
        home: paths.home.clone(),
        files,
        dry_run,
        writes,
    };
    let mut human = vec![
        if dry_run {
            "PREVIEW: Hydian initialization".into()
        } else {
            "✓ READY: Hydian configuration initialized".into()
        },
        format!("HOME: {}", paths.home.display()),
        format!("CONFIGURATION: {}", paths.config.display()),
        format!("MCP CONFIGURATION: {}", paths.mcp_config.display()),
        format!("LOGS: {}", paths.logs.display()),
    ];
    if dry_run {
        human.push("No files were changed.".into());
    } else {
        for write in &report.writes {
            human.extend(render_write_outcome(write));
        }
    }
    printer.success("init", &report, &human);
    Ok(())
}

fn run_profiles(
    printer: &Printer,
    paths: &HydianPaths,
    profile_override: Option<&str>,
    command: ProfileCommand,
) -> Result<()> {
    let mut config = HydianConfig::load(&paths.config)?;
    if let Some(profile) = profile_override {
        if !config.profiles.contains_key(profile) {
            bail!("profile `{profile}` is not defined");
        }
        config.runtime.active_profile = profile.into();
    }
    let mcp = load_mcp_config(&paths.mcp_config)?;

    match command {
        ProfileCommand::List => {
            let summaries = list_profiles(&config, &mcp)?;
            let human = summaries
                .iter()
                .map(|profile| {
                    format!(
                        "{} {} ({} server(s))",
                        if profile.active { "✓ ACTIVE" } else { "•" },
                        profile.name,
                        profile.resolved_servers.len()
                    )
                })
                .collect::<Vec<_>>();
            printer.success("profiles list", &summaries, &human);
        }
        ProfileCommand::Show { name } => {
            let summary = show_profile(&config, &mcp, &name)?;
            let human = vec![
                format!("PROFILE: {}", summary.name),
                format!("ACTIVE: {}", summary.active),
                format!("CONFIGURED: {}", summary.configured_servers.join(", ")),
                format!("VISIBLE: {}", summary.resolved_servers.join(", ")),
            ];
            printer.success("profiles show", &summary, &human);
        }
        ProfileCommand::Use { name, dry_run } => {
            let outcome = use_profile(&mut config, paths, &name, dry_run)?;
            let mut human = vec![
                if dry_run {
                    "PREVIEW: active profile change".into()
                } else {
                    "✓ READY: active profile changed".into()
                },
                format!("PROFILE: {name}"),
                format!("CONFIGURATION: {}", paths.config.display()),
                "MCP CONFIGURATION: unchanged".into(),
            ];
            if let Some(outcome) = &outcome {
                human.extend(render_write_outcome(outcome));
            } else {
                human.push("No files were changed.".into());
            }
            printer.success(
                "profiles use",
                &json!({"profile": name, "dry_run": dry_run, "write": outcome}),
                &human,
            );
        }
    }
    Ok(())
}

fn run_servers(printer: &Printer, paths: &HydianPaths, command: ServerCommand) -> Result<()> {
    let config = load_mcp_config(&paths.mcp_config)?;
    match command {
        ServerCommand::List => {
            let summaries = config
                .servers
                .iter()
                .map(|(name, definition)| {
                    json!({
                        "name": name,
                        "enabled": definition.enabled,
                        "transport": definition.transport(),
                    })
                })
                .collect::<Vec<_>>();
            let human = summaries
                .iter()
                .map(|server| {
                    format!(
                        "{} {} ({})",
                        if server["enabled"].as_bool().unwrap_or(false) {
                            "• CONFIGURED"
                        } else {
                            "• DISABLED"
                        },
                        server["name"].as_str().unwrap_or_default(),
                        server["transport"]
                            .as_str()
                            .unwrap_or("unrecognized transport")
                    )
                })
                .collect::<Vec<_>>();
            printer.success("servers list", &summaries, &human);
            Ok(())
        }
        ServerCommand::Show { name } => {
            let definition = config
                .servers
                .get(&name)
                .ok_or_else(|| anyhow!("server `{name}` is not configured"))?;
            let redacted = redact_json(&serde_json::to_value(definition)?);
            printer.success(
                "servers show",
                &json!({"name": name, "definition": redacted}),
                &[
                    format!("SERVER: {name}"),
                    serde_json::to_string_pretty(&redacted)?,
                ],
            );
            Ok(())
        }
        ServerCommand::Start(arguments) => {
            runtime_mutation("start", &arguments.name, arguments.dry_run)
        }
        ServerCommand::Stop(arguments) => {
            runtime_mutation("stop", &arguments.name, arguments.dry_run)
        }
        ServerCommand::Restart(arguments) => {
            runtime_mutation("restart", &arguments.name, arguments.dry_run)
        }
    }
}

fn runtime_mutation(action: &str, name: &str, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("PREVIEW: would {action} backend `{name}`");
        Ok(())
    } else {
        bail!(
            "cannot {action} backend `{name}` because no running Hydian control channel was found"
        )
    }
}

fn run_tools(printer: &Printer, paths: &HydianPaths, command: ToolCommand) -> Result<()> {
    let status = read_status(paths)?;
    let tools = status
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let selected = match command {
        ToolCommand::List => tools,
        ToolCommand::Search { query } => {
            let normalized = query.to_ascii_lowercase();
            tools
                .into_iter()
                .filter(|tool| tool.to_string().to_ascii_lowercase().contains(&normalized))
                .collect()
        }
        ToolCommand::Show { qualified_name } => {
            let tool = tools
                .into_iter()
                .find(|tool| tool["qualified_name"] == qualified_name)
                .ok_or_else(|| anyhow!("tool `{qualified_name}` is not in the runtime catalog"))?;
            vec![tool]
        }
    };
    let human = selected
        .iter()
        .map(|tool| {
            tool.get("qualified_name")
                .and_then(Value::as_str)
                .unwrap_or("<unnamed tool>")
                .to_owned()
        })
        .collect::<Vec<_>>();
    printer.success("tools", &selected, &human);
    Ok(())
}

fn run_status(printer: &Printer, paths: &HydianPaths) -> Result<()> {
    let status = read_status(paths)?;
    let human = vec![
        format!(
            "STATE: {}",
            status["state"].as_str().unwrap_or("unknown").to_uppercase()
        ),
        format!(
            "ENDPOINT: {}",
            status["endpoint"].as_str().unwrap_or("unavailable")
        ),
        format!("STATUS FILE: {}", paths.status.display()),
    ];
    printer.success("status", &status, &human);
    Ok(())
}

fn read_status(paths: &HydianPaths) -> Result<Value> {
    let input = fs::read_to_string(&paths.status).with_context(|| {
        format!(
            "runtime status is unavailable at {}; start Hydian with `hydian serve`",
            paths.status.display()
        )
    })?;
    serde_json::from_str(&input)
        .with_context(|| format!("runtime status is invalid at {}", paths.status.display()))
}

fn render_import_plan(plan: &crate::import::ImportPlan) -> Vec<String> {
    let mut lines = vec![
        format!("SOURCE: {}", plan.source.display()),
        format!("DESTINATION: {}", plan.destination.display()),
        format!("FORMAT: {:?}", plan.format),
    ];
    for entry in &plan.entries {
        lines.push(format!(
            "{:?}: {} -> {} ({})",
            entry.action, entry.source_name, entry.destination_name, entry.reason
        ));
        if !entry.unknown_fields.is_empty() {
            lines.push(format!(
                "  UNRECOGNIZED FIELDS PRESERVED: {}",
                entry.unknown_fields.join(", ")
            ));
        }
    }
    if !plan.root_unknown_fields.is_empty() {
        lines.push(format!(
            "UNRECOGNIZED ROOT FIELDS: {}",
            plan.root_unknown_fields.join(", ")
        ));
    }
    lines
}

fn render_doctor(report: &crate::diagnostics::DoctorReport) -> Vec<String> {
    let mut lines = Vec::new();
    for check in &report.checks {
        let label = match check.status {
            DiagnosticStatus::Ready => "✓ READY",
            DiagnosticStatus::Warning => "! WARNING",
            DiagnosticStatus::Failed => "✗ FAILED",
        };
        lines.push(format!("{label}: {}", check.name));
        lines.push(format!("    REASON: {}", check.reason));
        lines.push(format!("    AFFECTED: {}", check.affected));
        if let Some(configuration) = &check.configuration {
            lines.push(format!("    CONFIGURATION: {configuration}"));
        }
        if let Some(fix) = &check.fix {
            lines.push(format!("    FIX: {fix}"));
        }
    }
    lines
}

fn render_write_outcome(outcome: &WriteOutcome) -> Vec<String> {
    let mut lines = vec![format!("CHANGED FILE: {}", outcome.changed_file.display())];
    if let Some(backup) = &outcome.backup_file {
        lines.push(format!("BACKUP FILE: {}", backup.display()));
        lines.push(format!(
            "ROLLBACK: replace {} with {}",
            outcome.changed_file.display(),
            backup.display()
        ));
    } else {
        lines.push(format!(
            "ROLLBACK: remove {}",
            outcome.changed_file.display()
        ));
    }
    lines
}

fn explain(topic: ExplainTopic) -> &'static str {
    match topic {
        ExplainTopic::NonLoopbackWithoutAuth => {
            "A non-loopback listener can accept traffic from other machines or network namespaces.\nHydian v0.1 sends that traffic as plaintext HTTP and does not authenticate clients.\nEvery tool in the active profile may become reachable through that listener.\nPrefer loopback plus an authenticated tunnel or reverse proxy. If the network design is intentional, set `acknowledgements.non_loopback_without_auth = true`."
        }
        ExplainTopic::OriginValidation => {
            "Origin validation rejects browser-originated requests whose Origin header is not trusted.\nThis is required by the MCP Streamable HTTP transport to reduce DNS rebinding risk.\nDisabling it lets a malicious website attempt requests to a local Hydian listener.\nKeep it enabled unless another trusted layer performs equivalent validation."
        }
        ExplainTopic::PlaintextSecrets => {
            "Literal header values in mcp.json can expose credentials through file copies and support bundles.\nUse `env:VARIABLE_NAME` or `file:absolute/path` so Hydian resolves the value only when connecting.\nHydian redacts likely secret fields, but it cannot make an already shared configuration private."
        }
        ExplainTopic::ProviderExposure => {
            "Exposure providers make the local endpoint reachable beyond this computer.\nThe provider determines who can connect, where TLS terminates, and whether identity is verified.\nRun `hydian expose plan <provider>` and review the exact command, scope, authentication facts, and limitations before launch."
        }
    }
}
