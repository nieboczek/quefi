use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    prelude::*,
    widgets::*,
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

#[warn(clippy::shadow_unrelated)]
#[warn(clippy::shadow_same)]
#[warn(clippy::shadow_reuse)]
#[warn(clippy::exit)]
#[warn(clippy::unwrap_used)]
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
            Self::ReleaseNotFound => write!(f, "Correct release not found"),
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

fn save_data(data: &Savedata) {
    let contents = serde_json::to_string(&data).unwrap();
    let dir = get_quefi_dir();
    write(dir.join("data.json"), contents).unwrap();
}

fn load_data() -> Savedata {
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
            let data = Savedata {
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
    serde_json::from_str::<Savedata>(&contents).expect("Failed to decode")
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

enum PlayingType {
    Playlist,
    Song,
}
#[derive(PartialEq, Eq)]
enum Mode {
    Normal,
    Input,
}
enum InputMode {
    DownloadLink,
    ChooseFile,
    AddSong,
    GetDlp,
    None,
}

#[derive(Serialize, Deserialize)]
struct Savedata {
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

struct ItemList {
    playlist_items: Vec<Playlist>,
    song_items: Vec<Song>,
    state: ListState,
}

struct App {
    playing_type: Option<PlayingType>,
    playing_index: Option<usize>,
    textarea: TextArea<'static>,
    #[allow(dead_code)]
    handle: OutputStreamHandle,
    playlist_selection: usize,
    song_selection: usize,
    song_length: Duration,
    input_mode: InputMode,
    pending_name: String,
    #[allow(dead_code)]
    stream: OutputStream,
    playlist_mode: bool,
    savedata: Savedata,
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
            savedata: load_data(),
            should_exit: false,
            playlist_mode: true,
            list: ItemList {
                playlist_items: vec![],
                song_items: vec![],
                state: ListState::default(),
            },
            playlist_selection: 0,
            song_selection: 0,
            song_length: Duration::from_secs(0),
            playing_index: None,
            playing_type: None,
            log: String::from("Initialized!"),
            mode: Mode::Normal,
            input_mode: InputMode::None,
            textarea: TextArea::default(),
            valid_input: false,
            pending_name: String::new(),
        }
    }

    fn run(&mut self, mut terminal: Terminal<impl Backend>) -> io::Result<()> {
        while !self.should_exit {
            terminal.draw(|frame| {
                frame.render_widget(&mut *self, frame.size());
            })?;
            if let Event::Key(key) = event::read()? {
                match self.mode {
                    Mode::Normal if key.kind == KeyEventKind::Press => match key.code {
                        KeyCode::Char('q') => self.save_and_exit(),
                        KeyCode::Char('a') => self.add(),
                        KeyCode::Char('d') => self.download_link(),
                        KeyCode::Char('r') => self.remove_current(),
                        KeyCode::Left => self.decrease_volume(),
                        KeyCode::Right => self.increase_volume(),
                        KeyCode::Down => self.select_next(),
                        KeyCode::Up => self.select_previous(),
                        KeyCode::Enter => self.play_current(),
                        KeyCode::Char(' ') => self.space(),
                        _ => {}
                    },
                    Mode::Input if key.kind == KeyEventKind::Press => match key.code {
                        KeyCode::Esc => self.exit_input_mode(),
                        KeyCode::Enter => self.sumbit_input(),
                        _ => {
                            let input: Input = key.into();
                            if !(input.key == Key::Char('m') && input.ctrl)
                                && self.textarea.input(key)
                            {
                                self.validate_input();
                            }
                        }
                    },
                    Mode::Normal => {}
                    Mode::Input => {}
                }
            }
        }
        Ok(())
    }

    fn space(&mut self) {
        self.log = String::from("You clicked space!");
    }

    fn increase_volume(&mut self) {
        let volume = self.sink.volume();
        if volume >= 5. {
            self.log = String::from("Volume can't be above 500%");
            return;
        }
        self.sink.set_volume(volume + 0.05);
    }

    fn decrease_volume(&mut self) {
        let volume = self.sink.volume();
        // I love when computers fail with 8 digit accuracy
        if volume <= 0.00000001 {
            self.log = String::from("Volume can't be negative!");
            return;
        }
        self.sink.set_volume(volume - 0.05);
    }

    fn save_and_exit(&mut self) {
        save_data(&self.savedata);
        self.should_exit = true;
    }

    fn validate_input(&mut self) {
        match self.input_mode {
            InputMode::AddSong => {
                let text = self.textarea.lines()[0].trim();
                let mut name_exists = false;
                for song in &self.savedata.songs {
                    if song.name == text {
                        name_exists = true;
                        break;
                    }
                }

                let mut bad_input = String::new();
                if text.is_empty() {
                    bad_input = String::from("Song name cannot be empty");
                } else if name_exists {
                    bad_input = String::from("Song name cannot be same as existing song's name");
                } else if text.len() > 64 {
                    bad_input = String::from("Song name cannot be longer than 64 characters");
                }

                self.textarea_condition(
                    !text.is_empty() && !name_exists && text.len() <= 64,
                    String::from("Input song name"),
                    bad_input,
                );
            }
            InputMode::ChooseFile => {
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
            InputMode::DownloadLink => self.textarea_condition(
                is_valid_youtube_link(&self.textarea.lines()[0]),
                String::from("Input YouTube link"),
                String::from("Invalid YouTube link"),
            ),
            InputMode::GetDlp => {
                let text = &self.textarea.lines()[0].to_ascii_lowercase();
                self.textarea_condition(
                    text == "y" || text == "n" || text == "yes" || text == "no",
                    String::from("Download yt-dlp now?"),
                    String::from("Y/N only"),
                )
            }
            InputMode::None => unreachable!(),
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

    fn sumbit_input(&mut self) {
        if !self.valid_input {
            return;
        }
        self.log = String::from("Sumbitted input");
        match self.input_mode {
            InputMode::AddSong => {
                let input = &self.textarea.lines()[0];
                self.pending_name = input.to_owned();
                self.textarea.move_cursor(CursorMove::Head);
                self.textarea.delete_line_by_end();
                self.input_mode = InputMode::ChooseFile;
                self.validate_input();
            }
            InputMode::ChooseFile => {
                let input = &self.textarea.lines()[0];
                let name = self.pending_name.to_owned();
                let path = input.to_owned();
                self.savedata.songs.push(SerializableSong { name, path });
                self.list.song_items.push(Song {
                    name: self.pending_name.to_owned(),
                    path: input.to_owned(),
                    selected: false,
                    playing: false,
                });
                self.exit_input_mode();
            }
            InputMode::DownloadLink => {
                youtube::download(
                    self.savedata.config.dlp_path.to_owned(),
                    &self.textarea.lines()[0],
                )
                .unwrap();
                self.exit_input_mode();
            }
            InputMode::GetDlp => {
                if &self.textarea.lines()[0] == "n" || &self.textarea.lines()[0] == "no" {
                    self.exit_input_mode();
                    return;
                }
                youtube::get_dlp(&self.client).unwrap();
                self.exit_input_mode();
            }
            InputMode::None => unreachable!(),
        }
    }

    fn download_link(&mut self) {
        self.enter_input_mode(InputMode::DownloadLink);
    }

    fn play_current(&mut self) {
        if self.playlist_mode {
            if let Some(idx) = self.playing_index {
                if idx == self.playlist_selection {
                    self.log = format!("Stopped playing playlist (idx {idx})");
                    self.list.playlist_items[self.playlist_selection].playing = false;
                    self.playing_index = None;
                    self.playing_type = None;
                    self.sink.stop();
                } else {
                    self.log = format!(
                        "Changed to different playlist (idx {idx} -> idx {})",
                        self.playlist_selection
                    );
                    self.list.playlist_items[idx].playing = false;
                    self.list.playlist_items[self.playlist_selection].playing = true;
                    self.playing_index = Some(self.playlist_selection);
                    self.playing_type = Some(PlayingType::Playlist);
                    self.sink.stop();
                    for song in self.list.playlist_items[self.playlist_selection].songs.to_owned() {
                        self.play_path(&song.path);
                    }
                    self.log = format!("queue: {}", self.sink.len());
                    self.sink.play();
                }
            } else {
                self.list.playlist_items[self.playlist_selection].playing = true;
                self.playing_index = Some(self.playlist_selection);
                self.playing_type = Some(PlayingType::Playlist);
                for song in self.list.playlist_items[self.playlist_selection].songs.to_owned() {
                    self.play_path(&song.path);
                }
                self.log = format!("Queue: {}", self.sink.len());
                self.sink.play();
            }
        } else {
            if let Some(idx) = self.playing_index {
                if idx == self.song_selection {
                    self.log = format!("Stopped playing music (idx {idx})");
                    self.list.song_items[self.song_selection].playing = false;
                    self.playing_index = None;
                    self.playing_type = None;
                    self.sink.stop();
                } else {
                    self.log = format!(
                        "Changed to different music (idx {idx} -> idx {})",
                        self.song_selection
                    );
                    self.list.song_items[idx].playing = false;
                    self.list.song_items[self.song_selection].playing = true;
                    self.playing_index = Some(self.song_selection);
                    self.playing_type = Some(PlayingType::Song);
                    self.sink.stop();
                    self.play_path(&self.list.song_items[self.song_selection].path.to_owned());
                    self.sink.play();
                }
            } else {
                self.log = format!("Started playing music (idx {})", self.song_selection);
                self.list.song_items[self.song_selection].playing = true;
                self.playing_index = Some(self.song_selection);
                self.playing_type = Some(PlayingType::Song);
                self.play_path(&self.list.song_items[self.song_selection].path.to_owned());
                self.sink.play();
            }
        }
    }

    fn select_next(&mut self) {
        if self.playlist_mode {
            if self.playlist_selection + 1 == self.list.playlist_items.len() {
                self.list.playlist_items[self.playlist_selection].selected = false;
                self.playlist_selection = 0;
                self.list.playlist_items[self.playlist_selection].selected = true;
            } else {
                self.list.playlist_items[self.playlist_selection].selected = false;
                self.playlist_selection += 1;
                self.list.playlist_items[self.playlist_selection].selected = true;
            }
        } else {
            if self.song_selection + 1 == self.list.song_items.len() {
                self.list.song_items[self.song_selection].selected = false;
                self.song_selection = 0;
                self.list.song_items[self.song_selection].selected = true;
            } else {
                self.list.song_items[self.song_selection].selected = false;
                self.song_selection += 1;
                self.list.song_items[self.song_selection].selected = true;
            }
        }
    }

    fn select_previous(&mut self) {
        if self.playlist_mode {
            if self.playlist_selection == 0 {
                self.list.playlist_items[self.playlist_selection].selected = false;
                self.playlist_selection = self.list.playlist_items.len() - 1;
                self.list.playlist_items[self.playlist_selection].selected = true;
            } else {
                self.list.playlist_items[self.playlist_selection].selected = false;
                self.playlist_selection -= 1;
                self.list.playlist_items[self.playlist_selection].selected = true;
            }
        } else {
            if self.song_selection == 0 {
                self.list.song_items[self.song_selection].selected = false;
                self.song_selection = self.list.song_items.len() - 1;
                self.list.song_items[self.song_selection].selected = true;
            } else {
                self.list.song_items[self.song_selection].selected = false;
                self.song_selection -= 1;
                self.list.song_items[self.song_selection].selected = true;
            }
        }
    }

    fn play_path(&mut self, path: &str) {
        let file = File::open(path).unwrap();
        let source = Decoder::new(file).unwrap();
        self.song_length = source.total_duration().unwrap();
        self.sink.append(source);
    }

    fn add(&mut self) {
        self.log = String::from("Changed mode to input");
        self.enter_input_mode(InputMode::AddSong);
    }

    fn remove_current(&mut self) {
        if self.playlist_mode {
            if self.list.playlist_items.len() == 1 {
                // TODO: list can have 0 items
                self.log = String::from("Cannot remove! List cannot have 0 items!")
            } else {
                self.log = format!("Remove idx {}", self.playlist_selection);
                self.list.playlist_items.remove(self.playlist_selection);
                self.savedata.playlists.remove(self.playlist_selection);
                if let Some(idx) = self.playing_index {
                    if idx == self.playlist_selection {
                        self.playing_index = None;
                    }
                }
                if self.playlist_selection == self.list.playlist_items.len() {
                    self.playlist_selection -= 1;
                    self.list.playlist_items[self.playlist_selection].selected = true;
                } else {
                    self.list.playlist_items[self.playlist_selection].selected = true;
                }
            }
        } else {
            if self.list.song_items.len() == 1 {
                // TODO: list can have 0 items
                self.log = String::from("Cannot remove! List cannot have 0 items!")
            } else {
                self.log = format!("Remove idx {}", self.song_selection);
                self.list.song_items.remove(self.song_selection);
                self.savedata.songs.remove(self.song_selection);
                if let Some(idx) = self.playing_index {
                    if idx == self.song_selection {
                        self.playing_index = None;
                    }
                }
                if self.song_selection == self.list.song_items.len() {
                    self.song_selection -= 1;
                    self.list.song_items[self.song_selection].selected = true;
                } else {
                    self.list.song_items[self.song_selection].selected = true;
                }
            }
        }
    }

    fn init(&mut self) -> io::Result<()> {
        self.sink.set_volume(0.3); // For testing purposes
        let mut first = true;
        for playlist in &self.savedata.playlists {
            let songs = self
                .savedata
                .songs
                .iter()
                .filter(|song| playlist.songs.contains(&song.name))
                .cloned()
                .collect();
            self.list.playlist_items.push(Playlist {
                songs,
                name: playlist.name.to_owned(),
                selected: first,
                playing: false,
            });
            first = false;
        }
        if !Path::new(&self.savedata.config.dlp_path).exists() {
            self.enter_input_mode(InputMode::GetDlp);
        }
        Ok(())
    }

    fn enter_input_mode(&mut self, input_mode: InputMode) {
        self.input_mode = input_mode;
        self.mode = Mode::Input;
        self.validate_input();
    }

    fn exit_input_mode(&mut self) {
        self.textarea.move_cursor(CursorMove::Head);
        self.textarea.delete_line_by_end();
        self.input_mode = InputMode::None;
        self.mode = Mode::Normal;
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.mode == Mode::Input {
            let [header_area, main_area, input_area, player_area, log_area] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(3),
                Constraint::Length(3),
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
    // rendering stuff here
    fn render_input(&mut self, area: Rect, buf: &mut Buffer) {
        self.textarea.widget().render(area, buf);
    }

    fn render_player(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .title(Line::raw("Player"))
            .border_set(symbols::border::DOUBLE);
        const PLAY: &str = "▶️";
        const PAUSE: &str = "⏸️";
        let remaining = self.song_length.saturating_sub(self.sink.get_pos());
        let float = if self.song_length.as_secs_f32() != 0.0 {
            remaining.as_secs_f32() / self.song_length.as_secs_f32()
        } else {
            1.0
        };
        let title = "title";
        let num = "01";
        Paragraph::new(format!(
            "{num} {title}{}🔈{:.0}% {}\n{PAUSE} {}",
            " ".repeat((area.as_size().width - 22 - title.len() as u16) as usize),
            self.sink.volume() * 100.,
            "=".repeat((self.sink.volume() * 10.) as usize),
            "=".repeat(((area.as_size().width - 6) as f32 * (1.0 - float)) as usize),
        ))
        .block(block)
        .render(area, buf);
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        let mut content = "q - quit   a - add   r - remove   d - download yt video as mp3";
        if self.mode == Mode::Input {
            if self.valid_input {
                content = "Esc - discard & exit input mode   Enter - sumbit input";
            } else {
                content = "Esc - discard & exit input mode";
            }
        }
        let block = Block::bordered()
            .title(Line::raw("List"))
            .title_bottom(Line::raw(content))
            .border_set(symbols::border::DOUBLE);

        if self.playlist_mode {
            let list = List::new(self.list.playlist_items.to_owned()).block(block);
            StatefulWidget::render(list, area, buf, &mut self.list.state);
        } else {
            let list = List::new(self.list.song_items.to_owned()).block(block);
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
