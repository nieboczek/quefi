use app::{App, SerializablePlaylist, SerializableSong};
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
    fs::{create_dir_all, read_to_string, write},
    io::{self, stdout, ErrorKind},
    path::PathBuf,
};

mod app;
mod spotify;
mod youtube;

#[cfg(target_os = "windows")]
pub const DLP_EXECUTABLE_NAME: &str = "yt-dlp.exe";
#[cfg(not(target_os = "windows"))]
pub const DLP_EXECUTABLE_NAME: &str = "yt-dlp";

#[derive(Serialize, Deserialize)]
pub(crate) struct SaveData {
    config: Config,
    playlists: Vec<SerializablePlaylist>,
    songs: Vec<SerializableSong>,
}

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

fn save_data(data: &SaveData) {
    let contents = serde_json::to_string(&data).unwrap();
    let dir = get_quefi_dir();
    write(dir.join("data.json"), contents).unwrap();
}

fn load_data() -> SaveData {
    let dir = get_quefi_dir();
    if let Err(err) = create_dir_all(dir.join("songs")) {
        if err.kind() != ErrorKind::AlreadyExists {
            panic!("Could not create quefi/songs/ in the directory of the quefi executable file: {err}");
        }
    }
    let contents = match read_to_string(dir.join("data.json")) {
        Ok(contents) => contents,
        Err(err) => {
            if err.kind() != ErrorKind::NotFound {
                panic!("Could not read quefi/data.json: {err}");
            }
            let data = SaveData {
                config: Config {
                    dlp_path: String::new(),
                    spotify_client_id: String::new(),
                    spotify_client_secret: String::new(),
                },
                playlists: Vec::new(),
                songs: Vec::new(),
            };
            save_data(&data);
            return data;
        }
    };
    serde_json::from_str::<SaveData>(&contents).expect("Failed to load save data")
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
    let mut app = App::new(load_data());

    app.init()?;
    app.run(terminal).await?;

    save_data(&app.save_data);
    restore_terminal()?;
    Ok(())
}
