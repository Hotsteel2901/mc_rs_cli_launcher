//! A terminal-native frontend for mc-launcher.
//!
//! The CLI remains the source of truth: the TUI is an additional action in the
//! same binary, and every long-running operation reuses the CLI/download core.
//! The alternate screen, raw mode and cursor are always restored on quit,
//! suspend, error and main-thread panic.

pub mod app;
pub mod config;
mod input;
mod jobs;
mod theme;
mod ui;

use std::io::{self, IsTerminal, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::terminal::{Clear, ClearType, SetTitle};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::launcher::MinecraftLauncher;
use crate::progress;
use crate::tui::app::{App, AppCommand, Bootstrap};
use crate::tui::jobs::BackendEvent;

static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    active: bool,
}

impl TerminalSession {
    fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|e| format!("Cannot enable raw mode: {e}"))?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide,
            SetTitle("mc-launcher"),
            Clear(ClearType::All)
        ) {
            let _ = disable_raw_mode();
            let _ = execute!(
                stdout,
                Show,
                DisableBracketedPaste,
                LeaveAlternateScreen,
                SetTitle("")
            );
            return Err(format!("Cannot enter alternate screen: {error}"));
        }
        let backend = CrosstermBackend::new(io::stdout());
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                restore_terminal_best_effort();
                return Err(format!("Cannot initialize terminal: {error}"));
            }
        };
        if let Err(error) = terminal.clear() {
            restore_terminal_best_effort();
            return Err(format!("Cannot clear terminal: {error}"));
        }
        Ok(Self {
            terminal,
            active: true,
        })
    }

    fn suspend(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let mut stdout = io::stdout();
        disable_raw_mode().map_err(|e| e.to_string())?;
        execute!(
            stdout,
            Show,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            SetTitle("mc-launcher")
        )
        .map_err(|e| e.to_string())?;
        let _ = stdout.flush();
        self.active = false;
        TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn resume(&mut self) -> Result<(), String> {
        if self.active {
            return Ok(());
        }
        enable_raw_mode().map_err(|e| e.to_string())?;
        self.active = true;
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide,
            Clear(ClearType::All)
        ) {
            return Err(error.to_string());
        }
        self.terminal
            .clear()
            .map_err(|e| format!("Cannot redraw terminal: {e}"))?;
        Ok(())
    }

    fn restore(&mut self) {
        if !self.active {
            let _ = disable_raw_mode();
            return;
        }
        let mut stdout = io::stdout();
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout,
            Show,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            SetTitle("")
        );
        let _ = stdout.flush();
        self.active = false;
        TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore();
    }
}

fn restore_terminal_best_effort() {
    if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut stdout = io::stdout();
    let _ = disable_raw_mode();
    let _ = execute!(
        stdout,
        Show,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        SetTitle("")
    );
    let _ = stdout.flush();
}

/// Run the full-screen frontend until the user quits.
pub fn run(launcher: MinecraftLauncher, bootstrap: Bootstrap) -> Result<(), String> {
    if !terminal_available() {
        return Err(
            "The TUI needs an interactive terminal with ANSI color support. \
             Use the regular CLI commands instead."
                .into(),
        );
    }

    let (tx, rx) = jobs::channel();
    let sink_tx = tx.clone();
    install_progress_sink(sink_tx);

    let main_thread = std::thread::current().id();
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let on_main = std::thread::current().id() == main_thread;
        if on_main {
            restore_terminal_best_effort();
        }
        // While the alternate screen is active, a background thread panic
        // must not print over the UI. The job runner already turns caught
        // panics into visible error events.
        if on_main || !TERMINAL_ACTIVE.load(Ordering::SeqCst) {
            previous_hook(info);
        }
    }));

    crate::log::set_quiet(true);
    let mut session = match TerminalSession::enter() {
        Ok(session) => session,
        Err(error) => {
            crate::log::set_quiet(false);
            progress::clear_sink();
            progress::reset_cancel();
            let _ = panic::take_hook();
            return Err(error);
        }
    };
    let mut app = App::new(launcher, tx.clone(), bootstrap);
    let run_result = event_loop(&mut session, &mut app, &rx, &tx);
    if run_result.is_err() {
        // Error paths may bypass the normal quit cleanup in `event_loop`.
        progress::request_cancel();
        progress::set_job(0);
        jobs::kill_active_child();
        std::thread::sleep(Duration::from_millis(180));
    }
    session.restore();
    crate::log::set_quiet(false);
    progress::clear_sink();
    progress::reset_cancel();
    let _ = panic::take_hook();

    match run_result {
        Ok(()) => {
            println!();
            println!("  mc-launcher TUI closed · CLI is still available with --help");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn install_progress_sink(tx: jobs::EventSender) {
    progress::set_sink(Box::new(move |transfer| {
        let job_id = progress::current_job();
        if job_id != 0 {
            let _ = tx.send(BackendEvent::Transfer {
                id: job_id,
                transfer,
            });
        }
    }));
}

fn event_loop(
    session: &mut TerminalSession,
    app: &mut App,
    rx: &std::sync::mpsc::Receiver<BackendEvent>,
    tx: &jobs::EventSender,
) -> Result<(), String> {
    let mut suspended: Option<app::SuspendTask> = None;
    loop {
        app.tick();
        for _ in 0..256 {
            match rx.try_recv() {
                Ok(event) => app.handle_backend(event),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
        if app.should_quit() {
            break;
        }

        session
            .terminal
            .draw(|frame| ui::render(frame, app))
            .map_err(|e| format!("Cannot draw TUI: {e}"))?;

        let frame_wait = if app.settings.reduced_motion {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(33)
        };
        if event::poll(frame_wait).map_err(|e| format!("Input error: {e}"))? {
            match event::read().map_err(|e| format!("Input error: {e}"))? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Release {
                        if let AppCommand::Suspend(task) = app.handle_key(key) {
                            suspended = Some(task);
                        }
                    }
                }
                Event::Paste(text) => app.handle_paste(&text),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if let Some(task) = suspended.take() {
            progress::clear_sink();
            session.suspend()?;
            crate::log::set_quiet(false);
            let result = app.perform_suspended(task);
            crate::log::set_quiet(true);
            session.resume()?;
            install_progress_sink(tx.clone());
            app.after_suspended(task, result);
            app.tick();
        }
    }
    // Let cancellation reach an in-flight worker before the process exits so
    // temporary download files are dropped and child process trees are gone.
    let wait_for_job = app.job.as_ref().is_some_and(|job| job.is_running());
    progress::request_cancel();
    progress::set_job(0);
    jobs::kill_active_child();
    if wait_for_job {
        let deadline = Instant::now() + Duration::from_millis(1200);
        while Instant::now() < deadline {
            match rx.try_recv() {
                Ok(BackendEvent::Finished { id, .. })
                    if app.job.as_ref().is_some_and(|job| job.id == id) =>
                {
                    break;
                }
                Ok(_) => {}
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }
    std::thread::sleep(Duration::from_millis(60));
    Ok(())
}

/// True when the process appears to own an interactive terminal.
pub fn terminal_available() -> bool {
    let is_tty = io::stdout().is_terminal() && io::stdin().is_terminal();
    if !is_tty {
        return false;
    }
    // `TERM=dumb` advertises no cursor-addressing or color support; drawing the
    // UI there would only spray escape codes across the screen.
    if !cfg!(windows) && std::env::var("TERM").is_ok_and(|value| value.eq_ignore_ascii_case("dumb"))
    {
        return false;
    }
    true
}
