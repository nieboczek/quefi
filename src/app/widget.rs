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
            self.textarea.render(input_area, buf);
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
            .border_set(border::THICK);

        StatefulWidget::render(
            List::new(&self.playlists).block(block),
            area,
            buf,
            &mut self.playlist_list_state,
        );
    }

    fn render_player(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Player").border_set(border::THICK);

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

        Paragraph::new(format!(
            "{num} {title}{}{repeat_symbol} 🔈{:.0}% {}\n{pause_symbol} {}",
            // Spaces until other information won't fit
            " ".repeat((area.as_size().width - 25 - title.len() as u16) as usize), //prev: - 24 -
            // Volume percentage
            self.sink.volume() * 100.,
            // Volume
            "━".repeat((self.sink.volume() * 10.) as usize),
            // Song progress
            "━".repeat(((area.as_size().width - 6) as f32 * (1. - remaining_time)) as usize),
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
            .border_set(border::THICK);

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
                    List::new(&self.downloads).block(block),
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

impl From<&Playlist> for ListItem<'_> {
    fn from(value: &Playlist) -> Self {
        let mut prefix = match value.selected {
            Selected::None => String::from("   "),
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
    fn from(_value: &Download) -> Self {
        ListItem::from("")
    }
}

impl From<&ConfigField> for ListItem<'_> {
    fn from(field: &ConfigField) -> Self {
        let prefix = match field.selected {
            Selected::None => String::from("   "),
            Selected::Focused => String::from("►  "),
            Selected::Unfocused => String::from("⇨  "),
        };

        let name = match field.field_type {
            ConfigFieldType::DlpPath => "DLP path: ",
            ConfigFieldType::SpotifyClientId => "Spotify client ID: ",
            ConfigFieldType::SpotifyClientSecret => "Spotify client secret: ",
        };

        let value = match field.field_type {
            ConfigFieldType::DlpPath => &field.value,
            ConfigFieldType::SpotifyClientId => &field.value,
            ConfigFieldType::SpotifyClientSecret => "********************************",
        };

        ListItem::from(format!("{}{}{}", prefix, name, value))
    }
}
