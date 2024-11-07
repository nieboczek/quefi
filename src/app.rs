use crate::{get_quefi_dir, youtube, Config};
use ratatui::{
    backend::Backend,
    buffer::Buffer,
    crossterm::event::{self, poll, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    symbols::border,
    widgets::{Block, Paragraph, Widget},
    Terminal,
};
use reqwest::blocking::Client;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use serde::{Deserialize, Serialize};
use std::{
    fs::{create_dir_all, read_to_string, write, File},
    io::{self, ErrorKind},
    path::Path,
    time::Duration,
};
use tui_textarea::{CursorMove, Input, Key, TextArea};

fn save_data(data: &SaveData) {
    let contents = serde_json::to_string(&data).unwrap();
    let dir = get_quefi_dir();
    write(dir.join("data.json"), contents).unwrap();
}

fn load_data() -> SaveData {
    let dir = get_quefi_dir();
    match create_dir_all(dir.join("songs")) {
        Ok(_) => {}
        Err(err) => {
            if err.kind() != ErrorKind::AlreadyExists {
                panic!("Could not create quefi/songs/ in the directory of the quefi executable file: {err}");
            }
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
                },
                playlists: vec![],
                songs: vec![],
            };
            save_data(&data);
            return data;
        }
    };
    serde_json::from_str::<SaveData>(&contents).expect("Failed to load save data")
}

fn is_valid_youtube_link(url: &str) -> bool {
    let re =
        regex::Regex::new(r"^https?://(www\.)?(youtube\.com/watch\?v=|youtu\.be/)[\w-]{11}(&.*)?$")
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
    AddSong,
    GetDlp,
}

#[derive(PartialEq)]
enum Cursor {
    Playlist(usize),
    NonePlaylist,
    Song(usize),
    OnBack,
}

#[derive(Serialize, Deserialize)]
struct SaveData {
    config: Config,
    playlists: Vec<SerializablePlaylist>,
    songs: Vec<SerializableSong>,
}

#[derive(Serialize, Deserialize)]
struct SerializablePlaylist {
    songs: Vec<String>,
    name: String,
}

#[derive(Clone)]
struct Playlist {
    songs: Vec<SerializableSong>,
    selected: bool,
    playing: bool,
    name: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct SerializableSong {
    name: String,
    path: String,
}

#[derive(Debug, Clone)]
struct Song {
    selected: bool,
    name: String,
    path: String,
    playing: bool,
}

struct QueuedSong {
    name: String,
    song_idx: u16,
    duration: Duration,
}

pub struct App<'app> {
    current_playlist_index: Option<usize>,
    playing_index: Option<usize>,
    _handle: OutputStreamHandle,
    song_queue: Vec<QueuedSong>,
    playlists: Vec<Playlist>,
    last_queue_length: usize,
    textarea: TextArea<'app>,
    _stream: OutputStream,
    save_data: SaveData,
    should_exit: bool,
    valid_input: bool,
    songs: Vec<Song>,
    cursor: Cursor,
    client: Client,
    log: String,
    sink: Sink,
    mode: Mode,
}

impl App<'_> {
    pub fn default() -> Self {
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
            current_playlist_index: None,
            last_queue_length: 0,
            song_queue: Vec::new(),
            save_data: load_data(),
            should_exit: false,
            playlists: Vec::new(),
            songs: Vec::new(),
            cursor: Cursor::Playlist(0),
            playing_index: None,
            log: String::from("Initialized!"),
            mode: Mode::Normal,
            textarea: TextArea::default(),
            valid_input: false,
        }
    }

    pub fn run(&mut self, mut terminal: Terminal<impl Backend>) -> io::Result<()> {
        while !self.should_exit {
            terminal.draw(|frame| {
                frame.render_widget(&mut *self, frame.area());
            })?;
            // Force updates every 0.1 seconds
            if poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    match self.mode {
                        Mode::Normal if key.kind == KeyEventKind::Press => match key.code {
                            KeyCode::Char('q') => self.save_and_exit(),
                            KeyCode::Char('h') => self.help(),
                            KeyCode::Char(' ') => self.pause(),
                            KeyCode::Char('o') => self.seek_back(),
                            KeyCode::Char('p') => self.seek_forward(),
                            KeyCode::Char('e') => self.enter_playlist(),
                            KeyCode::Char('a') => self.add_item(),
                            KeyCode::Char('r') => self.remove_current(),
                            KeyCode::Char('f') => self.sink.skip_one(),
                            KeyCode::Char('l') => self.enter_input_mode(InputMode::AddSong),
                            KeyCode::Char('d') => self.download_link(),
                            KeyCode::Left => self.decrease_volume(),
                            KeyCode::Right => self.increase_volume(),
                            KeyCode::Down => self.select_next(),
                            KeyCode::Up => self.select_previous(),
                            KeyCode::Enter => self.play_current(),
                            _ => {}
                        },
                        Mode::Input(_) if key.kind == KeyEventKind::Press => match key.code {
                            KeyCode::Esc => self.exit_input_mode(),
                            KeyCode::Enter => self.submit_input(),
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
                            KeyCode::Char('h') => self.help(),
                            KeyCode::Char('q') => self.save_and_exit(),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
            if self.sink.len() != self.last_queue_length && !self.song_queue.is_empty() {
                self.last_queue_length = self.sink.len();
                self.song_queue.remove(0);
            }
        }
        Ok(())
    }

    fn seek_back(&mut self) {
        if !self.song_queue.is_empty() {
            self.sink
                .try_seek(self.sink.get_pos() - Duration::from_secs(5))
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

    fn enter_playlist(&mut self) {
        if let Cursor::Playlist(idx) = self.cursor {
            self.current_playlist_index = Some(idx);
            for song in &self.playlists[idx].songs {
                self.songs.push(Song {
                    selected: false,
                    name: song.name.clone(),
                    path: song.path.clone(),
                    playing: false,
                });
            }
            self.cursor = Cursor::OnBack;
        } else if let Cursor::Song(_) | Cursor::OnBack = self.cursor {
            self.cursor = Cursor::Playlist(0);
            self.songs.clear();
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

    fn save_and_exit(&mut self) {
        save_data(&self.save_data);
        self.should_exit = true;
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
            Mode::Input(InputMode::AddSong) => {
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
                is_valid_youtube_link(&self.textarea.lines()[0]),
                String::from("Input YouTube link"),
                String::from("Invalid YouTube link"),
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
                .border_set(border::DOUBLE);
            self.textarea.set_block(block);
            self.valid_input = true;
        } else {
            let block = Block::bordered()
                .title(title)
                .title_bottom(bad_input)
                .style(Style::default().light_red())
                .border_set(border::DOUBLE);
            self.textarea.set_block(block);
            self.valid_input = false;
        }
    }

    fn submit_input(&mut self) {
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
                    selected: false,
                    playing: false,
                    name: input.clone(),
                });
                if self.cursor == Cursor::NonePlaylist {
                    self.playlists[0].selected = true;
                    self.cursor = Cursor::Playlist(0);
                }
                self.exit_input_mode();
            }
            Mode::Input(InputMode::AddSongToPlaylist) => {
                if let Cursor::Song(idx) = self.cursor {
                    let song_name = &self.textarea.lines()[0];
                    self.save_data.playlists[self.current_playlist_index.unwrap()]
                        .songs
                        .insert(idx + 1, song_name.clone());

                    let mut song_path = String::new();
                    for song in &self.save_data.songs {
                        if &song.name == song_name {
                            song_path = song.path.clone();
                        }
                    }

                    self.songs.insert(
                        idx + 1,
                        Song {
                            selected: false,
                            name: song_name.clone(),
                            path: song_path,
                            playing: false,
                        },
                    );
                    self.exit_input_mode();
                }
            }
            Mode::Input(InputMode::AddSong) => {
                let input = (&self.textarea.lines()[0]).clone();
                self.textarea.move_cursor(CursorMove::Head);
                self.textarea.delete_line_by_end();
                self.mode = Mode::Input(InputMode::ChooseFile(input));
                self.validate_input();
            }
            Mode::Input(InputMode::ChooseFile(song_name)) => {
                let input = &self.textarea.lines()[0];
                self.save_data.songs.push(SerializableSong {
                    name: song_name.clone(),
                    path: input.clone(),
                });
                self.exit_input_mode();
            }
            Mode::Input(InputMode::DownloadLink) => {
                youtube::download_song(&self.save_data.config.dlp_path, &self.textarea.lines()[0])
                    .expect("Failed to download a song");
                self.exit_input_mode();
            }
            Mode::Input(InputMode::GetDlp) => {
                if &self.textarea.lines()[0] == "n" {
                    self.exit_input_mode();
                    return;
                }
                youtube::download_dlp(&self.client).expect("Failed to download dlp");
                self.exit_input_mode();
            }
            _ => unreachable!(),
        }
    }

    fn download_link(&mut self) {
        self.enter_input_mode(InputMode::DownloadLink);
    }

    fn play_current(&mut self) {
        match self.cursor {
            Cursor::Playlist(idx) => {
                if let Some(playing_idx) = self.playing_index {
                    if playing_idx == idx {
                        self.log = format!("Stopped playing playlist (idx {idx})");
                        self.playlists[idx].playing = false;
                        self.playing_index = None;
                        self.song_queue.clear();
                        self.sink.stop();
                    } else {
                        self.log = format!(
                            "Changed to different playlist (idx {playing_idx} -> idx {idx})"
                        );
                        self.playlists[playing_idx].playing = false;
                        self.playlists[idx].playing = true;
                        self.playing_index = Some(idx);
                        self.song_queue.clear();
                        self.sink.stop();
                        for song in &self.playlists[idx].songs.clone() {
                            self.play_path(&song.name, &song.path);
                        }
                        self.last_queue_length = self.sink.len();
                        self.log = format!("Queue length: {}", self.sink.len());
                        self.sink.play();
                    }
                } else {
                    self.playlists[idx].playing = true;
                    self.playing_index = Some(idx);
                    self.song_queue.clear();
                    for song in &self.playlists[idx].songs.clone() {
                        self.play_path(&song.name, &song.path);
                    }
                    self.last_queue_length = self.sink.len();
                    self.log = format!("Queue length: {}", self.sink.len());
                    self.sink.play();
                }
            }
            Cursor::Song(idx) => {
                if let Some(playing_idx) = self.playing_index {
                    if playing_idx == idx {
                        self.log = format!("Stopped playing music (idx {idx})");
                        self.songs[idx].playing = false;
                        self.playing_index = None;
                        self.song_queue.clear();
                        self.sink.stop();
                    } else {
                        self.log =
                            format!("Changed to different music (idx {playing_idx} -> idx {idx})");
                        self.songs[playing_idx].playing = false;
                        self.songs[idx].playing = true;
                        self.playing_index = Some(idx);
                        self.song_queue.clear();
                        self.sink.stop();
                        self.play_path(
                            &self.songs[idx].name.clone(),
                            &self.songs[idx].path.clone(),
                        );
                        self.last_queue_length = self.sink.len();
                        self.sink.play();
                    }
                } else {
                    self.log = format!("Started playing music (idx {})", idx);
                    self.songs[idx].playing = true;
                    self.playing_index = Some(idx);
                    self.song_queue.clear();
                    self.play_path(&self.songs[idx].name.clone(), &self.songs[idx].path.clone());
                    self.last_queue_length = self.sink.len();
                    self.sink.play();
                }
            }
            _ => self.enter_playlist(),
        }
    }

    fn select_next(&mut self) {
        match self.cursor {
            Cursor::Playlist(idx) => {
                if idx + 1 == self.playlists.len() {
                    self.playlists[idx].selected = false;
                    self.cursor = Cursor::Playlist(0);
                    self.playlists[0].selected = true;
                } else {
                    self.playlists[idx].selected = false;
                    self.cursor = Cursor::Playlist(idx + 1);
                    self.playlists[idx + 1].selected = true;
                }
            }
            Cursor::Song(idx) => {
                if idx + 1 == self.songs.len() {
                    self.songs[idx].selected = false;
                    self.cursor = Cursor::OnBack;
                } else {
                    self.songs[idx].selected = false;
                    self.cursor = Cursor::Song(idx + 1);
                    self.songs[idx + 1].selected = true;
                }
            }
            Cursor::OnBack => {
                if !self.songs.is_empty() {
                    self.cursor = Cursor::Song(0);
                    self.songs[0].selected = true;
                }
            }
            _ => {}
        }
    }

    fn select_previous(&mut self) {
        match self.cursor {
            Cursor::Playlist(idx) => {
                if idx == 0 {
                    self.playlists[idx].selected = false;
                    let new_index = self.playlists.len() - 1;
                    self.cursor = Cursor::Playlist(new_index);
                    self.playlists[new_index].selected = true;
                } else {
                    self.playlists[idx].selected = false;
                    self.cursor = Cursor::Playlist(idx - 1);
                    self.playlists[idx - 1].selected = true;
                }
            }
            Cursor::Song(idx) => {
                if idx == 0 {
                    self.songs[idx].selected = false;
                    self.cursor = Cursor::OnBack;
                } else {
                    self.songs[idx].selected = false;
                    self.cursor = Cursor::Song(idx - 1);
                    self.songs[idx - 1].selected = true;
                }
            }
            Cursor::OnBack => {
                if !self.songs.is_empty() {
                    let new_selection = self.songs.len() - 1;
                    self.cursor = Cursor::Song(new_selection);
                    self.songs[new_selection].selected = true;
                }
            }
            _ => {}
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
        if let Cursor::Playlist(_) | Cursor::NonePlaylist = self.cursor {
            self.enter_input_mode(InputMode::AddPlaylist);
        } else {
            self.enter_input_mode(InputMode::AddSongToPlaylist);
        }
    }

    fn remove_current(&mut self) {
        if let Cursor::Playlist(idx) = self.cursor {
            self.log = format!("Remove index {idx}");
            self.playlists.remove(idx);
            self.save_data.playlists.remove(idx);
            if let Some(playing_idx) = self.playing_index {
                if playing_idx == idx {
                    self.playing_index = None;
                }
            }
            if self.playlists.is_empty() {
                self.cursor = Cursor::NonePlaylist;
            } else if idx == self.playlists.len() {
                self.cursor = Cursor::Playlist(idx - 1);
                self.playlists[idx - 1].selected = true;
            } else {
                self.playlists[idx - 1].selected = true;
            }
        } else if let Cursor::Song(idx) = self.cursor {
            self.log = format!("Remove idx {}", idx);
            self.songs.remove(idx);
            self.playlists[self.current_playlist_index.unwrap()]
                .songs
                .remove(idx);
            self.save_data.playlists[self.current_playlist_index.unwrap()]
                .songs
                .remove(idx);

            if let Some(playing_idx) = self.playing_index {
                if playing_idx == idx {
                    self.playing_index = None;
                }
            }

            if self.songs.is_empty() {
                self.cursor = Cursor::OnBack;
            } else if idx == self.songs.len() {
                self.cursor = Cursor::Song(idx - 1);
                self.songs[idx - 1].selected = true;
            } else {
                self.songs[idx - 1].selected = true;
            }
        } else {
            self.log = String::from("Can't remove!");
        }
    }

    pub fn init(&mut self) -> io::Result<()> {
        self.sink.set_volume(0.25); // For testing purposes (so my ears don't blow up)
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
                selected: first,
                playing: false,
            });
            first = false;
        }
        if !Path::new(&self.save_data.config.dlp_path).exists() {
            self.enter_input_mode(InputMode::GetDlp);
        }
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

impl Widget for &mut App<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if let Mode::Input(_) = self.mode {
            let [header_area, main_area, input_area, player_area, log_area] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(3),
                Constraint::Length(4),
                Constraint::Length(1),
            ])
            .areas(area);
            App::render_header(header_area, buf);
            self.render_list(main_area, buf);
            self.render_input(input_area, buf);
            self.render_player(player_area, buf);
            self.render_log(log_area, buf);
        } else {
            let [header_area, main_area, player_area, log_area] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(4),
                Constraint::Length(1),
            ])
            .areas(area);
            App::render_header(header_area, buf);
            self.render_list(main_area, buf);
            self.render_player(player_area, buf);
            self.render_log(log_area, buf);
        }
    }
}

impl App<'_> {
    fn render_input(&mut self, area: Rect, buf: &mut Buffer) {
        self.textarea.render(area, buf);
    }

    fn render_player(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Player").border_set(border::DOUBLE);

        let pause_symbol = if self.sink.is_paused() { "||" } else { ">>" };

        let remaining_time = if !self.song_queue.is_empty() {
            let remaining = self.song_queue[0]
                .duration
                .saturating_sub(self.sink.get_pos());
            if self.song_queue[0].duration.as_secs_f32() != 0.0 {
                remaining.as_secs_f32() / self.song_queue[0].duration.as_secs_f32()
            } else {
                1.0
            }
        } else {
            1.0
        };

        let title: &str;
        let num: String;
        if !self.song_queue.is_empty() {
            title = &self.song_queue[0].name;
            let song_idx = self.song_queue[0].song_idx;
            if song_idx < 10 {
                num = format!("0{song_idx}");
            } else {
                num = song_idx.to_string();
            }
        } else {
            title = "";
            num = String::from("XX");
        }

        // Using unicode "═" instead of the normal equal sign, because some fonts like to mess with multiple of equal signs
        Paragraph::new(format!(
            "{num} {title}{}🔈{:.0}% {}\n{pause_symbol} {}",
            // Spaces until sound controls won't fit
            " ".repeat((area.as_size().width - 22 - title.len() as u16) as usize),
            // Volume percentage
            self.sink.volume() * 100.,
            // Volume as "equal signs"
            "═".repeat((self.sink.volume() * 10.) as usize),
            // Song progress as "equal signs"
            "═".repeat(((area.as_size().width - 6) as f32 * (1. - remaining_time)) as usize),
        ))
        .block(block)
        .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        let content = if let Mode::Input(_) = self.mode {
            if self.valid_input {
                "Esc - discard & exit input mode   Enter - submit input"
            } else {
                "Esc - discard & exit input mode"
            }
        } else {
            "q - quit   h - help"
        };

        let block = Block::bordered()
            .title("List")
            .title_bottom(content)
            .border_set(border::DOUBLE);

        if self.mode == Mode::Help {
            let paragraph = Paragraph::new(concat!(
                "",
                "  q - quit the program",
                "  h - display this text",
                "  enter - play song/playlist",
                "  space - pause song/playlist",
                "  e - enter a playlist (see songs inside)",
                "  a - add song/playlist",
                "  r - remove song/playlist",
                "  f - skip song",
                "  l - add song globally",
                "  d - download video from YouTube as mp3",
                "  o - seek back 5 seconds",
                "  p - seek forward 5 seconds",
                "  up/down - select previous/next item",
                "  left/right - decrease/increase volume",
            ))
            .block(block);
            paragraph.render(area, buf);
            return;
        }

        if let Cursor::Playlist(_) | Cursor::NonePlaylist = self.cursor {
            let paragraph = Paragraph::new(
                self.playlists
                    .iter()
                    .map(|playlist| playlist.to_string())
                    .collect::<Vec<String>>()
                    .join("\n"),
            )
            .block(block);

            paragraph.render(area, buf);
        } else {
            let mut items: Vec<String> = self.songs.iter().map(|song| song.to_string()).collect();

            if self.cursor == Cursor::OnBack {
                items.insert(0, "💲 [Back]".bold().to_string());
            } else {
                items.insert(0, "   [Back]".bold().to_string());
            }

            let paragraph = Paragraph::new(items.join("\n"));
            paragraph.render(area, buf);
        }
    }

    fn render_log(&mut self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.log.clone())
            .reversed()
            .render(area, buf);
    }

    fn render_header(area: Rect, buf: &mut Buffer) {
        Paragraph::new(format!("Quefi v{}", env!("CARGO_PKG_VERSION")))
            .bold()
            .centered()
            .render(area, buf);
    }
}

impl ToString for &Song {
    fn to_string(&self) -> String {
        let mut prefix = String::new();
        if self.selected {
            prefix.push_str("💲 ");
        } else {
            prefix.push_str("   "); // 3x space because emojis take up 2x the space a normal letter does
        }
        if self.playing {
            prefix.push_str("🔈 ");
        }
        format!("{}{}", prefix, self.name)
    }
}

impl ToString for &Playlist {
    fn to_string(&self) -> String {
        let mut prefix = String::new();
        if self.selected {
            prefix.push_str("💲 ");
        } else {
            prefix.push_str("   "); // 3x space because emojis take up 2x the space a normal letter does
        }
        if self.playing {
            prefix.push_str("🔈 ");
        }
        format!("{}{}", prefix, self.name)
    }
}
