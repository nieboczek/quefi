use crate::{
    spotify::{validate_spotify_link, SpotifyLink},
    youtube::{self, download_song},
    Error, SaveData,
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

#[derive(PartialEq)]
enum Mode {
    Input(InputMode),
    Normal,
    Help,
}

#[derive(PartialEq)]
enum InputMode {
    DownloadLink,
    AddPlaylist,
    AddSongToPlaylist,
    ChooseFile(String),
    AddGlobalSong,
    GetDlp,
}

#[derive(PartialEq)]
enum Focused {
    Right,
    Left,
}

/// ## Possibly just use GlobalSongs instead of None in `App::default`
#[derive(PartialEq)]
enum Window {
    Songs,
    GlobalSongs,
    DownloadManager,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Selected {
    None,
    Focused,
    Unfocused,
}

#[derive(PartialEq)]
enum Playing {
    GlobalSong(usize),
    Playlist(usize),
    Song(usize),
    None,
}

#[derive(PartialEq)]
enum Repeat {
    All,
    One,
    None,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SerializablePlaylist {
    songs: Vec<String>,
    name: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct SerializableSong {
    name: String,
    path: String,
}

#[derive(Clone)]
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

struct QueuedSong {
    name: String,
    song_idx: u16,
    duration: Duration,
}

pub(crate) struct App<'a> {
    err_join_handle: Option<JoinHandle<Result<(), Error>>>,
    global_song_list_state: ListState,
    playlist_list_state: ListState,
    pub(crate) save_data: SaveData,
    _handle: OutputStreamHandle,
    song_queue: Vec<QueuedSong>,
    song_list_state: ListState,
    playlists: Vec<Playlist>,
    last_queue_length: usize,
    global_songs: Vec<Song>,
    textarea: TextArea<'a>,
    _stream: OutputStream,
    valid_input: bool,
    songs: Vec<Song>,
    playing: Playing,
    focused: Focused,
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
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();

        let (stream, handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&handle).unwrap();

        App {
            _handle: handle,
            _stream: stream,
            client,
            sink,
            repeat: Repeat::None,
            window: Window::Songs,
            playlist_list_state: ListState::default().with_selected(Some(0)),
            global_song_list_state: ListState::default().with_selected(Some(0)),
            song_list_state: ListState::default(),
            focused: Focused::Left,
            err_join_handle: None,
            last_queue_length: 0,
            song_queue: Vec::new(),
            save_data: data,
            global_songs: Vec::new(),
            playlists: Vec::new(),
            songs: Vec::new(),
            playing: Playing::None,
            log: String::from("Initialized!"),
            mode: Mode::Normal,
            textarea: TextArea::default(),
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
                                    && self.textarea.input(key)
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
            if self.err_join_handle.is_some() {
                if let Err(err) = self.err_join_handle.as_mut().unwrap().await.unwrap() {
                    self.log = format!("{}", err);
                }
            }
        }
        Ok(())
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
            Window::DownloadManager => self.log = String::from("tooddddd"),
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
            Window::DownloadManager => self.log = String::from("tood"),
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
        let volume = self.sink.volume();
        if volume >= 5.0 {
            self.log = String::from("Volume can't be above 500%");
            return;
        }
        self.sink.set_volume(volume + 0.05);
    }

    fn decrease_volume(&mut self) {
        let new_volume = self.sink.volume() - 0.05;
        if new_volume < 0. {
            self.log = String::from("Volume can't be negative!");
            self.sink.set_volume(0.);
        } else {
            self.sink.set_volume(new_volume);
        }
    }

    fn validate_input(&mut self) {
        match self.mode {
            Mode::Input(InputMode::AddPlaylist) => {
                let text = self.textarea.lines()[0].trim();
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
                let text = self.textarea.lines()[0].trim();
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
                let text = self.textarea.lines()[0].trim();
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
                let path = Path::new(&self.textarea.lines()[0]);
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
                is_valid_youtube_link(&self.textarea.lines()[0])
                    || validate_spotify_link(&self.textarea.lines()[0]) != SpotifyLink::Invalid,
                String::from("Input Spotify/YouTube link"),
                String::from("Invalid Spotify/YouTube link"),
            ),
            Mode::Input(InputMode::GetDlp) => {
                let text = &self.textarea.lines()[0].to_ascii_lowercase();
                self.textarea_condition(
                    text == "y" || text == "n",
                    String::from("Download yt-dlp now?"),
                    String::from("Y/N only"),
                )
            }
            _ => unreachable!(),
        }
    }

    fn textarea_condition(&mut self, condition: bool, title: String, bad_input: String) {
        if condition {
            let block = Block::bordered()
                .title(title)
                .style(Style::default().light_green())
                .border_set(border::THICK);
            self.textarea.set_block(block);
            self.valid_input = true;
        } else {
            let block = Block::bordered()
                .title(title)
                .title_bottom(bad_input)
                .style(Style::default().light_red())
                .border_set(border::THICK);
            self.textarea.set_block(block);
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
                let input = &self.textarea.lines()[0];
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
                // TODO: Select new playlist if no playlists are there
                self.exit_input_mode();
            }
            Mode::Input(InputMode::AddSongToPlaylist) => {
                let song_name = self.textarea.lines()[0].clone();
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

                self.exit_input_mode();
            }
            Mode::Input(InputMode::AddGlobalSong) => {
                let input = self.textarea.lines()[0].clone();
                self.textarea.move_cursor(CursorMove::Head);
                self.textarea.delete_line_by_end();
                self.mode = Mode::Input(InputMode::ChooseFile(input));
                self.validate_input();
            }
            Mode::Input(InputMode::ChooseFile(song_name)) => {
                let input = self.textarea.lines()[0].clone();
                self.save_data.songs.push(SerializableSong {
                    name: song_name.clone(),
                    path: input,
                });
                self.exit_input_mode();
            }
            Mode::Input(InputMode::DownloadLink) => {
                let link = validate_spotify_link(&self.textarea.lines()[0]);

                match link {
                    SpotifyLink::Playlist(id) => {
                        self.log = format!("not implemented yet SpotifyLink::Playlist({})", id);
                    }
                    SpotifyLink::Track(id) => {
                        self.log = format!("not implemented yet SpotifyLink::Track({})", id);
                    }
                    SpotifyLink::Invalid => {
                        let dlp_path = self.save_data.config.dlp_path.clone();
                        let input = self.textarea.lines()[0].clone();

                        self.err_join_handle = Some(tokio::spawn(async move {
                            download_song(&dlp_path, &input).await
                        }));
                    }
                }
                self.exit_input_mode();
            }
            Mode::Input(InputMode::GetDlp) => {
                if &self.textarea.lines()[0] == "n" {
                    self.exit_input_mode();
                    return;
                }

                let client = self.client.clone();
                self.err_join_handle =
                    Some(tokio::spawn(
                        async move { youtube::download_dlp(&client).await },
                    ));
                self.exit_input_mode();
            }
            _ => unreachable!(),
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
                Window::DownloadManager => {}
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
                Window::DownloadManager => self.log = String::from("tUUUTOOO"),
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
                Window::DownloadManager => self.log = String::from("toooDOOOTOO"),
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

                if idx == self.playlists.len() {
                    self.playlist_list_state.select(Some(idx - 1));
                    self.playlists[idx - 1].selected = Selected::Focused;
                    self.see_songs_in_playlist();
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

                        if idx == self.songs.len() {
                            self.song_list_state.select(Some(idx - 1));
                            self.songs[idx - 1].selected = Selected::Focused;
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

                        if idx == self.global_songs.len() {
                            self.global_song_list_state.select(Some(idx - 1));
                            self.global_songs[idx - 1].selected = Selected::Focused;
                        }
                    }
                }
                Window::DownloadManager => self.log = String::from("totototototo"),
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

        if !Path::new(&self.save_data.config.dlp_path).exists() {
            self.enter_input_mode(InputMode::GetDlp);
        }
        
        self.sink.set_volume(0.25);
        Ok(())
    }

    fn enter_input_mode(&mut self, input_mode: InputMode) {
        self.mode = Mode::Input(input_mode);
        self.validate_input();
    }

    fn exit_input_mode(&mut self) {
        self.textarea.move_cursor(CursorMove::Head);
        self.textarea.delete_line_by_end();
        self.mode = Mode::Normal;
    }
}
