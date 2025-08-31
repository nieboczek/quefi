use crate::{SaveData, TaskResult};
use ratatui::widgets::ListState;
use regex::Regex;
use reqwest::Client;
use rodio::{OutputStream, OutputStreamBuilder, Sink};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use tokio::task::JoinHandle;
use tui_textarea::TextArea;

#[macro_use]
mod macros;

mod imp;
mod widget;

fn is_valid_youtube_link(url: &str) -> bool {
    let re = Regex::new(r"^https?://(www\.)?(youtube\.com/watch\?v=|youtu\.be/)[\w-]{11}(&.*)?$")
        .unwrap();
    re.is_match(url)
}

#[derive(Debug, PartialEq)]
enum Mode {
    Input(InputMode),
    Normal,
    Help,
}

#[derive(Debug, PartialEq)]
enum InputMode {
    DownloadLink,
    AddPlaylist,
    AddSongToPlaylist,
    ChooseFile(String),
    AddGlobalSong,
    GetDlp,
    DlpPath,
    SpotifyClientId,
    SpotifyClientSecret,
}

#[derive(Debug, PartialEq)]
enum Focused {
    Right,
    Left,
}

#[derive(Debug, PartialEq)]
enum Window {
    Songs,
    GlobalSongs,
    ConfigurationMenu,
    DownloadManager,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Selected {
    None,
    Moving,
    Focused,
    Unfocused,
}

type PlaylistSongIdx = usize;

#[derive(Debug, PartialEq)]
enum Playing {
    GlobalSong(usize),
    Playlist(usize, PlaylistSongIdx),
    None,
}

#[derive(Debug, PartialEq)]
enum Repeat {
    None,
    All,
    One,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SerializablePlaylist {
    songs: Vec<String>,
    name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct SerializableSong {
    name: String,
    path: String,
}

#[derive(Debug, Clone)]
struct Playlist {
    songs: Vec<Song>,
    selected: Selected,
    playing: bool,
    name: String,
}

#[derive(Debug, Clone)]
struct Song {
    selected: Selected,
    name: String,
    path: String,
    playing: bool,
}

#[derive(Debug)]
struct QueuedSong {
    name: String,
    song_idx: usize,
    duration: Duration,
}

enum ConfigFieldType {
    SpotifyClientSecret,
    SpotifyClientId,
    DlpPath,
}

struct ConfigField {
    field_type: ConfigFieldType,
    selected: Selected,
    value: String,
}

struct Config {
    spotify_client_secret: ConfigField,
    spotify_client_id: ConfigField,
    dlp_path: ConfigField,
}

type SongQuery = String;
type SongName = String;

#[derive(Debug)]
struct ProcessingPlaylistSongs {
    searching_songs: Vec<SongName>,
    downloading_songs: Vec<SongName>,
    total_to_download: usize,
    total_to_search: usize,
    playlist_name: String,
    downloaded: u16,
    searched: u16,
}

#[derive(Debug)]
enum Download {
    ProcessingPlaylistSongs(ProcessingPlaylistSongs),
    SearchingForSong(SongQuery),
    DownloadingSong(SongName),
    DownloadingYoutubeSong,
    FetchingSpotifyToken,
    FetchingPlaylistInfo,
    FetchingTrackInfo,
    Empty,
}

pub(crate) struct App<'a> {
    _keep_alive: OutputStream,
    join_handles: Vec<JoinHandle<TaskResult>>,
    global_song_list_state: ListState,
    downloads: HashMap<u8, Download>,
    playlist_list_state: ListState,
    pub(crate) save_data: SaveData,
    config_menu_state: ListState,
    song_queue: Vec<QueuedSong>,
    song_list_state: ListState,
    download_state: ListState,
    playlists: Vec<Playlist>,
    last_queue_length: usize,
    global_songs: Vec<Song>,
    text_area: TextArea<'a>,
    valid_input: bool,
    playing: Playing,
    focused: Focused,
    config: Config,
    client: Client,
    window: Window,
    repeat: Repeat,
    log: String,
    sink: Sink,
    mode: Mode,
}

impl App<'_> {
    pub(crate) fn new(data: SaveData) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();

        let mut stream = OutputStreamBuilder::open_default_stream().unwrap();
        let sink = Sink::connect_new(stream.mixer());
        
        stream.log_on_drop(false);

        App {
            _keep_alive: stream,
            client,
            sink,
            config: Config {
                dlp_path: ConfigField {
                    field_type: ConfigFieldType::DlpPath,
                    value: data.dlp_path.clone(),
                    selected: Selected::Unfocused,
                },
                spotify_client_id: ConfigField {
                    field_type: ConfigFieldType::SpotifyClientId,
                    value: data.spotify_client_id.clone(),
                    selected: Selected::None,
                },
                spotify_client_secret: ConfigField {
                    field_type: ConfigFieldType::SpotifyClientSecret,
                    value: data.spotify_client_secret.clone(),
                    selected: Selected::None,
                },
            },
            repeat: Repeat::None,
            window: Window::Songs,
            download_state: ListState::default().with_selected(Some(0)),
            playlist_list_state: ListState::default().with_selected(Some(0)),
            global_song_list_state: ListState::default().with_selected(Some(0)),
            song_list_state: ListState::default().with_selected(Some(0)),
            config_menu_state: ListState::default().with_selected(Some(0)),
            focused: Focused::Left,
            last_queue_length: 0,
            save_data: data,
            join_handles: Vec::new(),
            song_queue: Vec::new(),
            global_songs: Vec::new(),
            downloads: HashMap::new(),
            playlists: Vec::new(),
            playing: Playing::None,
            log: String::from("Initialized!"),
            mode: Mode::Normal,
            text_area: TextArea::default(),
            valid_input: false,
        }
    }
}
