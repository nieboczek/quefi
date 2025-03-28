use std::time::Duration;

use crate::app::{App, Mode, Playlist, Selected, Song};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    symbols::border,
    widgets::{Block, List, ListItem, Paragraph, StatefulWidget, Widget},
};

use super::{ConfigField, ConfigFieldType, Download, Repeat, Window};

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

            let [playlist_area, main_area] =
                Layout::horizontal([Constraint::Percentage(20), Constraint::Fill(1)])
                    .areas(main_area);

            App::render_header(header_area, buf);
            self.render_playlists(playlist_area, buf);
            self.render_window(main_area, buf);
            self.text_area.render(input_area, buf);
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

            let [playlist_area, main_area] =
                Layout::horizontal([Constraint::Percentage(20), Constraint::Fill(1)])
                    .areas(main_area);

            App::render_header(header_area, buf);
            self.render_playlists(playlist_area, buf);
            self.render_window(main_area, buf);
            self.render_player(player_area, buf);
            self.render_log(log_area, buf);
        }
    }
}

impl App<'_> {
    fn render_playlists(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .title("Playlists")
            .border_set(border::PLAIN);

        StatefulWidget::render(
            List::new(&self.playlists).block(block),
            area,
            buf,
            &mut self.playlist_list_state,
        );
    }

    fn render_player(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Player").border_set(border::PLAIN);

        let repeat_symbol = match self.repeat {
            Repeat::All => "🔁",
            Repeat::One => "🔂",
            Repeat::None => "  ",
        };
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

        let remaining_song_time: Duration;
        let title: &str;
        let num: String;
        if !self.song_queue.is_empty() {
            remaining_song_time = self.song_queue[0]
                .duration
                .saturating_sub(self.sink.get_pos());
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
            remaining_song_time = Duration::from_secs(0);
        }

        let progress_width = area.as_size().width - 11;
        let progress = (progress_width as f32 * (1. - remaining_time)) as usize;
        let mut inverted_progress = (progress_width as f32 * remaining_time) as usize;

        if progress + inverted_progress != progress_width as usize {
            inverted_progress += 1;
        }

        Paragraph::new(format!(
            "{num} {title}{}{repeat_symbol} 🔈{:.0}% {} \n{pause_symbol} {}{} {} ",
            // Spaces until other information won't fit
            " ".repeat((area.as_size().width - 26 - title.len() as u16) as usize),
            // Volume percentage
            self.sink.volume() * 100.,
            // Volume
            "━".repeat((self.sink.volume() * 10.) as usize),
            // Song progress
            "━".repeat(progress),
            // Spaces until remaining time won't fit
            " ".repeat(inverted_progress),
            // Remaining time
            format_duration(remaining_song_time),
        ))
        .block(block)
        .render(area, buf);
    }

    fn render_window(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .title(match self.window {
                Window::Songs => "Songs",
                Window::GlobalSongs => "Global song manager",
                Window::DownloadManager => "Download manager",
                Window::ConfigurationMenu => "Configuration menu",
            })
            .title_bottom("q - quit   y - help")
            .border_set(border::PLAIN);

        if self.mode == Mode::Help {
            Paragraph::new(concat!(
                "",
                "\n  q - quit the program",
                "\n  y - display this text",
                "\n  r - toggle repeating",
                "\n  enter - play song/playlist",
                "\n  space - pause song/playlist",
                "\n  a - add song/playlist",
                "\n  n - remove song/playlist",
                "\n  f - skip song",
                "\n  g - open global song manager",
                "\n  d - open download manager",
                "\n  u/i - decrease/increase volume",
                "\n  o/p - seek backward/forward 5 seconds",
                "\n  left/right - select the left/right window",
                "\n  up/down - select previous/next item",
                "\n",
                "\n  can use h/l to replace left/right",
                "\n  can use k/j to replace up/down",
            ))
            .block(block)
            .render(area, buf);
        } else {
            match self.window {
                Window::Songs => StatefulWidget::render(
                    List::new(&self.songs).block(block),
                    area,
                    buf,
                    &mut self.song_list_state,
                ),
                Window::GlobalSongs => StatefulWidget::render(
                    List::new(&self.global_songs).block(block),
                    area,
                    buf,
                    &mut self.song_list_state,
                ),
                Window::DownloadManager => StatefulWidget::render(
                    List::new(self.downloads.values()).block(block),
                    area,
                    buf,
                    &mut self.download_state,
                ),
                Window::ConfigurationMenu => StatefulWidget::render(
                    List::new([
                        &self.config.dlp_path,
                        &self.config.spotify_client_id,
                        &self.config.spotify_client_secret,
                    ])
                    .block(block),
                    area,
                    buf,
                    &mut self.config_menu_state,
                ),
            }
        }
    }

    fn render_log(&mut self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.log.as_str())
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

fn format_duration(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    let seconds = duration.as_secs() % 60;
    format!("{}:{:02}", minutes, seconds)
}

impl From<&Playlist> for ListItem<'_> {
    fn from(value: &Playlist) -> Self {
        let mut prefix = match value.selected {
            Selected::None => String::from("   "),
            Selected::Moving => String::from("⇅  "),
            Selected::Focused => String::from("►  "),
            Selected::Unfocused => String::from("⇨  "),
        };

        if value.playing {
            prefix.push_str("🔈 ");
        }

        ListItem::from(format!("{}{}", prefix, value.name))
    }
}

impl From<&Song> for ListItem<'_> {
    fn from(value: &Song) -> Self {
        let mut prefix = match value.selected {
            Selected::None => String::from("   "),
            Selected::Moving => String::from("⇅  "),
            Selected::Focused => String::from("►  "),
            Selected::Unfocused => String::from("⇨  "),
        };

        if value.playing {
            prefix.push_str("🔈 ");
        }

        ListItem::from(format!("{}{}", prefix, value.name))
    }
}

impl From<&Download> for ListItem<'_> {
    fn from(value: &Download) -> Self {
        match value {
            Download::ProcessingPlaylistSongs(processing) => ListItem::from(format!(
                "Searching songs for {} ({}/{}):\n{}\nDownloading songs for {} ({}/{}):\n{}",
                processing.playlist_name,
                processing.searched,
                processing.total_to_search,
                {
                    let mut songs = processing
                        .searching_songs
                        .iter()
                        .take(4)
                        .map(|song| format!(" {}", song))
                        .collect::<Vec<_>>();

                    if processing.searching_songs.len() > 4 {
                        songs.push("...".to_string());
                    }

                    songs.join("\n")
                },
                processing.playlist_name,
                processing.downloaded,
                processing.total_to_download,
                {
                    let mut songs = processing
                        .downloading_songs
                        .iter()
                        .take(4)
                        .map(|song| format!(" {}", song))
                        .collect::<Vec<_>>();

                    if processing.downloading_songs.len() > 4 {
                        songs.push("...".to_string());
                    }

                    songs.join("\n")
                },
            )),
            Download::FetchingSpotifyToken => ListItem::from("Fetching Spotify token..."),
            Download::FetchingPlaylistInfo => ListItem::from("Fetching playlist info..."),
            Download::FetchingTrackInfo => ListItem::from("Fetching track info..."),
            Download::SearchingForSong(query) => {
                ListItem::from(format!("Searching for {}...", query))
            }
            Download::DownloadingSong(name) => ListItem::from(format!("Downloading {}...", name)),
            Download::DownloadingYoutubeSong => ListItem::from("Downloading song from YouTube..."),
            Download::Empty => panic!("Tried to display empty download"), // TODO: check if it always crashes
        }
    }
}

impl From<&ConfigField> for ListItem<'_> {
    fn from(value: &ConfigField) -> Self {
        let prefix = match value.selected {
            Selected::None => String::from("   "),
            Selected::Moving => String::from("⇅  "),
            Selected::Focused => String::from("►  "),
            Selected::Unfocused => String::from("⇨  "),
        };

        let name = match value.field_type {
            ConfigFieldType::DlpPath => "DLP path: ",
            ConfigFieldType::SpotifyClientId => "Spotify client ID: ",
            ConfigFieldType::SpotifyClientSecret => "Spotify client secret: ",
        };

        let value = match value.field_type {
            ConfigFieldType::DlpPath => &value.value,
            ConfigFieldType::SpotifyClientId => &value.value,
            ConfigFieldType::SpotifyClientSecret => "********************************",
        };

        ListItem::from(prefix + name + value)
    }
}
