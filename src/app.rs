use crate::{
    get_quefi_dir, make_safe_filename,
    spotify::{
        create_token, fetch_playlist_info, fetch_track_info, validate_spotify_link, SpotifyLink,
    },
    youtube::{self, download_song, search_ytmusic},
    Error, SaveData, SearchFor, TaskResult, TaskReturn,
};
use ratatui::{
    backend::Backend,
    crossterm::event::{self, poll, Event, KeyCode, KeyEventKind},
    style::{Style, Stylize},
    symbols::border,
    widgets::{Block, ListState},
    Terminal,
};
use regex::Regex;
use reqwest::Client;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use serde::{Deserialize, Serialize};
use std::{fs::File, io, path::Path, time::Duration};
use tokio::task::JoinHandle;
use tui_textarea::{CursorMove, Input, Key, TextArea};

#[macro_use]
mod macros;

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
    Focused,
    Unfocused,
}

#[derive(Debug, PartialEq)]
enum Playing {
    GlobalSong(usize),
    Playlist(usize),
    Song(usize),
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
    songs: Vec<SerializableSong>,
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
    song_idx: u16,
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

#[derive(Debug)]
enum Download {}

pub(crate) struct App<'a> {
    join_handles: Vec<JoinHandle<Result<TaskReturn, Error>>>,
    global_song_list_state: ListState,
    playlist_list_state: ListState,
    pub(crate) save_data: SaveData,
    config_menu_state: ListState,
    _handle: OutputStreamHandle,
    song_queue: Vec<QueuedSong>,
    song_list_state: ListState,
    download_state: ListState,
    playlists: Vec<Playlist>,
    downloads: Vec<Download>,
    last_queue_length: usize,
    global_songs: Vec<Song>,
    text_area: TextArea<'a>,
    _stream: OutputStream,
    valid_input: bool,
    songs: Vec<Song>,
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

        let (stream, handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&handle).unwrap();

        App {
            _handle: handle,
            _stream: stream,
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
            downloads: Vec::new(),
            playlists: Vec::new(),
            songs: Vec::new(),
            playing: Playing::None,
            log: String::from("Initialized!"),
            mode: Mode::Normal,
            text_area: TextArea::default(),
            valid_input: false,
        }
    }

    pub(crate) async fn run(&mut self, mut terminal: Terminal<impl Backend>) -> io::Result<()> {
        loop {
            terminal.draw(|frame| {
                frame.render_widget(&mut *self, frame.area());
            })?;

            // Force updates every 0.1 seconds
            if poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    match self.mode {
                        Mode::Normal if key.kind == KeyEventKind::Press => match key.code {
                            KeyCode::Char('q') => break,
                            KeyCode::Char('y') => self.help(),
                            KeyCode::Char(' ') => self.pause(),
                            KeyCode::Char('o') => self.seek_back(),
                            KeyCode::Char('p') => self.seek_forward(),
                            KeyCode::Char('a') => self.add_item(),
                            KeyCode::Char('n') => self.remove_current(),
                            KeyCode::Char('r') => self.toggle_repeat(),
                            KeyCode::Char('f') => self.sink.skip_one(),
                            KeyCode::Char('g') => self.window = Window::GlobalSongs,
                            KeyCode::Char('d') => self.window = Window::DownloadManager,
                            KeyCode::Char('c') => self.window = Window::ConfigurationMenu,
                            KeyCode::Char('u') => self.decrease_volume(),
                            KeyCode::Char('i') => self.increase_volume(),
                            KeyCode::Char('h') | KeyCode::Left => self.select_left_window(),
                            KeyCode::Char('l') | KeyCode::Right => self.select_right_window(),
                            KeyCode::Char('j') | KeyCode::Down => self.select_next(),
                            KeyCode::Char('k') | KeyCode::Up => self.select_previous(),
                            KeyCode::Enter => self.play_current(),
                            _ => {}
                        },
                        Mode::Input(_) if key.kind == KeyEventKind::Press => match key.code {
                            KeyCode::Esc => self.exit_input_mode(),
                            KeyCode::Enter => self.submit_input().await,
                            _ => {
                                let input: Input = key.into();
                                if !(input.key == Key::Char('m') && input.ctrl)
                                    && self.text_area.input(key)
                                {
                                    self.validate_input();
                                }
                            }
                        },
                        Mode::Help if key.kind == KeyEventKind::Press => match key.code {
                            KeyCode::Char('y') => self.help(),
                            KeyCode::Char('q') => break,
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
            self.update_song_queue();

            let mut completed_futures = Vec::new();

            for handle in self.join_handles.iter_mut() {
                if handle.is_finished() {
                    completed_futures.push(handle.await.unwrap());
                }
            }

            self.join_handles.retain(|handle| !handle.is_finished());

            for completed_future in completed_futures {
                self.handle_result(completed_future);
            }
        }
        Ok(())
    }

    fn handle_result(&mut self, result: TaskResult) {
        match result {
            Ok(TaskReturn::PlaylistInfo(playlist_info)) => {
                self.log = String::from("Playlist fetched successfully! Starting search...");

                let tracks_len = playlist_info.tracks.len();

                self.save_data.playlists.push(SerializablePlaylist {
                    songs: vec![String::new(); tracks_len],
                    name: playlist_info.name.clone(),
                });

                self.playlists.push(Playlist {
                    songs: vec![
                        SerializableSong {
                            name: String::new(),
                            path: String::new()
                        };
                        tracks_len
                    ],
                    selected: Selected::None,
                    playing: false,
                    name: playlist_info.name.clone(),
                });

                let playlist_idx = self.save_data.playlists.len() - 1;

                for (idx, track) in playlist_info.tracks.into_iter().enumerate() {
                    let client = self.client.clone();

                    self.join_handles.push(tokio::spawn(async move {
                        search_ytmusic(
                            &client,
                            &track.query,
                            SearchFor::Playlist(playlist_idx, track.name, idx),
                        )
                        .await
                    }));
                }
            }
            Ok(TaskReturn::TrackInfo(track_info)) => {
                self.log = String::from("Track fetched successfully! Starting search...");
                let client = self.client.clone();

                self.join_handles.push(tokio::spawn(async move {
                    search_ytmusic(
                        &client,
                        &track_info.query,
                        SearchFor::GlobalSong(track_info.name),
                    )
                    .await
                }));
            }
            Ok(TaskReturn::SearchResult(search_result, search_for)) => {
                self.log = String::from("Search successful!");
                let dlp_path = self.save_data.dlp_path.clone();

                let filename = make_safe_filename(search_for.song_name());

                self.join_handles.push(tokio::spawn(async move {
                    download_song(
                        &dlp_path,
                        &format!("https://youtube.com/watch?v={}", search_result.video_id),
                        &filename,
                        search_for,
                    )
                    .await
                }));
            }
            Ok(TaskReturn::SongDownloaded(SearchFor::Playlist(idx, song_name, song_idx))) => {
                self.log = format!("Song for playlist downloaded: {}", song_name);

                let song = SerializableSong {
                    path: get_quefi_dir()
                        .join("songs")
                        .join(format!("{}.mp3", make_safe_filename(&song_name)))
                        .to_string_lossy()
                        .to_string(),
                    name: song_name.clone(),
                };

                self.global_songs.push(Song {
                    path: song.path.clone(),
                    name: song_name.clone(),
                    playing: false,
                    selected: Selected::None,
                });

                self.save_data.playlists[idx].songs[song_idx] = song_name.clone();
                self.save_data.songs.push(song.clone());

                self.playlists[idx].songs[song_idx] = song;
            }
            Ok(TaskReturn::SongDownloaded(SearchFor::GlobalSong(name))) => {
                self.log = format!("Song downloaded: {}", name);

                let path = get_quefi_dir()
                    .join(make_safe_filename(&name))
                    .to_string_lossy()
                    .to_string();

                self.save_data.songs.push(SerializableSong {
                    path: path.clone(),
                    name: name.clone(),
                });

                self.global_songs.push(Song {
                    path,
                    name,
                    playing: false,
                    selected: Selected::None,
                });
            }
            Ok(TaskReturn::DlpDownloaded) => {}
            Ok(TaskReturn::Token(token, link)) => {
                self.save_data.last_valid_token = token;
                self.action_on_link(link);
            }
            Err(err) => {
                if let Error::SpotifyBadAuth(link) = err {
                    self.recreate_spotify_token(link);
                } else {
                    self.log = err.to_string();
                }
            }
        }
    }

    fn recreate_spotify_token(&mut self, link: SpotifyLink) {
        self.log = String::from("Spotify token expired. Creating a new one...");

        let client_id = self.save_data.spotify_client_id.clone();
        let client_secret = self.save_data.spotify_client_secret.clone();
        let client = self.client.clone();

        self.join_handles.push(tokio::spawn(async move {
            create_token(&client, &client_id, &client_secret, link).await
        }));
    }

    fn update_song_queue(&mut self) {
        if self.sink.len() != self.last_queue_length {
            // if self.repeat == Repeat::One {
            //     // TODO: this
            // }

            if !self.song_queue.is_empty() {
                self.last_queue_length = self.sink.len();
                self.song_queue.remove(0);
            }

            if self.song_queue.is_empty() && self.repeat == Repeat::All {
                if let Playing::Playlist(idx) = self.playing {
                    for song in &self.playlists[idx].songs.clone() {
                        self.play_path(&song.name, &song.path);
                    }

                    self.last_queue_length = self.sink.len();
                }
            }
        }
    }

    fn toggle_repeat(&mut self) {
        self.repeat = match self.repeat {
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::None,
            Repeat::None => Repeat::All,
        }
    }

    fn select_left_window(&mut self) {
        self.focused = Focused::Left;

        match self.window {
            Window::Songs => {
                if let Some(idx) = self.song_list_state.selected() {
                    self.songs[idx].selected = Selected::Unfocused;
                }
            }
            Window::GlobalSongs => {
                if let Some(idx) = self.global_song_list_state.selected() {
                    self.global_songs[idx].selected = Selected::Unfocused;
                }
            }
            Window::DownloadManager => {
                self.log = String::from("DownloadManager TODO! select_left_window")
            }
            Window::ConfigurationMenu => {
                if let Some(idx) = self.config_menu_state.selected() {
                    match idx {
                        0 => self.config.dlp_path.selected = Selected::Unfocused,
                        1 => self.config.spotify_client_id.selected = Selected::Unfocused,
                        2 => self.config.spotify_client_secret.selected = Selected::Unfocused,
                        _ => panic!("Index out of range for config menu"),
                    }
                }
            }
        }

        if let Some(idx) = self.playlist_list_state.selected() {
            self.playlists[idx].selected = Selected::Focused;
        }
    }

    fn select_right_window(&mut self) {
        self.focused = Focused::Right;

        match self.window {
            Window::Songs => {
                if let Some(idx) = self.song_list_state.selected() {
                    self.songs[idx].selected = Selected::Focused;
                }
            }
            Window::GlobalSongs => {
                if let Some(idx) = self.global_song_list_state.selected() {
                    self.global_songs[idx].selected = Selected::Focused;
                }
            }
            Window::DownloadManager => {
                self.log = String::from("DownloadManager TODO! select_right_window")
            }
            Window::ConfigurationMenu => {
                if let Some(idx) = self.config_menu_state.selected() {
                    match idx {
                        0 => self.config.dlp_path.selected = Selected::Focused,
                        1 => self.config.spotify_client_id.selected = Selected::Focused,
                        2 => self.config.spotify_client_secret.selected = Selected::Focused,
                        _ => panic!("Index out of range for config menu"),
                    }
                }
            }
        }

        if let Some(idx) = self.playlist_list_state.selected() {
            self.playlists[idx].selected = Selected::Unfocused;
        }
    }

    fn seek_back(&mut self) {
        if !self.song_queue.is_empty() {
            self.sink
                .try_seek(self.sink.get_pos().saturating_sub(Duration::from_secs(5)))
                .expect("Seeking failed");
        }
    }

    fn seek_forward(&mut self) {
        if !self.song_queue.is_empty() {
            self.sink
                .try_seek(self.sink.get_pos() + Duration::from_secs(5))
                .expect("Seeking failed");
        }
    }

    fn pause(&mut self) {
        if self.sink.is_paused() {
            self.sink.play();
        } else {
            self.sink.pause();
        }
    }

    fn help(&mut self) {
        if self.mode == Mode::Help {
            self.mode = Mode::Normal;
        } else {
            self.mode = Mode::Help;
        }
    }

    fn see_songs_in_playlist(&mut self) {
        if let Some(idx) = self.playlist_list_state.selected() {
            self.songs.clear();

            for song in &self.playlists[idx].songs {
                self.songs.push(Song {
                    selected: Selected::None,
                    name: song.name.clone(),
                    path: song.path.clone(),
                    playing: false,
                });
            }

            self.window = Window::Songs;
            self.song_list_state.select_first();
        }
    }

    fn increase_volume(&mut self) {
        let new_volume = self.sink.volume() + 0.05;
        if new_volume >= 5.05 {
            self.log = String::from("Volume can't be above 500%");
        } else {
            self.sink.set_volume(new_volume);
            self.save_data.last_volume = new_volume;
        }
    }

    fn decrease_volume(&mut self) {
        let new_volume = self.sink.volume() - 0.05;
        if new_volume < 0. {
            self.log = String::from("Volume can't be negative!");
        } else {
            self.sink.set_volume(new_volume);
            self.save_data.last_volume = new_volume;
        }
    }

    fn validate_input(&mut self) {
        match self.mode {
            Mode::Input(InputMode::AddPlaylist) => {
                let text = self.text_area.lines()[0].trim();
                let mut name_exists = false;
                for playlist in &self.save_data.playlists {
                    if playlist.name == text {
                        name_exists = true;
                        break;
                    }
                }

                let bad_input = if text.is_empty() {
                    String::from("Playlist name cannot be empty")
                } else if name_exists {
                    String::from("Playlist name cannot be same as existing playlist's name")
                } else if text.len() > 64 {
                    String::from("Playlist name cannot be longer than 64 characters")
                } else {
                    String::new()
                };

                self.textarea_condition(
                    !text.is_empty() && !name_exists && text.len() <= 64,
                    String::from("Input playlist name"),
                    bad_input,
                );
            }
            Mode::Input(InputMode::AddSongToPlaylist) => {
                let text = self.text_area.lines()[0].trim();
                let mut name_exists = false;
                for song in &self.save_data.songs {
                    if song.name == text {
                        name_exists = true;
                        break;
                    }
                }

                self.textarea_condition(
                    name_exists,
                    String::from("Input song name"),
                    String::from("Song doesn't exist"),
                );
            }
            Mode::Input(InputMode::AddGlobalSong) => {
                let text = self.text_area.lines()[0].trim();
                let mut name_exists = false;
                for song in &self.save_data.songs {
                    if song.name == text {
                        name_exists = true;
                        break;
                    }
                }

                let bad_input = if text.is_empty() {
                    String::from("Song name cannot be empty")
                } else if name_exists {
                    String::from("Song name cannot be same as existing song's name")
                } else if text.len() > 64 {
                    String::from("Song name cannot be longer than 64 characters")
                } else {
                    String::new()
                };

                self.textarea_condition(
                    !text.is_empty() && !name_exists && text.len() <= 64,
                    String::from("Input song name"),
                    bad_input,
                );
            }
            Mode::Input(InputMode::ChooseFile(_)) => {
                let path = Path::new(&self.text_area.lines()[0]);
                // TODO: Symlinks??? More file formats???
                self.textarea_condition(
                    path.exists()
                        && path.is_file()
                        && path.extension().unwrap_or_default() == "mp3",
                    String::from("Input file path"),
                    String::from("File path is not pointing to a mp3 file"),
                )
            }
            Mode::Input(InputMode::DownloadLink) => self.textarea_condition(
                is_valid_youtube_link(&self.text_area.lines()[0])
                    || validate_spotify_link(&self.text_area.lines()[0]) != SpotifyLink::Invalid,
                String::from("Input Spotify/YouTube link"),
                String::from("Invalid Spotify/YouTube link"),
            ),
            Mode::Input(InputMode::GetDlp) => {
                let text = &self.text_area.lines()[0].to_ascii_lowercase();
                self.textarea_condition(
                    text == "y" || text == "n",
                    String::from("Download yt-dlp now?"),
                    String::from("Y/N only"),
                )
            }
            Mode::Input(InputMode::DlpPath) => {
                let path = Path::new(&self.text_area.lines()[0]);

                #[cfg(target_os = "windows")]
                let extension = "exe";

                #[cfg(not(target_os = "windows"))]
                let extension = "";

                self.textarea_condition(
                    path.exists()
                        && path.is_file()
                        && path.extension().unwrap_or_default() == extension,
                    String::from("Input yt-dlp path"),
                    String::from("File path is not pointing to a yt-dlp executable"),
                )
            }
            Mode::Input(InputMode::SpotifyClientId) => self.textarea_condition(
                self.text_area.lines()[0].len() == 32,
                String::from("Input Spotify Client ID"),
                String::from("Invalid Spotify Client ID"),
            ),
            Mode::Input(InputMode::SpotifyClientSecret) => self.textarea_condition(
                self.text_area.lines()[0].len() == 32,
                String::from("Input Spotify Client Secret"),
                String::from("Invalid Spotify Client Secret"),
            ),
            _ => panic!("No input handler implemented for {:?}", self.mode),
        }
    }

    fn textarea_condition(&mut self, condition: bool, title: String, bad_input: String) {
        if condition {
            let block = Block::bordered()
                .title(title)
                .style(Style::default().light_green())
                .border_set(border::THICK);
            self.text_area.set_block(block);
            self.valid_input = true;
        } else {
            let block = Block::bordered()
                .title(title)
                .title_bottom(bad_input)
                .style(Style::default().light_red())
                .border_set(border::THICK);
            self.text_area.set_block(block);
            self.valid_input = false;
        }
    }

    async fn submit_input(&mut self) {
        if !self.valid_input {
            return;
        }
        self.log = String::from("Submitted input");
        match &self.mode {
            Mode::Input(InputMode::AddPlaylist) => {
                let input = &self.text_area.lines()[0];
                let was_empty = self.playlists.is_empty();

                self.save_data.playlists.push(SerializablePlaylist {
                    name: input.clone(),
                    songs: Vec::new(),
                });

                self.playlists.push(Playlist {
                    songs: Vec::new(),
                    selected: Selected::None,
                    playing: false,
                    name: input.clone(),
                });

                if was_empty {
                    select!(self.playlists, self.playlist_list_state, 0);
                    self.see_songs_in_playlist();
                }

                self.exit_input_mode();
            }
            Mode::Input(InputMode::AddSongToPlaylist) => {
                let song_name = self.text_area.lines()[0].clone();
                let was_empty = self.songs.is_empty();

                let mut song_path = String::new();
                for song in &self.save_data.songs {
                    if song.name == song_name {
                        song_path = song.path.clone();
                    }
                }

                let playlist_idx = self.playlist_list_state.selected().unwrap();
                let idx = if let Some(idx) = self.song_list_state.selected() {
                    idx + 1
                } else {
                    0
                };

                self.save_data.playlists[playlist_idx]
                    .songs
                    .insert(idx, song_name.clone());

                self.songs.insert(
                    idx,
                    Song {
                        selected: Selected::None,
                        name: song_name.clone(),
                        path: song_path.clone(),
                        playing: false,
                    },
                );

                self.playlists[playlist_idx].songs.insert(
                    idx,
                    SerializableSong {
                        name: song_name,
                        path: song_path,
                    },
                );

                if was_empty {
                    select!(self.songs, self.song_list_state, 0);
                }

                self.exit_input_mode();
            }
            Mode::Input(InputMode::AddGlobalSong) => {
                let input = self.text_area.lines()[0].clone();
                self.text_area.move_cursor(CursorMove::Head);
                self.text_area.delete_line_by_end();

                self.mode = Mode::Input(InputMode::ChooseFile(input));
                self.validate_input();
            }
            Mode::Input(InputMode::ChooseFile(song_name)) => {
                let input = self.text_area.lines()[0].clone();
                let was_empty = self.global_songs.is_empty();

                self.global_songs.push(Song {
                    selected: Selected::None,
                    name: song_name.clone(),
                    path: input.clone(),
                    playing: false,
                });

                self.save_data.songs.push(SerializableSong {
                    name: song_name.clone(),
                    path: input,
                });

                if was_empty {
                    select!(self.global_songs, self.global_song_list_state, 0);
                }

                self.exit_input_mode();
            }
            Mode::Input(InputMode::DownloadLink) => {
                let link = validate_spotify_link(&self.text_area.lines()[0]);
                self.action_on_link(link);

                self.exit_input_mode();
            }
            Mode::Input(InputMode::GetDlp) => {
                if &self.text_area.lines()[0] == "n" {
                    self.exit_input_mode();
                    return;
                }

                let client = self.client.clone();
                self.join_handles.push(tokio::spawn(async move {
                    youtube::download_dlp(&client).await
                }));
                self.exit_input_mode();
            }
            Mode::Input(InputMode::DlpPath) => {
                let input = self.text_area.lines()[0].clone();
                self.config.dlp_path.value = input.clone();
                self.save_data.dlp_path = input;
                self.exit_input_mode();
            }
            Mode::Input(InputMode::SpotifyClientId) => {
                let input = self.text_area.lines()[0].clone();
                self.config.spotify_client_id.value = input.clone();
                self.save_data.spotify_client_id = input;
                self.exit_input_mode();
            }
            Mode::Input(InputMode::SpotifyClientSecret) => {
                let input = self.text_area.lines()[0].clone();
                self.config.spotify_client_secret.value = input.clone();
                self.save_data.spotify_client_secret = input;
                self.text_area.clear_mask_char();
                self.exit_input_mode();
            }
            _ => unreachable!(),
        }
    }

    fn action_on_link(&mut self, link: SpotifyLink) {
        match link.clone() {
            SpotifyLink::Playlist(id) => {
                if self.save_data.last_valid_token.is_empty() {
                    self.recreate_spotify_token(link);
                    return;
                }

                let last_valid_token = self.save_data.last_valid_token.clone();
                let client = self.client.clone();

                self.join_handles.push(tokio::spawn(async move {
                    fetch_playlist_info(&client, &id, &last_valid_token).await
                }));
            }
            SpotifyLink::Track(id) => {
                if self.save_data.last_valid_token.is_empty() {
                    self.recreate_spotify_token(link);
                    return;
                }

                let last_valid_token = self.save_data.last_valid_token.clone();
                let client = self.client.clone();

                self.join_handles.push(tokio::spawn(async move {
                    fetch_track_info(&client, &id, &last_valid_token).await
                }));
            }
            SpotifyLink::Invalid => {
                let dlp_path = self.save_data.dlp_path.clone();
                let input = self.text_area.lines()[0].clone();

                self.join_handles.push(tokio::spawn(async move {
                    download_song(
                        &dlp_path,
                        &input,
                        &make_safe_filename(&input),
                        SearchFor::GlobalSong(String::from("Song from YT Link")),
                    )
                    .await
                }));
            }
        }
    }

    fn stop_playing_current(&mut self) {
        match self.playing {
            Playing::Playlist(idx) if !self.playlists.is_empty() => {
                self.playlists[idx].playing = false
            }
            Playing::Song(idx) if !self.songs.is_empty() => self.songs[idx].playing = false,
            Playing::GlobalSong(idx) if !self.global_songs.is_empty() => {
                self.global_songs[idx].playing = false
            }
            Playing::None => panic!("Tried to stop playing Playing::None"),
            _ => {}
        }
        self.playing = Playing::None;
        self.song_queue.clear();
        self.sink.stop();
    }

    fn play_current(&mut self) {
        if self.focused == Focused::Left {
            if let Some(idx) = self.playlist_list_state.selected() {
                match self.playing {
                    Playing::Playlist(playing_idx) => {
                        self.stop_playing_current();
                        if playing_idx != idx {
                            self.playlists[idx].playing = true;
                            self.playing = Playing::Playlist(idx);
                            for song in &self.playlists[idx].songs.clone() {
                                self.play_path(&song.name, &song.path);
                            }
                            self.last_queue_length = self.sink.len();
                            self.log = format!("Queue length: {}", self.sink.len());
                            self.sink.play();
                        }
                    }
                    Playing::Song(_) => {
                        self.stop_playing_current();
                        self.playlists[idx].playing = true;
                        self.playing = Playing::Playlist(idx);
                        for song in &self.playlists[idx].songs.clone() {
                            self.play_path(&song.name, &song.path);
                        }
                        self.last_queue_length = self.sink.len();
                        self.log = format!("Queue length: {}", self.sink.len());
                        self.sink.play();
                    }
                    Playing::GlobalSong(_) => {
                        self.stop_playing_current();
                        self.songs[idx].playing = true;
                        self.playing = Playing::Song(idx);
                        self.play_path(
                            &self.songs[idx].name.clone(),
                            &self.songs[idx].path.clone(),
                        );
                        self.last_queue_length = self.sink.len();
                        self.sink.play();
                    }
                    Playing::None => {
                        self.playlists[idx].playing = true;
                        self.playing = Playing::Playlist(idx);
                        self.song_queue.clear();
                        for song in &self.playlists[idx].songs.clone() {
                            self.play_path(&song.name, &song.path);
                        }
                        self.last_queue_length = self.sink.len();
                        self.log = format!("Queue length: {}", self.sink.len());
                        self.sink.play();
                    }
                };
            }
        } else {
            match self.window {
                Window::Songs => {
                    if let Some(idx) = self.song_list_state.selected() {
                        match self.playing {
                            Playing::Playlist(_) => {
                                self.stop_playing_current();
                                self.songs[idx].playing = true;
                                self.playing = Playing::Song(idx);
                                self.play_path(
                                    &self.songs[idx].name.clone(),
                                    &self.songs[idx].path.clone(),
                                );
                                self.last_queue_length = self.sink.len();
                                self.sink.play();
                            }
                            Playing::Song(playing_idx) => {
                                self.stop_playing_current();
                                if playing_idx != idx {
                                    self.songs[idx].playing = true;
                                    self.playing = Playing::Song(idx);
                                    self.play_path(
                                        &self.songs[idx].name.clone(),
                                        &self.songs[idx].path.clone(),
                                    );
                                    self.last_queue_length = self.sink.len();
                                    self.sink.play();
                                }
                            }
                            Playing::GlobalSong(_) => {
                                self.stop_playing_current();
                                self.songs[idx].playing = true;
                                self.playing = Playing::Song(idx);
                                self.play_path(
                                    &self.songs[idx].name.clone(),
                                    &self.songs[idx].path.clone(),
                                );
                                self.last_queue_length = self.sink.len();
                                self.sink.play();
                            }
                            Playing::None => {
                                self.songs[idx].playing = true;
                                self.playing = Playing::Song(idx);
                                self.song_queue.clear();
                                self.play_path(
                                    &self.songs[idx].name.clone(),
                                    &self.songs[idx].path.clone(),
                                );
                                self.last_queue_length = self.sink.len();
                                self.sink.play();
                            }
                        }
                    }
                }
                Window::GlobalSongs => {
                    if let Some(idx) = self.global_song_list_state.selected() {
                        match self.playing {
                            Playing::Playlist(_) => {
                                self.stop_playing_current();
                                self.global_songs[idx].playing = true;
                                self.playing = Playing::GlobalSong(idx);
                                self.play_path(
                                    &self.global_songs[idx].name.clone(),
                                    &self.global_songs[idx].path.clone(),
                                );
                                self.last_queue_length = self.sink.len();
                                self.sink.play();
                            }
                            Playing::Song(_) => {
                                self.stop_playing_current();
                                self.global_songs[idx].playing = true;
                                self.playing = Playing::GlobalSong(idx);
                                self.play_path(
                                    &self.global_songs[idx].name.clone(),
                                    &self.global_songs[idx].path.clone(),
                                );
                                self.last_queue_length = self.sink.len();
                                self.sink.play();
                            }
                            Playing::GlobalSong(playing_idx) => {
                                self.stop_playing_current();
                                if playing_idx != idx {
                                    self.global_songs[idx].playing = true;
                                    self.playing = Playing::GlobalSong(idx);
                                    self.play_path(
                                        &self.global_songs[idx].name.clone(),
                                        &self.global_songs[idx].path.clone(),
                                    );
                                    self.last_queue_length = self.sink.len();
                                    self.sink.play();
                                }
                            }
                            Playing::None => {
                                self.global_songs[idx].playing = true;
                                self.playing = Playing::Song(idx);
                                self.song_queue.clear();
                                self.play_path(
                                    &self.global_songs[idx].name.clone(),
                                    &self.global_songs[idx].path.clone(),
                                );
                                self.last_queue_length = self.sink.len();
                                self.sink.play();
                            }
                        }
                    }
                }
                Window::DownloadManager => {}
                Window::ConfigurationMenu => {
                    if let Some(idx) = self.config_menu_state.selected() {
                        match idx {
                            0 => self.enter_input_mode(InputMode::DlpPath),
                            1 => self.enter_input_mode(InputMode::SpotifyClientId),
                            2 => {
                                self.text_area.set_mask_char('*');

                                self.enter_input_mode(InputMode::SpotifyClientSecret)
                            }
                            _ => panic!("Index out of range for config menu"),
                        }
                    }
                }
            }
        }
    }

    fn select_next(&mut self) {
        if self.focused == Focused::Left {
            select_next!(self.playlists, self.playlist_list_state);
            self.see_songs_in_playlist();
        } else {
            match self.window {
                Window::Songs => {
                    select_next!(self.songs, self.song_list_state);
                }
                Window::GlobalSongs => {
                    select_next!(self.global_songs, self.global_song_list_state);
                }
                Window::DownloadManager => {
                    self.log = String::from("DownloadManager TODO! select_next")
                }
                Window::ConfigurationMenu => {
                    if let Some(idx) = self.config_menu_state.selected() {
                        match idx {
                            0 => {
                                self.config.dlp_path.selected = Selected::None;
                                self.config.spotify_client_id.selected = Selected::Focused;
                                self.config_menu_state.select_next();
                            }
                            1 => {
                                self.config.spotify_client_id.selected = Selected::None;
                                self.config.spotify_client_secret.selected = Selected::Focused;
                                self.config_menu_state.select_next();
                            }
                            2 => {
                                self.config.spotify_client_secret.selected = Selected::None;
                                self.config.dlp_path.selected = Selected::Focused;
                                self.config_menu_state.select_first();
                            }
                            _ => panic!("Index out of range for config menu"),
                        }
                    }
                }
            }
        }
    }

    fn select_previous(&mut self) {
        if self.focused == Focused::Left {
            select_previous!(self.playlists, self.playlist_list_state);
            self.see_songs_in_playlist();
        } else {
            match self.window {
                Window::Songs => {
                    select_previous!(self.songs, self.song_list_state);
                }
                Window::GlobalSongs => {
                    select_previous!(self.global_songs, self.global_song_list_state);
                }
                Window::DownloadManager => {
                    self.log = String::from("DownloadManager TODO! select_previous")
                }
                Window::ConfigurationMenu => {
                    if let Some(idx) = self.config_menu_state.selected() {
                        match idx {
                            0 => {
                                self.config.dlp_path.selected = Selected::None;
                                self.config.spotify_client_secret.selected = Selected::Focused;
                                self.config_menu_state.select_last();
                            }
                            1 => {
                                self.config.spotify_client_id.selected = Selected::None;
                                self.config.dlp_path.selected = Selected::Focused;
                                self.config_menu_state.select_previous();
                            }
                            2 => {
                                self.config.spotify_client_secret.selected = Selected::None;
                                self.config.spotify_client_id.selected = Selected::Focused;
                                self.config_menu_state.select_previous();
                            }
                            _ => panic!("Index out of range for config menu"),
                        }
                    }
                }
            }
        }
    }

    fn play_path(&mut self, song_name: &str, path: &str) {
        // TODO: Actually handle errors
        let file = File::open(path).expect("Failed to open file");
        let source = Decoder::new(file).expect("Failed to decode file");

        if let Some(duration) = source.total_duration() {
            let queued_song = self.song_queue.last();
            if let Some(last_song) = queued_song {
                self.song_queue.push(QueuedSong {
                    name: song_name.to_string(),
                    song_idx: last_song.song_idx + 1,
                    duration,
                });
            } else {
                self.song_queue.push(QueuedSong {
                    name: song_name.to_string(),
                    song_idx: 0,
                    duration,
                });
            }
        } else {
            self.log = String::from("Duration not known for a song in your playlist.");
        }
        self.sink.append(source);
    }

    fn add_item(&mut self) {
        if self.focused == Focused::Right {
            match self.window {
                Window::Songs => self.enter_input_mode(InputMode::AddSongToPlaylist),
                Window::GlobalSongs => self.enter_input_mode(InputMode::AddGlobalSong),
                Window::DownloadManager => self.enter_input_mode(InputMode::DownloadLink),
                Window::ConfigurationMenu => {}
            }
        } else {
            self.enter_input_mode(InputMode::AddPlaylist);
        }
    }

    fn remove_current(&mut self) {
        if self.focused == Focused::Left {
            if let Some(idx) = self.playlist_list_state.selected() {
                self.log = format!("Remove index {idx}");
                self.playlists.remove(idx);
                self.save_data.playlists.remove(idx);

                if let Playing::Playlist(playing_idx) = self.playing {
                    if playing_idx == idx {
                        self.playing = Playing::None;
                    }
                }

                if !self.playlists.is_empty() {
                    if idx == self.playlists.len() {
                        select!(self.playlists, self.playlist_list_state, idx - 1);
                        self.see_songs_in_playlist();
                    } else if idx == 0 {
                        select!(self.playlists, self.playlist_list_state, 0);
                        self.see_songs_in_playlist();
                    }
                }
            }
        } else {
            match self.window {
                Window::Songs => {
                    if let Some(idx) = self.song_list_state.selected() {
                        let playlist_idx = self.playlist_list_state.selected().unwrap();
                        self.log = format!("Remove index {idx}");

                        self.songs.remove(idx);
                        self.playlists[playlist_idx].songs.remove(idx);
                        self.save_data.playlists[playlist_idx].songs.remove(idx);

                        if let Playing::Song(playing_idx) = self.playing {
                            if playing_idx == idx {
                                self.playing = Playing::None;
                            }
                        }

                        if !self.songs.is_empty() {
                            if idx == self.songs.len() {
                                select!(self.songs, self.song_list_state, idx - 1);
                            } else if idx == 0 {
                                select!(self.songs, self.song_list_state, 0);
                            }
                        }
                    }
                }
                Window::GlobalSongs => {
                    if let Some(idx) = self.global_song_list_state.selected() {
                        self.global_songs.remove(idx);
                        self.save_data.songs.remove(idx);

                        if let Playing::GlobalSong(playing_idx) = self.playing {
                            if playing_idx == idx {
                                self.playing = Playing::None;
                            }
                        }

                        if !self.global_songs.is_empty() {
                            if idx == self.global_songs.len() {
                                select!(self.global_songs, self.global_song_list_state, idx - 1);
                            } else if idx == 0 {
                                select!(self.global_songs, self.global_song_list_state, 0);
                            }
                        }
                    }
                }
                Window::DownloadManager => {
                    self.log = String::from("DownloadManager TODO! remove_current")
                }
                Window::ConfigurationMenu => {
                    self.log = String::from("Configuration TODO! remove_current")
                }
            }
        }
    }

    pub(crate) fn init(&mut self) -> io::Result<()> {
        let mut first = true;

        for playlist in &self.save_data.playlists {
            let songs = self
                .save_data
                .songs
                .iter()
                .filter(|song| playlist.songs.contains(&song.name))
                .cloned()
                .collect();

            self.playlists.push(Playlist {
                songs,
                name: playlist.name.clone(),
                selected: if first {
                    Selected::Focused
                } else {
                    Selected::None
                },
                playing: false,
            });

            first = false;
        }

        for song in &self.save_data.songs {
            self.global_songs.push(Song {
                selected: Selected::None,
                name: song.name.clone(),
                path: song.path.clone(),
                playing: false,
            });
        }

        for song in &self.playlists[0].songs {
            self.songs.push(Song {
                selected: Selected::None,
                name: song.name.clone(),
                path: song.path.clone(),
                playing: false,
            });
        }

        if !Path::new(&self.save_data.dlp_path).exists() {
            self.enter_input_mode(InputMode::GetDlp);
        }

        self.sink.set_volume(self.save_data.last_volume);
        Ok(())
    }

    fn enter_input_mode(&mut self, input_mode: InputMode) {
        self.mode = Mode::Input(input_mode);
        self.validate_input();
    }

    fn exit_input_mode(&mut self) {
        self.text_area.move_cursor(CursorMove::Head);
        self.text_area.delete_line_by_end();
        self.mode = Mode::Normal;
    }
}
