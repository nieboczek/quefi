use ratatui::{
    backend::{Backend, CrosstermBackend},
    crossterm::{
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    Terminal,
};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display, Formatter},
    io::{self, stdout},
    path::PathBuf,
};

mod app;
mod spotify;
mod youtube;

#[cfg(target_os = "windows")]
pub const DLP_EXECUTABLE_NAME: &str = "yt-dlp.exe";
#[cfg(not(target_os = "windows"))]
pub const DLP_EXECUTABLE_NAME: &str = "yt-dlp";

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    dlp_path: String,
    spotify_client_id: String,
    spotify_client_secret: String,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Http(reqwest::Error),
    Timeout,
    InvalidJson,
    ReleaseNotFound,
    YtMusicError,
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::Http(err)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(err) => write!(f, "HTTP Error: {err}"),
            Self::Io(err) => write!(f, "IO Error: {err}"),
            Self::Timeout => write!(f, "Process timed out"),
            Self::InvalidJson => write!(f, "Tried to parse invalid JSON"),
            Self::ReleaseNotFound => write!(f, "Correct release of yt-dlp not found"),
            Self::YtMusicError => write!(f, "Failed to search YT Music"),
        }
    }
}

pub fn get_quefi_dir() -> PathBuf {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => panic!("Failed to get executable file. {err}"),
    };
    exe.parent().unwrap().join("quefi")
}

fn init_terminal() -> io::Result<Terminal<impl Backend>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    Ok(terminal)
}

fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let terminal = init_terminal()?;
    let mut app = app::App::default();

    app.init()?;
    app.run(terminal).await?;

    restore_terminal()?;
    Ok(())
}
