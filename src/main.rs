use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    prelude::*,
    widgets::*,
    Terminal,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display, Formatter},
    fs::read_dir,
    io::{self, stdout, ErrorKind},
    path::{Path, PathBuf},
    process::exit,
    time::Duration,
};
use tokio::fs::{create_dir_all, write};

#[warn(clippy::all)]
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
struct SetupOutput {
    config: Config,
    file_idx: u32,
}
#[derive(Serialize, Deserialize, Clone)]
struct Config {
    dlp_path: String,
    timeout: u64,
}
impl Default for Config {
    fn default() -> Self {
        Config {
            dlp_path: DLP_PATH.to_string(),
            timeout: 30,
        }
    }
}

pub fn get_quefi_dir() -> PathBuf {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            eprintln!("Failed to get executable file. {err}");
            exit(1);
        }
    };
    exe.parent().unwrap().join("quefi")
}

async fn count_files_in_dir(dir_path: &str) -> Result<usize, io::Error> {
    let mut count = 0;
    let entries = match read_dir(dir_path) {
        Ok(entries) => entries,
        Err(err) => return Err(err),
    };
    for entry in entries {
        match entry {
            Ok(entry) => {
                if entry.file_type()?.is_file() {
                    count += 1;
                }
            }
            Err(err) => {
                eprintln!("{err}");
                exit(1);
            }
        }
    }
    Ok(count)
}

async fn setup(client: &Client) -> SetupOutput {
    let dir = get_quefi_dir();
    match create_dir_all(dir.join("songs")).await {
        Ok(_) => {}
        Err(err) => {
            if err.kind() != ErrorKind::AlreadyExists {
                eprintln!("{err}");
                exit(1);
            }
        }
    }
    let contents = match tokio::fs::read_to_string(dir.join("config.json")).await {
        Ok(contents) => contents,
        Err(_) => {
            eprintln!("Could not find the config file, creating new one");
            let default = Config::default();
            save_config(&default).await;
            serde_json::to_string(&default).unwrap()
        }
    };
    let config = match serde_json::from_str::<Config>(contents.as_str()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}");
            exit(1);
        }
    };
    if !Path::new(&config.dlp_path).exists() {
        println!("yt-dlp was not found"); // TODO find something else for asking user input
        if true {
            match youtube::get_dlp(client).await {
                Ok(_) => {}
                Err(err) => {
                    eprintln!("{err}");
                    exit(1);
                }
            }
        }
    }
    let file_idx = count_files_in_dir(dir.join("songs").to_str().unwrap()).await;
    match file_idx {
        Ok(file_idx) => SetupOutput {
            config,
            file_idx: file_idx as u32,
        },
        Err(err) => {
            eprintln!("{err}");
            exit(1);
        }
    }
}

async fn save_config(data: &Config) {
    let contents = serde_json::to_string(&data).unwrap();
    let dir = get_quefi_dir();
    match write(dir.join("config.json"), contents).await {
        Ok(_) => {}
        Err(err) => eprintln!("{err}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), io::Error> {
    let mut app = App::default().await;
    let terminal = App::init_terminal()?;
    app.run(terminal)?;
    App::restore_terminal()?;
    Ok(())
}

struct App {
    should_exit: bool,
    client: Client,
    setup_output: SetupOutput,
    list: SongList,
    selection: usize,
    playing_index: Option<usize>,
    log: String,
}
struct Playlist {
    songs: Vec<Song>,
}
#[derive(Debug, Clone)]
struct Song {
    selected: bool,
    name: String,
    playing: bool,
}
#[derive(Clone)]
struct SongList {
    items: Vec<Song>,
    state: ListState,
}

impl App {
    async fn default() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        App {
            should_exit: false,
            client: client.clone(),
            list: SongList {
                items: vec![
                    Song {
                        name: String::from("something"),
                        playing: false,
                        selected: true,
                    },
                    Song {
                        name: String::from("elseif"),
                        playing: false,
                        selected: false,
                    },
                ],
                state: ListState::default(),
            },
            setup_output: setup(&client).await,
            selection: 0,
            playing_index: None,
            log: String::from("Initialized!"),
        }
    }

    fn init_terminal() -> Result<Terminal<impl Backend>, io::Error> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        Ok(terminal)
    }

    fn run(&mut self, mut terminal: Terminal<impl Backend>) -> io::Result<()> {
        while !self.should_exit {
            terminal.draw(|f| f.render_widget(&mut *self, f.size()))?;
            if let Event::Key(key) = event::read()? {
                self.handle_key(key);
            };
        }
        Ok(())
    }

    fn restore_terminal() -> Result<(), io::Error> {
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }

    fn toggle_current(&mut self) {
        if let Some(idx) = self.playing_index {
            if idx == self.selection {
                self.log = format!("Stopped playing music ({idx} idx)");
                self.list.items[self.selection].playing = false;
                self.playing_index = None;
            } else {
                self.log = format!(
                    "Changed to different music ({idx} idx -> {} idx)",
                    self.selection
                );
                self.list.items[idx].playing = false;
                self.list.items[self.selection].playing = true;
                self.playing_index = Some(self.selection);
            }
        } else {
            self.log = format!("Started playing music ({} idx)", self.selection);
            self.list.items[self.selection].playing = true;
            self.playing_index = Some(self.selection);
        }
    }

    fn select_next(&mut self) {
        if self.selection + 1 == self.list.items.len() {
            self.list.items[self.selection].selected = false;
            self.selection = 0;
            self.list.items[self.selection].selected = true;
        } else {
            self.list.items[self.selection].selected = false;
            self.selection += 1;
            self.list.items[self.selection].selected = true;
        }
    }

    fn select_previous(&mut self) {
        if self.selection == 0 {
            self.list.items[self.selection].selected = false;
            self.selection = self.list.items.len() - 1;
            self.list.items[self.selection].selected = true;
        } else {
            self.list.items[self.selection].selected = false;
            self.selection -= 1;
            self.list.items[self.selection].selected = true;
        }
    }

    fn add_song(&mut self) {
        self.log = String::from("Not implemented!");
    }

    fn remove_current_song(&mut self) {
        if self.list.items.len() == 1 {
            self.log = String::from("Cannot remove! List cannot have 0 items!") // TODO: list can have 0 items
        } else {
            self.log = format!("Remove {} idx", self.selection);
            self.list.items.remove(self.selection);
            if self.selection == self.list.items.len() {
                self.selection -= 1;
                self.list.items[self.selection].selected = true;
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_exit = true,
            KeyCode::Char('a') => self.add_song(),
            KeyCode::Char('r') => self.remove_current_song(),
            KeyCode::Down => self.select_next(),
            KeyCode::Up => self.select_previous(),
            KeyCode::Enter => self.toggle_current(),
            _ => {}
        }
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let [header_area, main_area, log_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);
        App::render_header(header_area, buf);
        App::render_list(self, main_area, buf);
        App::render_log(self, log_area, buf);
    }
}

impl App {
    // rendering stuff here
    fn render_list(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::new()
            .title(Line::raw("Song List"))
            .title_bottom(Line::raw("q to exit, a to add, r to remove"))
            .borders(Borders::ALL)
            .border_set(symbols::border::DOUBLE);

        let list = List::new(self.list.items.clone()).block(block);

        StatefulWidget::render(list, area, buf, &mut self.list.state);
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

impl From<Song> for ListItem<'_> {
    fn from(song: Song) -> ListItem<'static> {
        let mut symbols = String::new();
        if song.selected {
            symbols.push_str("💲 ");
        } else {
            symbols.push_str("   "); // 3x space because emojis take up 2x the space a normal letter does
        }
        if song.playing {
            symbols.push_str("🔈 ");
        }
        ListItem::new(format!("{}{}", symbols, song.name))
    }
}
