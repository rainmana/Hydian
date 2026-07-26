use std::{
    fs,
    io::{self, IsTerminal, Stdout},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::{
    config::{HydianConfig, SymbolMode, TuiTheme},
    diagnostics::{DoctorReport, run as run_doctor},
    paths::HydianPaths,
    routing::ToolSummary,
    runtime::{GatewayState, Runtime, RuntimeStatus},
    secrets::redact_text,
};

const MINIMUM_WIDTH: u16 = 80;
const MINIMUM_HEIGHT: u16 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Servers,
    Tools,
    Profiles,
    Doctor,
    Exposure,
    Logs,
    Help,
}

impl Screen {
    const ALL: [Self; 8] = [
        Self::Overview,
        Self::Servers,
        Self::Tools,
        Self::Profiles,
        Self::Doctor,
        Self::Exposure,
        Self::Logs,
        Self::Help,
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Servers => "Servers",
            Self::Tools => "Tools",
            Self::Profiles => "Profiles",
            Self::Doctor => "Doctor",
            Self::Exposure => "Exposure",
            Self::Logs => "Logs",
            Self::Help => "Help",
        }
    }
}

#[allow(clippy::struct_excessive_bools)]
pub struct UiState {
    pub screen: Screen,
    pub selected: usize,
    pub query: String,
    pub searching: bool,
    pub help_overlay: bool,
    pub quit_pending: bool,
    pub symbols: SymbolMode,
    pub monochrome: bool,
    pub status: RuntimeStatus,
    pub tools: Vec<ToolSummary>,
    pub doctor: DoctorReport,
    pub profiles: Vec<(String, Vec<String>)>,
    pub logs: Vec<String>,
    pub paused_logs: bool,
    pub exposure_details: Vec<String>,
    pub pending_exposure: Option<crate::exposure::ExposurePlan>,
    pub confirmation: Option<String>,
}

impl UiState {
    fn new(
        config: &HydianConfig,
        status: RuntimeStatus,
        tools: Vec<ToolSummary>,
        doctor: DoctorReport,
    ) -> Self {
        Self {
            screen: Screen::Overview,
            selected: 0,
            query: String::new(),
            searching: false,
            help_overlay: false,
            quit_pending: false,
            symbols: config.tui.symbols,
            monochrome: matches!(
                config.tui.theme,
                TuiTheme::Monochrome | TuiTheme::HighContrast
            ),
            status,
            tools,
            doctor,
            profiles: config
                .profiles
                .iter()
                .map(|(name, profile)| (name.clone(), profile.servers.clone()))
                .collect(),
            logs: Vec::new(),
            paused_logs: false,
            exposure_details: vec![
                "Provider detection is refreshed when the dashboard starts.".into(),
            ],
            pending_exposure: None,
            confirmation: None,
        }
    }

    fn marker(&self, state: GatewayState) -> &'static str {
        match (self.symbols, state) {
            (SymbolMode::Unicode, GatewayState::Ready) => "✓ READY",
            (SymbolMode::Unicode, GatewayState::Degraded) => "! DEGRADED",
            (SymbolMode::Unicode, GatewayState::Failed) => "✗ FAILED",
            (SymbolMode::Ascii, GatewayState::Ready) => "[OK] READY",
            (SymbolMode::Ascii, GatewayState::Degraded) => "[!] DEGRADED",
            (SymbolMode::Ascii, GatewayState::Failed) => "[X] FAILED",
        }
    }

    fn next_screen(&mut self, backwards: bool) {
        let current = Screen::ALL
            .iter()
            .position(|screen| *screen == self.screen)
            .unwrap_or_default();
        let next = if backwards {
            current.checked_sub(1).unwrap_or(Screen::ALL.len() - 1)
        } else {
            (current + 1) % Screen::ALL.len()
        };
        self.screen = Screen::ALL[next];
        self.selected = 0;
    }

    fn filtered_tools(&self) -> Vec<&ToolSummary> {
        let query = self.query.to_ascii_lowercase();
        self.tools
            .iter()
            .filter(|tool| {
                query.is_empty()
                    || tool.qualified_name.to_ascii_lowercase().contains(&query)
                    || tool.original_name.to_ascii_lowercase().contains(&query)
                    || tool.backend.to_ascii_lowercase().contains(&query)
                    || tool
                        .description
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&query)
            })
            .collect()
    }
}

#[must_use]
pub fn default_launch_allowed(plain: bool) -> bool {
    default_launch_allowed_for(
        io::stdin().is_terminal(),
        io::stdout().is_terminal(),
        plain,
        std::env::var("TERM").ok().as_deref(),
    )
}

#[must_use]
pub fn default_launch_allowed_for(
    stdin_terminal: bool,
    stdout_terminal: bool,
    plain: bool,
    term: Option<&str>,
) -> bool {
    stdin_terminal
        && stdout_terminal
        && !plain
        && !term.is_some_and(|value| value.eq_ignore_ascii_case("dumb"))
}

pub async fn run(runtime: Arc<Runtime>, paths: &HydianPaths) -> Result<()> {
    if !default_launch_allowed(false) {
        bail!(
            "the terminal dashboard requires an interactive stdin and stdout; use `hydian serve` for headless operation"
        );
    }
    let config = runtime.config().clone();
    let status = runtime.status().await;
    let tools = runtime.tool_summaries().await?;
    let doctor = run_doctor(paths, false);
    let mut state = UiState::new(&config, status, tools, doctor);
    state.logs = read_logs(paths);
    state.exposure_details = exposure_details().await;

    let mut guard = TerminalGuard::enter(config.tui.mouse)?;
    install_panic_restore_hook();
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .context("could not initialize terminal renderer")?;
    terminal.clear()?;
    let tick = Duration::from_millis(config.tui.refresh_rate_ms.max(100));

    loop {
        terminal.draw(|frame| render(frame, &state))?;
        if event::poll(tick).context("could not poll terminal events")?
            && let Event::Key(key) = event::read().context("could not read terminal event")?
            && key.kind == KeyEventKind::Press
            && handle_key(key.code, key.modifiers, &mut state, &runtime, paths).await?
        {
            break;
        }
        state.status = runtime.status().await;
        state.tools = runtime.tool_summaries().await?;
        if !state.paused_logs {
            state.logs = read_logs(paths);
        }
    }

    terminal.show_cursor()?;
    guard.restore();
    Ok(())
}

async fn handle_key(
    code: KeyCode,
    modifiers: KeyModifiers,
    state: &mut UiState,
    runtime: &Runtime,
    paths: &HydianPaths,
) -> Result<bool> {
    if state.searching {
        match code {
            KeyCode::Esc | KeyCode::Enter => state.searching = false,
            KeyCode::Backspace => {
                state.query.pop();
            }
            KeyCode::Char(character) if !modifiers.contains(KeyModifiers::CONTROL) => {
                state.query.push(character);
            }
            _ => {}
        }
        return Ok(false);
    }
    match code {
        KeyCode::Char(character)
            if character == 'q'
                || character == 'c' && modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if state.quit_pending {
                return Ok(true);
            }
            state.quit_pending = true;
            state.confirmation = Some(
                "Press q again to stop the foreground Hydian runtime and all child backends."
                    .into(),
            );
        }
        KeyCode::Esc => {
            state.help_overlay = false;
            state.quit_pending = false;
            state.pending_exposure = None;
            state.confirmation = None;
        }
        KeyCode::Char('?') => state.help_overlay = !state.help_overlay,
        KeyCode::Char('/') => state.searching = true,
        KeyCode::Tab => state.next_screen(modifiers.contains(KeyModifiers::SHIFT)),
        KeyCode::Down | KeyCode::Char('j') => state.selected = state.selected.saturating_add(1),
        KeyCode::Up | KeyCode::Char('k') => state.selected = state.selected.saturating_sub(1),
        KeyCode::Char('p') if state.screen == Screen::Logs => {
            state.paused_logs = !state.paused_logs;
        }
        KeyCode::Char('r') if state.screen == Screen::Servers => {
            if let Some(backend) = state.status.backends.get(state.selected) {
                runtime.restart_backend(&backend.name).await?;
            }
        }
        KeyCode::Char('s') if state.screen == Screen::Servers => {
            if let Some(backend) = state.status.backends.get(state.selected) {
                if matches!(
                    backend.state,
                    crate::backend::BackendState::Stopped | crate::backend::BackendState::Failed
                ) {
                    runtime.start_backend(&backend.name).await?;
                } else {
                    runtime.stop_backend(&backend.name).await?;
                }
            }
        }
        KeyCode::Char('s') if state.screen == Screen::Exposure => {
            use crate::exposure::ExposureProvider;
            crate::exposure::CommandProvider::new("custom")
                .stop(paths)
                .await?;
            state.confirmation = Some("Hydian-managed exposure was stopped.".into());
        }
        KeyCode::Enter if state.screen == Screen::Profiles => {
            if let Some((name, _)) = state.profiles.get(state.selected) {
                runtime.activate_profile(name).await?;
            }
        }
        KeyCode::Enter if state.screen == Screen::Exposure => {
            use crate::exposure::ExposureProvider;
            if let Some(plan) = state.pending_exposure.take() {
                paths.create_directories()?;
                let provider = crate::exposure::CommandProvider::new(&plan.provider);
                let provider_name = plan.provider.clone();
                provider.start(&plan, paths).await?;
                state.confirmation = Some(format!(
                    "{provider_name} exposure started. Press s on this screen to stop it."
                ));
            } else {
                let names = ["tailscale", "ngrok", "cloudflare", "custom"];
                let name = names[state.selected.min(names.len() - 1)];
                let provider = crate::exposure::CommandProvider::new(name);
                match provider.plan(runtime.config(), None, None, &[]).await {
                    Ok(plan) => {
                        state.confirmation = Some(format!(
                            "Press Enter again to launch this exact command:\n{}\nScope: {}\nAuthentication: {}\nTLS: {}",
                            plan.command_display,
                            plan.expected_scope,
                            plan.authentication,
                            plan.tls
                        ));
                        state.pending_exposure = Some(plan);
                    }
                    Err(error) => {
                        state.confirmation = Some(format!(
                            "Cannot plan {name}: {error}\nUse `hydian expose plan {name}` for provider-native arguments."
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(false)
}

pub fn render(frame: &mut Frame<'_>, state: &UiState) {
    let area = frame.area();
    if area.width < MINIMUM_WIDTH || area.height < MINIMUM_HEIGHT {
        let text = format!(
            "Hydian needs at least 80 columns × 20 rows.\nCurrent size: {} × {}.\nResize the terminal or press q to exit.",
            area.width, area.height
        );
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);
    render_header(frame, chunks[0], state);
    render_screen(frame, chunks[1], state);
    render_footer(frame, chunks[2], state);
    if state.help_overlay {
        render_help(frame, centered(area, 70, 70), state);
    }
    if let Some(message) = &state.confirmation {
        frame.render_widget(
            Paragraph::new(message.clone())
                .block(Block::default().title("Confirm").borders(Borders::ALL))
                .wrap(Wrap { trim: true }),
            centered(area, 62, 24),
        );
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let style = if state.monochrome {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("HYDIAN", style),
            Span::raw(format!(
                "  {}  {}",
                state.screen.title(),
                state.marker(state.status.state)
            )),
        ]))
        .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_screen(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    match state.screen {
        Screen::Overview => render_overview(frame, area, state),
        Screen::Servers => render_servers(frame, area, state),
        Screen::Tools => render_tools(frame, area, state),
        Screen::Profiles => render_profiles(frame, area, state),
        Screen::Doctor => render_doctor_screen(frame, area, state),
        Screen::Exposure => render_exposure(frame, area, state),
        Screen::Logs => render_logs(frame, area, state),
        Screen::Help => render_help(frame, area, state),
    }
}

fn render_overview(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let text = vec![
        Line::from(format!("PROCESS: {}", state.marker(state.status.state))),
        Line::from(format!("LOCAL MCP ENDPOINT: {}", state.status.endpoint)),
        Line::from(format!("ACTIVE PROFILE: {}", state.status.active_profile)),
        Line::from(format!(
            "BACKENDS: {} ready / {} unavailable",
            state.status.ready_backends.len(),
            state.status.unavailable_backends.len()
        )),
        Line::from(format!("VISIBLE TOOLS: {}", state.status.tool_count)),
        Line::from(format!(
            "EXPOSURE: {}",
            state
                .status
                .active_exposure_provider
                .as_deref()
                .unwrap_or("stopped")
        )),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().title("Gateway").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_servers(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let items = state
        .status
        .backends
        .iter()
        .enumerate()
        .map(|(index, backend)| {
            let marker = if index == state.selected { ">" } else { " " };
            ListItem::new(format!(
                "{marker} {:<20} {:?}  {:?}  tools={} pid={} restarts={}  {}",
                backend.name,
                backend.state,
                backend.transport,
                backend.tool_count,
                backend
                    .pid
                    .map_or_else(|| "-".into(), |pid| pid.to_string()),
                backend.restart_count,
                backend.last_error.as_deref().unwrap_or("")
            ))
        });
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title("Servers — r restart, s start/stop, Enter inspect")
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_tools(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let items = state
        .filtered_tools()
        .into_iter()
        .enumerate()
        .map(|(index, tool)| {
            let marker = if index == state.selected { ">" } else { " " };
            ListItem::new(format!(
                "{marker} {}  original={} backend={} available={}",
                tool.qualified_name, tool.original_name, tool.backend, tool.available
            ))
        });
    let title = if state.searching {
        format!("Tools — search: {}_", state.query)
    } else {
        format!("Tools — / search ({})", state.query)
    };
    frame.render_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn render_profiles(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let items = state
        .profiles
        .iter()
        .enumerate()
        .map(|(index, (name, servers))| {
            let marker = if index == state.selected { ">" } else { " " };
            let active = if name == &state.status.active_profile {
                "ACTIVE"
            } else {
                ""
            };
            ListItem::new(format!(
                "{marker} {name:<20} {active:<8} servers={}  preview: {}",
                servers.len(),
                servers.join(", ")
            ))
        });
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title("Profiles — Enter activates")
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_doctor_screen(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let items = state.doctor.checks.iter().map(|check| {
        ListItem::new(format!(
            "{:?} {}\n  REASON: {}\n  AFFECTED: {}\n  CONFIGURATION: {}\n  FIX: {}",
            check.status,
            check.name,
            check.reason,
            check.affected,
            check.configuration.as_deref().unwrap_or("-"),
            check.fix.as_deref().unwrap_or("none")
        ))
    });
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title("Doctor — concrete diagnostic details")
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_exposure(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let mut lines = state
        .exposure_details
        .iter()
        .enumerate()
        .map(|(index, detail)| {
            Line::from(format!(
                "{} {detail}",
                if index == state.selected { ">" } else { " " }
            ))
        })
        .collect::<Vec<_>>();
    lines.extend([
        Line::from(""),
        Line::from("Use `hydian expose plan <provider>` to inspect the exact command first."),
        Line::from(format!(
            "ACTIVE: {}",
            state
                .status
                .active_exposure_provider
                .as_deref()
                .unwrap_or("none")
        )),
    ]);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title("Exposure — plan before launch")
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_logs(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let filtered = state
        .logs
        .iter()
        .filter(|line| {
            state.query.is_empty()
                || line
                    .to_ascii_lowercase()
                    .contains(&state.query.to_ascii_lowercase())
        })
        .collect::<Vec<_>>();
    let start = filtered.len().saturating_sub(100);
    let lines = filtered[start..]
        .iter()
        .map(|line| Line::from(redact_text(line, &[])))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(if state.paused_logs {
                        "Logs — PAUSED (p resumes)"
                    } else {
                        "Logs — live (p pauses, / searches)"
                    })
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

async fn exposure_details() -> Vec<String> {
    use crate::exposure::ExposureProvider;
    let mut details = Vec::new();
    for (name, scope) in [
        ("tailscale", "tailnet or public"),
        ("ngrok", "public HTTPS"),
        ("cloudflare", "quick experimental or existing"),
        ("custom", "operator-supplied command"),
    ] {
        let detection = crate::exposure::CommandProvider::new(name).detect().await;
        details.push(format!(
            "{name:<10} {}  executable={}  version={}  scope={scope}",
            if detection.available {
                "detected"
            } else {
                "missing"
            },
            detection
                .executable
                .as_ref()
                .map_or_else(|| "-".into(), |path| path.display().to_string()),
            detection.version.as_deref().unwrap_or("-")
        ));
    }
    details
}

fn render_help(frame: &mut Frame<'_>, area: Rect, _state: &UiState) {
    frame.render_widget(
        Paragraph::new(
            "Up/Down or j/k move selection\nLeft/Right or h/l move panels\nTab/Shift+Tab change screen\nEnter inspect or activate\nEsc close or cancel\n/ search\n? contextual help\nr restart backend\ns start or stop backend\np pause logs\nq or Ctrl+C request shutdown\n\nRisk topics: `hydian explain non-loopback-without-auth`, `origin-validation`, `plaintext-secrets`, `provider-exposure`.",
        )
        .block(Block::default().title("Help").borders(Borders::ALL))
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    frame.render_widget(
        Paragraph::new(format!(
            "Tab screens  j/k move  Enter inspect  / search  ? help  q quit{}",
            if state.searching {
                format!("  SEARCH: {}", state.query)
            } else {
                String::new()
            }
        )),
        area,
    );
}

fn centered(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height_percent) / 2),
        Constraint::Percentage(height_percent),
        Constraint::Percentage((100 - height_percent) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - width_percent) / 2),
        Constraint::Percentage(width_percent),
        Constraint::Percentage((100 - width_percent) / 2),
    ])
    .split(vertical[1])[1]
}

fn read_logs(paths: &HydianPaths) -> Vec<String> {
    crate::logging::latest_log_file(&paths.logs)
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|content| content.lines().map(ToOwned::to_owned).collect())
        .unwrap_or_default()
}

struct TerminalGuard {
    mouse: bool,
    restored: bool,
}

impl TerminalGuard {
    fn enter(mouse: bool) -> Result<Self> {
        enable_raw_mode().context("could not enable terminal raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, Hide)?;
        if mouse {
            execute!(stdout, EnableMouseCapture)?;
        }
        Ok(Self {
            mouse,
            restored: false,
        })
    }

    fn restore(&mut self) {
        if !self.restored {
            restore_terminal(self.mouse);
            self.restored = true;
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn restore_terminal(mouse: bool) {
    let _ = disable_raw_mode();
    let mut stdout: Stdout = io::stdout();
    if mouse {
        let _ = execute!(stdout, DisableMouseCapture);
    }
    let _ = execute!(stdout, DisableBracketedPaste, Show, LeaveAlternateScreen);
}

fn install_panic_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal(true);
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::{Screen, UiState, default_launch_allowed_for, render};
    use crate::{
        config::HydianConfig,
        diagnostics::DoctorReport,
        runtime::{GatewayState, RuntimeStatus},
    };
    use chrono::Utc;
    use ratatui::{Terminal, backend::TestBackend};

    fn state() -> UiState {
        let config = HydianConfig::default();
        UiState::new(
            &config,
            RuntimeStatus {
                schema_version: 1,
                generated_at: Utc::now(),
                state: GatewayState::Degraded,
                ready: true,
                degraded: true,
                endpoint: "http://127.0.0.1:7337/mcp".into(),
                active_profile: "default".into(),
                ready_backends: vec!["alpha".into()],
                unavailable_backends: vec!["broken".into()],
                tool_count: 1,
                tools: Vec::new(),
                backends: Vec::new(),
                active_exposure_provider: None,
            },
            Vec::new(),
            DoctorReport {
                ready: true,
                strict: false,
                checks: Vec::new(),
            },
        )
    }

    fn buffer_text(state: &UiState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, state)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .fold(String::new(), |mut output, cell| {
                output.push_str(cell.symbol());
                output
            })
    }

    #[test]
    fn overview_renders_degraded_label_and_endpoint() {
        let text = buffer_text(&state(), 100, 30);
        assert!(text.contains("DEGRADED"));
        assert!(text.contains("127.0.0.1:7337/mcp"));
    }

    #[test]
    fn ascii_and_monochrome_modes_retain_text_labels() {
        let mut state = state();
        state.symbols = crate::config::SymbolMode::Ascii;
        state.monochrome = true;
        assert!(buffer_text(&state, 100, 30).contains("[!] DEGRADED"));
    }

    #[test]
    fn narrow_terminal_has_clear_fallback() {
        assert!(buffer_text(&state(), 62, 15).contains("at least 80 columns"));
    }

    #[test]
    fn footer_shortcuts_are_stable() {
        assert!(buffer_text(&state(), 100, 30).contains("Tab screens"));
    }

    #[test]
    fn server_selection_is_rendered() {
        let mut state = state();
        state.screen = Screen::Servers;
        assert!(buffer_text(&state, 100, 30).contains("Servers"));
    }

    #[test]
    fn search_filtering_matches_backend_and_description() {
        let mut state = state();
        state.tools.push(crate::routing::ToolSummary {
            qualified_name: "alpha__echo".into(),
            original_name: "echo".into(),
            backend: "alpha".into(),
            description: Some("copies input".into()),
            available: true,
            input_schema: serde_json::json!({"type": "object"}),
        });
        state.query = "copies".into();
        assert_eq!(state.filtered_tools().len(), 1);
    }

    #[test]
    fn profile_preview_screen_is_present() {
        let mut state = state();
        state.screen = Screen::Profiles;
        assert!(buffer_text(&state, 100, 30).contains("preview"));
    }

    #[test]
    fn diagnostic_detail_screen_names_required_fields() {
        let mut state = state();
        state.screen = Screen::Doctor;
        let text = buffer_text(&state, 100, 30);
        assert!(text.contains("concrete diagnostic details"));
    }

    #[test]
    fn exposure_plan_confirmation_language_is_explicit() {
        let mut state = state();
        state.screen = Screen::Exposure;
        assert!(buffer_text(&state, 100, 30).contains("exact command"));
    }

    #[test]
    fn secret_redaction_applies_to_log_rendering() {
        let mut state = state();
        state.screen = Screen::Logs;
        state.logs = vec!["authorization=secret".into()];
        let text = buffer_text(&state, 100, 30);
        assert!(text.contains("[REDACTED]"));
        assert!(!text.contains("authorization=secret"));
    }

    #[test]
    fn terminal_capability_detection_covers_redirection_plain_and_dumb() {
        assert!(default_launch_allowed_for(true, true, false, Some("xterm")));
        assert!(!default_launch_allowed_for(
            true,
            false,
            false,
            Some("xterm")
        ));
        assert!(!default_launch_allowed_for(true, true, true, Some("xterm")));
        assert!(!default_launch_allowed_for(true, true, false, Some("dumb")));
    }
}
