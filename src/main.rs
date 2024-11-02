use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{
            self,
            poll,
            Event,
            KeyCode,
            KeyEventKind,
            // DisableMouseCapture, EnableMouseCapture, for mouse controls
        },
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    prelude::*,
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Terminal,
};
use reqwest::blocking::Client;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display, Formatter},
    fs::{create_dir_all, read_to_string, write, File},
    io::{self, stdout, ErrorKind},
    path::{Path, PathBuf},
    time::Duration,
    vec,
};
use tui_textarea::{CursorMove, Input, Key, TextArea};

mod youtube;

#[cfg(target_os = "windows")]
pub const DLP_PATH: &str = "yt-dlp.exe";
#[cfg(not(target_os = "windows"))]
pub const DLP_PATH: &str = "yt-dlp";

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Http(reqwest::Error),
    Timeout,
    InvalidJson,
    ReleaseNotFound,
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
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    dlp_path: String,
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
    match create_dir_all(dir.join("songs")) {
        Ok(_) => {}
        Err(err) => {
            if err.kind() != ErrorKind::AlreadyExists {
                panic!("Could not create quefi/songs/ in parent directory of the quefi executable file: {err}");
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
    serde_json::from_str::<SaveData>(&contents).expect("Failed to decode")
}

fn is_valid_youtube_link(url: &str) -> bool {
    let re =
        regex::Regex::new(r"^https?://(www\.)?(youtube\.com/watch\?v=|youtu\.be/)[\w-]{11}(&.*)?$")
            .unwrap();
    re.is_match(url)
}

fn init_terminal() -> Result<Terminal<impl Backend>, io::Error> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    Ok(terminal)
}

fn restore_terminal() -> Result<(), io::Error> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn main() -> Result<(), io::Error> {
    let terminal = init_terminal()?;
    let mut app = App::default();

    app.init()?;
    app.run(terminal)?;

    restore_terminal()?;
    Ok(())
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
struct ItemList {
    playlists: Vec<Playlist>,
    songs: Vec<Song>,
    state: ListState,
}

struct App {
    #[allow(dead_code)]
    handle: OutputStreamHandle,
    #[allow(dead_code)]
    stream: OutputStream,
    current_playlist_index: Option<usize>,
    cursor: Cursor,
    playing_index: Option<usize>,
    song_queue: Vec<QueuedSong>,
    textarea: TextArea<'static>,
    last_queue_length: usize,
    save_data: SaveData,
    should_exit: bool,
    valid_input: bool,
    list: ItemList,
    client: Client,
    log: String,
    sink: Sink,
    mode: Mode,
}
impl App {
    fn default() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        let (stream, handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&handle).unwrap();
        App {
            client,
            handle,
            sink,
            stream,
            current_playlist_index: None,
            last_queue_length: 0,
            song_queue: Vec::new(),
            save_data: load_data(),
            should_exit: false,
            list: ItemList {
                playlists: vec![],
                songs: vec![],
                state: ListState::default(),
            },
            cursor: Cursor::Playlist(0),
            playing_index: None,
            log: String::from("Initialized!"),
            mode: Mode::Normal,
            textarea: TextArea::default(),
            valid_input: false,
        }
    }

    fn run(&mut self, mut terminal: Terminal<impl Backend>) -> io::Result<()> {
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
            if self.sink.len() != self.last_queue_length {
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
            for song in &self.list.playlists[idx].songs {
                self.list.songs.push(Song {
                    selected: false,
                    name: song.name.to_owned(),
                    path: song.path.to_owned(),
                    playing: false,
                });
            }
            self.cursor = Cursor::OnBack;
        } else if let Cursor::Song(_) | Cursor::OnBack = self.cursor {
            self.cursor = Cursor::Playlist(0);
            self.list.songs.clear();
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
            // Mode::Input(InputMode::AddSong) => {
            //     let text = self.textarea.lines()[0].trim();
            //     let mut name_exists = false;
            //     for song in &self.save_data.songs {
            //         if song.name == text {
            //             name_exists = true;
            //             break;
            //         }
            //     }

            //     let bad_input = if text.is_empty() {
            //         String::from("Song name cannot be empty")
            //     } else if name_exists {
            //         String::from("Song name cannot be same as existing song's name")
            //     } else if text.len() > 64 {
            //         String::from("Song name cannot be longer than 64 characters")
            //     } else {
            //         String::new()
            //     };

            //     self.textarea_condition(
            //         !text.is_empty() && !name_exists && text.len() <= 64,
            //         String::from("Input song name"),
            //         bad_input,
            //     );
            // }
            // Mode::Input(InputMode::ChooseFile) => {
            //     let path = Path::new(&self.textarea.lines()[0]);
            //     // TODO: Symlinks??? More file formats???
            //     self.textarea_condition(
            //         path.exists()
            //             && path.is_file()
            //             && path.extension().unwrap_or_default() == "mp3",
            //         String::from("Input file path"),
            //         String::from("File path is not pointing to a mp3 file"),
            //     )
            // }
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
                .title(Line::raw(title))
                .style(Style::default().light_green())
                .border_set(symbols::border::DOUBLE);
            self.textarea.set_block(block);
            self.valid_input = true;
        } else {
            let block = Block::bordered()
                .title(Line::raw(title))
                .title_bottom(Line::raw(bad_input))
                .style(Style::default().light_red())
                .border_set(symbols::border::DOUBLE);
            self.textarea.set_block(block);
            self.valid_input = false;
        }
    }

    fn submit_input(&mut self) {
        if !self.valid_input {
            return;
        }
        self.log = String::from("Submitted input");
        match self.mode {
            Mode::Input(InputMode::AddPlaylist) => {
                let input = &self.textarea.lines()[0];
                self.list.playlists.push(Playlist {
                    songs: Vec::new(),
                    selected: false,
                    playing: false,
                    name: input.clone(),
                });
                if self.cursor == Cursor::NonePlaylist {
                    self.list.playlists[0].selected = true;
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

                    self.list.songs.insert(
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
            // Mode::Input(InputMode::AddSong) => {
            //     let input = &self.textarea.lines()[0];
            //     self.pending_name = input.to_owned();
            //     self.textarea.move_cursor(CursorMove::Head);
            //     self.textarea.delete_line_by_end();
            //     self.mode = Mode::Input(InputMode::ChooseFile);
            //     self.validate_input();
            // }
            // Mode::Input(InputMode::ChooseFile) => {
            //     let input = &self.textarea.lines()[0];
            //     let name = self.pending_name.to_owned();
            //     let path = input.to_owned();
            //     self.save_data.songs.push(SerializableSong { name, path });
            //     self.list.song_items.push(Song {
            //         name: self.pending_name.to_owned(),
            //         path: input.to_owned(),
            //         selected: false,
            //         playing: false,
            //     });
            //     self.exit_input_mode();
            // }
            Mode::Input(InputMode::DownloadLink) => {
                youtube::download_song(&self.save_data.config.dlp_path, &self.textarea.lines()[0])
                    .unwrap();
                self.exit_input_mode();
            }
            Mode::Input(InputMode::GetDlp) => {
                if &self.textarea.lines()[0] == "n" {
                    self.exit_input_mode();
                    return;
                }
                youtube::download_dlp(&self.client).unwrap();
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
                        self.list.playlists[idx].playing = false;
                        self.playing_index = None;
                        self.song_queue.clear();
                        self.sink.stop();
                    } else {
                        self.log = format!(
                            "Changed to different playlist (idx {playing_idx} -> idx {idx})"
                        );
                        self.list.playlists[playing_idx].playing = false;
                        self.list.playlists[idx].playing = true;
                        self.playing_index = Some(idx);
                        self.song_queue.clear();
                        self.sink.stop();
                        for song in self.list.playlists[idx].songs.clone() {
                            self.play_path(song.name, &song.path);
                        }
                        self.last_queue_length = self.sink.len();
                        self.log = format!("Queue length: {}", self.sink.len());
                        self.sink.play();
                    }
                } else {
                    self.list.playlists[idx].playing = true;
                    self.playing_index = Some(idx);
                    self.song_queue.clear();
                    for song in &self.list.playlists[idx].songs.to_owned() {
                        self.play_path(song.name.clone(), &song.path);
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
                        self.list.songs[idx].playing = false;
                        self.playing_index = None;
                        self.song_queue.clear();
                        self.sink.stop();
                    } else {
                        self.log =
                            format!("Changed to different music (idx {playing_idx} -> idx {idx})");
                        self.list.songs[playing_idx].playing = false;
                        self.list.songs[idx].playing = true;
                        self.playing_index = Some(idx);
                        self.song_queue.clear();
                        self.sink.stop();
                        self.play_path(
                            self.list.songs[idx].name.clone(),
                            &self.list.songs[idx].path.to_owned(),
                        );
                        self.last_queue_length = self.sink.len();
                        self.sink.play();
                    }
                } else {
                    self.log = format!("Started playing music (idx {})", idx);
                    self.list.songs[idx].playing = true;
                    self.playing_index = Some(idx);
                    self.song_queue.clear();
                    self.play_path(
                        self.list.songs[idx].name.clone(),
                        &self.list.songs[idx].path.to_owned(),
                    );
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
                if idx + 1 == self.list.playlists.len() {
                    self.list.playlists[idx].selected = false;
                    self.cursor = Cursor::Playlist(0);
                    self.list.playlists[0].selected = true;
                } else {
                    self.list.playlists[idx].selected = false;
                    self.cursor = Cursor::Playlist(idx + 1);
                    self.list.playlists[idx + 1].selected = true;
                }
            }
            Cursor::Song(idx) => {
                if idx + 1 == self.list.songs.len() {
                    self.list.songs[idx].selected = false;
                    self.cursor = Cursor::OnBack;
                } else {
                    self.list.songs[idx].selected = false;
                    self.cursor = Cursor::Song(idx + 1);
                    self.list.songs[idx + 1].selected = true;
                }
            }
            Cursor::OnBack => {
                if !self.list.songs.is_empty() {
                    self.cursor = Cursor::Song(0);
                    self.list.songs[0].selected = true;
                }
            }
            _ => {}
        }
    }

    fn select_previous(&mut self) {
        match self.cursor {
            Cursor::Playlist(idx) => {
                if idx == 0 {
                    self.list.playlists[idx].selected = false;
                    let new_index = self.list.playlists.len() - 1;
                    self.cursor = Cursor::Playlist(new_index);
                    self.list.playlists[new_index].selected = true;
                } else {
                    self.list.playlists[idx].selected = false;
                    self.cursor = Cursor::Playlist(idx - 1);
                    self.list.playlists[idx - 1].selected = true;
                }
            }
            Cursor::Song(idx) => {
                if idx == 0 {
                    self.list.songs[idx].selected = false;
                    self.cursor = Cursor::OnBack;
                } else {
                    self.list.songs[idx].selected = false;
                    self.cursor = Cursor::Song(idx - 1);
                    self.list.songs[idx - 1].selected = true;
                }
            }
            Cursor::OnBack => {
                if !self.list.songs.is_empty() {
                    let new_selection = self.list.songs.len() - 1;
                    self.cursor = Cursor::Song(new_selection);
                    self.list.songs[new_selection].selected = true;
                }
            }
            _ => {}
        }
    }

    fn play_path(&mut self, song_name: String, path: &str) {
        // TODO: Actually handle errors
        let file = File::open(path).expect("Failed to open file");
        let source = Decoder::new(file).expect("Failed to decode file");
        if let Some(duration) = source.total_duration() {
            let queued_song = self.song_queue.last();
            if let Some(last_song) = queued_song {
                self.song_queue.push(QueuedSong {
                    name: song_name,
                    song_idx: last_song.song_idx + 1,
                    duration,
                });
            } else {
                self.song_queue.push(QueuedSong {
                    name: song_name,
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
            self.list.playlists.remove(idx);
            self.save_data.playlists.remove(idx);
            if let Some(playing_idx) = self.playing_index {
                if playing_idx == idx {
                    self.playing_index = None;
                }
            }
            if self.list.playlists.is_empty() {
                self.cursor = Cursor::NonePlaylist;
            } else if idx == self.list.playlists.len() {
                self.cursor = Cursor::Playlist(idx - 1);
                self.list.playlists[idx - 1].selected = true;
            } else {
                self.list.playlists[idx - 1].selected = true;
            }
        } else if let Cursor::Song(idx) = self.cursor {
            self.log = format!("Remove idx {}", idx);
            self.list.songs.remove(idx);
            self.list.playlists[self.current_playlist_index.unwrap()]
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

            if self.list.songs.is_empty() {
                self.cursor = Cursor::OnBack;
            } else if idx == self.list.songs.len() {
                self.cursor = Cursor::Song(idx - 1);
                self.list.songs[idx - 1].selected = true;
            } else {
                self.list.songs[idx - 1].selected = true;
            }
        } else {
            self.log = String::from("Can't remove!");
        }
    }

    fn init(&mut self) -> io::Result<()> {
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
            self.list.playlists.push(Playlist {
                songs,
                name: playlist.name.to_owned(),
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

impl Widget for &mut App {
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

impl App {
    fn render_input(&mut self, area: Rect, buf: &mut Buffer) {
        self.textarea.render(area, buf);
    }

    fn render_player(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .title(Line::raw("Player"))
            .border_set(symbols::border::DOUBLE);

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

        let title: String;
        let num: String;
        if !self.song_queue.is_empty() {
            title = self.song_queue[0].name.clone();
            let song_idx = self.song_queue[0].song_idx;
            if song_idx < 10 {
                num = format!("0{song_idx}");
            } else {
                num = song_idx.to_string();
            }
        } else {
            title = String::new();
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
            .title(Line::raw("List"))
            .title_bottom(Line::raw(content))
            .border_set(symbols::border::DOUBLE);

        if self.mode == Mode::Help {
            let list = List::new(vec![
                "",
                "  q - quit the program",
                "  h - display this text",
                "  enter - play song/playlist",
                "  space - pause song/playlist",
                "  e - enter a playlist (see songs inside)",
                "  a - add song/playlist",
                "  r - remove song/playlist",
                "  f - skip song",
                "  d - download video from YouTube as mp3",
                "  o - seek back 5 seconds",
                "  p - seek forward 5 seconds",
                "  up/down - select previous/next item",
                "  left/right - decrease/increase volume",
            ])
            .block(block);
            StatefulWidget::render(list, area, buf, &mut self.list.state);
            return;
        }

        if let Cursor::Playlist(_) | Cursor::NonePlaylist = self.cursor {
            let list = List::new(self.list.playlists.to_owned()).block(block);
            StatefulWidget::render(list, area, buf, &mut self.list.state);
        } else {
            let mut items: Vec<ListItem> = self
                .list
                .songs
                .iter()
                .map(|song| ListItem::from(song.to_owned()))
                .collect();
            if self.cursor == Cursor::OnBack {
                items.insert(0, ListItem::new("💲 [Back]".bold()));
            } else {
                items.insert(0, ListItem::new("   [Back]".bold()));
            }
            let list = List::new(items).block(block);
            StatefulWidget::render(list, area, buf, &mut self.list.state);
        }
    }

    fn render_log(&mut self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.log.to_owned())
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

impl From<Song> for ListItem<'_> {
    fn from(song: Song) -> ListItem<'static> {
        let mut prefix = String::new();
        if song.selected {
            prefix.push_str("💲 ");
        } else {
            prefix.push_str("   "); // 3x space because emojis take up 2x the space a normal letter does
        }
        if song.playing {
            prefix.push_str("🔈 ");
        }
        ListItem::new(format!("{}{}", prefix, song.name))
    }
}

impl From<Playlist> for ListItem<'_> {
    fn from(playlist: Playlist) -> ListItem<'static> {
        let mut prefix = String::new();
        if playlist.selected {
            prefix.push_str("💲 ");
        } else {
            prefix.push_str("   "); // 3x space because emojis take up 2x the space a normal letter does
        }
        if playlist.playing {
            prefix.push_str("🔈 ");
        }
        ListItem::new(format!("{}{}", prefix, playlist.name))
    }
}
