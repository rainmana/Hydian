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

pub async fn run(cli: Cli) -> Result<()> {
    let printer = Printer::from_cli(&cli);
    let plain_requested = cli.plain;
    let paths = HydianPaths::resolve(
        cli.home.as_deref(),
        cli.config.as_deref(),
        cli.mcp_config.as_deref(),
    )?;
    let profile_override = cli.profile.clone();

    match cli.command {
        None => {
            if crate::tui::default_launch_allowed(plain_requested) {
                run_tui_gateway(&paths, profile_override.as_deref()).await
            } else {
                print_help()
            }
        }
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
                if !printer.is_json() {
                    printer.success("doctor", &report, &human);
                }
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
        Some(Command::Serve(arguments)) => {
            run_serve(&printer, &paths, profile_override.as_deref(), arguments.tui).await
        }
        Some(Command::Stdio) => run_stdio(&paths, profile_override.as_deref()).await,
        Some(Command::Tui) => run_tui_gateway(&paths, profile_override.as_deref()).await,
        Some(Command::Service(arguments)) => run_service(&printer, &paths, arguments.command).await,
        Some(Command::Expose(arguments)) => run_exposure(&printer, &paths, arguments.command).await,
        #[cfg(debug_assertions)]
        Some(Command::Fixture(arguments)) => crate::fixture::run(arguments).await,
    }
}

struct LoadedRuntime {
    runtime: std::sync::Arc<crate::runtime::Runtime>,
    _log_guard: tracing_appender::non_blocking::WorkerGuard,
}

async fn load_runtime(paths: &HydianPaths, profile: Option<&str>) -> Result<LoadedRuntime> {
    paths.create_directories()?;
    let config = HydianConfig::load(&paths.config)?;
    let log_guard = crate::logging::init(&config, &paths.logs)?;
    let mcp = load_mcp_config(&paths.mcp_config)?;
    let runtime = crate::runtime::Runtime::start(config, mcp, paths.clone(), profile).await?;
    Ok(LoadedRuntime {
        runtime,
        _log_guard: log_guard,
    })
}

async fn run_serve(
    printer: &Printer,
    paths: &HydianPaths,
    profile: Option<&str>,
    with_tui: bool,
) -> Result<()> {
    let loaded = load_runtime(paths, profile).await?;
    let runtime = loaded.runtime.clone();
    let frontend = crate::frontend::streamable_http::start(runtime.clone()).await?;
    let status = runtime.status().await;
    runtime.write_status().await?;
    if with_tui {
        crate::tui::run(runtime.clone(), paths).await?;
    } else {
        printer.success(
            "serve",
            &json!({
                "endpoint": status.endpoint,
                "state": status.state,
                "address": frontend.address,
                "status_file": paths.status,
            }),
            &[
                format!("✓ READY: Hydian is listening at {}", status.endpoint),
                format!("STATE: {:?}", status.state).to_uppercase(),
                format!("STATUS: http://{}/status", frontend.address),
                "Press Ctrl+C to stop.".into(),
            ],
        );
        tokio::signal::ctrl_c()
            .await
            .context("could not listen for Ctrl+C")?;
    }
    frontend.shutdown().await?;
    runtime.shutdown().await;
    Ok(())
}

async fn run_stdio(paths: &HydianPaths, profile: Option<&str>) -> Result<()> {
    let loaded = load_runtime(paths, profile).await?;
    crate::frontend::stdio::serve(loaded.runtime).await
}

async fn run_tui_gateway(paths: &HydianPaths, profile: Option<&str>) -> Result<()> {
    let loaded = load_runtime(paths, profile).await?;
    let runtime = loaded.runtime.clone();
    let frontend = crate::frontend::streamable_http::start(runtime.clone()).await?;
    runtime.write_status().await?;
    let result = crate::tui::run(runtime.clone(), paths).await;
    let frontend_result = frontend.shutdown().await;
    runtime.shutdown().await;
    result.and(frontend_result)
}

async fn run_service(
    printer: &Printer,
    paths: &HydianPaths,
    command: crate::cli::ServiceCommand,
) -> Result<()> {
    use crate::cli::ServiceCommand;
    let (system, dry_run) = match &command {
        ServiceCommand::Install {
            system, dry_run, ..
        }
        | ServiceCommand::Uninstall { system, dry_run }
        | ServiceCommand::Start { system, dry_run }
        | ServiceCommand::Stop { system, dry_run }
        | ServiceCommand::Restart { system, dry_run } => (*system, *dry_run),
        ServiceCommand::Status { system } => (*system, false),
    };
    let plan = crate::service::current_plan(system, paths)?;
    let commands = match &command {
        ServiceCommand::Install { .. } => plan.install_commands.clone(),
        ServiceCommand::Uninstall { .. } => plan.uninstall_commands.clone(),
        ServiceCommand::Start { .. } => vec![plan.start_command.clone()],
        ServiceCommand::Stop { .. } => vec![plan.stop_command.clone()],
        ServiceCommand::Restart { .. } => {
            vec![plan.stop_command.clone(), plan.start_command.clone()]
        }
        ServiceCommand::Status { .. } => vec![plan.status_command.clone()],
    };
    let action = match &command {
        ServiceCommand::Install { .. } => "install",
        ServiceCommand::Uninstall { .. } => "uninstall",
        ServiceCommand::Start { .. } => "start",
        ServiceCommand::Stop { .. } => "stop",
        ServiceCommand::Restart { .. } => "restart",
        ServiceCommand::Status { .. } => "status",
    };
    let output = if dry_run {
        None
    } else {
        match command {
            ServiceCommand::Install {
                acknowledge_temporary_path,
                ..
            } => {
                if crate::service::is_temporary_path(&plan.executable)
                    && !acknowledge_temporary_path
                {
                    bail!(
                        "refusing to install a service from temporary executable {}; move Hydian to a stable path or pass --acknowledge-temporary-path",
                        plan.executable.display()
                    );
                }
                crate::service::install(&plan).await?;
                Some("service definition installed and verified".into())
            }
            ServiceCommand::Uninstall { .. } => {
                crate::service::uninstall(&plan).await?;
                Some("service definition removed".into())
            }
            ServiceCommand::Start { .. } => {
                Some(crate::service::execute(&plan.start_command).await?)
            }
            ServiceCommand::Stop { .. } => Some(crate::service::execute(&plan.stop_command).await?),
            ServiceCommand::Restart { .. } => {
                let _ = crate::service::execute(&plan.stop_command).await;
                Some(crate::service::execute(&plan.start_command).await?)
            }
            ServiceCommand::Status { .. } => {
                Some(crate::service::execute(&plan.status_command).await?)
            }
        }
    };
    let mut human = vec![
        if dry_run {
            format!("PREVIEW: service {action}")
        } else {
            format!("✓ READY: service {action}")
        },
        format!("PLATFORM: {:?}", plan.platform),
        format!("SEMANTICS: {}", plan.semantics),
        format!("EXECUTABLE: {}", plan.executable.display()),
        format!("HYDIAN HOME: {}", plan.hydian_home.display()),
        format!("DEFINITION: {}", plan.definition_path.display()),
        format!("CONFIGURATION: {}", plan.configuration_path.display()),
        format!("LOGS: {}", plan.log_path.display()),
    ];
    human.extend(
        commands
            .iter()
            .map(|command| format!("COMMAND: {}", crate::service::display_command(command))),
    );
    if let Some(output) = &output {
        human.push(format!("RESULT: {output}"));
    }
    printer.success(
        &format!("service {action}"),
        &json!({"dry_run": dry_run, "plan": plan, "commands": commands, "result": output}),
        &human,
    );
    Ok(())
}

async fn run_exposure(
    printer: &Printer,
    paths: &HydianPaths,
    command: crate::cli::ExposeCommand,
) -> Result<()> {
    use crate::{
        cli::ExposeCommand,
        exposure::{CommandProvider, ExposureProvider},
    };
    let config = HydianConfig::load(&paths.config)?;
    let is_start_command = matches!(&command, ExposeCommand::Start(_));
    match command {
        ExposeCommand::Plan(arguments) | ExposeCommand::Start(arguments) => {
            let provider = CommandProvider::new(&arguments.provider);
            let plan = provider
                .plan(
                    &config,
                    arguments.scope.as_deref(),
                    arguments.mode.as_deref(),
                    &arguments.provider_args,
                )
                .await?;
            let should_start = is_start_command && !arguments.dry_run;
            let state = if should_start {
                paths.create_directories()?;
                Some(provider.start(&plan, paths).await?)
            } else {
                None
            };
            let mut human = vec![
                if should_start {
                    format!("✓ READY: {} exposure started", plan.provider)
                } else {
                    format!("PREVIEW: {} exposure", plan.provider)
                },
                format!(
                    "EXECUTABLE: {}",
                    plan.detection.executable.as_ref().map_or_else(
                        || "<provider command>".into(),
                        |path| path.display().to_string()
                    )
                ),
                format!(
                    "VERSION: {}",
                    plan.detection.version.as_deref().unwrap_or("unavailable")
                ),
                format!("LOCAL ENDPOINT: {}", plan.local_url),
                format!("COMMAND: {}", plan.command_display),
                format!("SCOPE: {}", plan.expected_scope),
                format!("AUTHENTICATION: {}", plan.authentication),
                format!("TLS: {}", plan.tls),
                format!("EXPERIMENTAL: {}", plan.experimental),
            ];
            human.extend(
                plan.limitations
                    .iter()
                    .map(|limitation| format!("LIMITATION: {limitation}")),
            );
            if let Some(state) = &state {
                human.push(format!(
                    "PUBLIC URL: {}",
                    state.public_url.as_deref().unwrap_or("not available")
                ));
            }
            printer.success(
                if should_start {
                    "expose start"
                } else {
                    "expose plan"
                },
                &json!({"plan": plan, "started": should_start, "state": state}),
                &human,
            );
            Ok(())
        }
        ExposeCommand::Stop(arguments) => {
            let state = if let Some(provider) = &arguments.provider {
                CommandProvider::new(provider).status(paths).await?
            } else {
                crate::exposure::CommandProvider::new("custom")
                    .status(paths)
                    .await?
            };
            let name = arguments
                .provider
                .or_else(|| state.as_ref().map(|state| state.provider.clone()))
                .unwrap_or_else(|| "custom".into());
            if !arguments.dry_run {
                CommandProvider::new(&name).stop(paths).await?;
            }
            printer.success(
                "expose stop",
                &json!({"provider": name, "dry_run": arguments.dry_run, "previous_state": state}),
                &[
                    if arguments.dry_run {
                        format!("PREVIEW: stop {name} exposure")
                    } else {
                        format!("✓ READY: {name} exposure stopped")
                    },
                    format!("STATUS FILE: {}", paths.run.join("exposure.json").display()),
                ],
            );
            Ok(())
        }
        ExposeCommand::Status => {
            let state = CommandProvider::new("custom").status(paths).await?;
            printer.success(
                "expose status",
                &state,
                &[state.as_ref().map_or_else(
                    || "• STOPPED: no Hydian-managed exposure".into(),
                    |state| {
                        format!(
                            "{}: {} {}",
                            if state.running {
                                "✓ RUNNING"
                            } else {
                                "! STOPPED"
                            },
                            state.provider,
                            state.public_url.as_deref().unwrap_or("")
                        )
                    },
                )],
            );
            Ok(())
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
