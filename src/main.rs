use app::{App, SerializablePlaylist, SerializableSong};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    crossterm::{
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    Terminal,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use spotify::{PlaylistInfo, SpotifyLink, TrackInfo};
use std::{
    fmt::{self, Display, Formatter},
    fs::{create_dir_all, read_to_string, write},
    io::{self, stdout, ErrorKind},
    path::PathBuf,
};
use youtube::SearchResult;

mod app;
mod spotify;
mod youtube;

#[derive(Serialize, Deserialize)]
pub(crate) struct SaveData {
    dlp_path: String,
    last_volume: f32,
    playlists: Vec<SerializablePlaylist>,
    songs: Vec<SerializableSong>,
    spotify_client_id: String,
    spotify_client_secret: String,
    last_valid_token: String,
}

type TaskResult = Result<TaskReturn, Error>;
type DownloadId = u8;

#[derive(Debug)]
pub(crate) enum TaskReturn {
    SearchResult(DownloadId, SearchResult, SearchFor),
    Token(DownloadId, String, SpotifyLink),
    PlaylistInfo(DownloadId, PlaylistInfo),
    SongDownloaded(DownloadId, SearchFor),
    TrackInfo(DownloadId, TrackInfo),
    DlpDownloaded,
}

type PlaylistIdx = usize;
type SongName = String;
type SongIdx = usize;

#[derive(Debug)]
pub(crate) enum SearchFor {
    // TODO: PlaylistIdx may be inaccurate when a new playlist is added, fix would be needed!
    Playlist(PlaylistIdx, SongName, SongIdx),
    GlobalSong(SongName),
}

#[derive(Debug)]
pub(crate) enum Error {
    SpotifyBadAuth(DownloadId, SpotifyLink),
    Http(reqwest::Error),
    Io(std::io::Error),
    YtMusic,
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
            Self::YtMusic => write!(f, "Failed to search YT Music"),
            &Self::SpotifyBadAuth(..) => {
                panic!("Wanted to display Error::SpotifyBadAuth")
            }
        }
    }
}

pub(crate) fn get_quefi_dir() -> PathBuf {
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
                dlp_path: String::new(),
                spotify_client_id: String::new(),
                spotify_client_secret: String::new(),
                playlists: Vec::new(),
                songs: Vec::new(),
                last_valid_token: String::new(),
                last_volume: 0.25,
            };
            save_data(&data);
            return data;
        }
    };
    serde_json::from_str::<SaveData>(&contents).expect("Failed to load save data")
}

pub(crate) fn make_safe_filename(input: &str) -> String {
    let input = Regex::new("[<>:\"/\\\\|?*\u{0000}-\u{001F}\u{007F}\u{0080}-\u{009F}]+")
        .unwrap()
        .replace_all(input.as_ref(), "_");
    let input = Regex::new("^\\.+|\\.+$")
        .unwrap()
        .replace_all(input.as_ref(), "_");

    let mut result = input.into_owned();
    if Regex::new("^(con|prn|aux|nul|com\\d|lpt\\d)$")
        .unwrap()
        .is_match(result.as_str())
    {
        result.push('_');
    }

    result
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
