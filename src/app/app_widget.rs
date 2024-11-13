use crate::app::{App, Cursor, Mode, Playlist, Song};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    symbols::border,
    text::Text,
    widgets::{Block, Paragraph, Widget},
};

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
            App::render_header(header_area, buf);
            self.render_list(main_area, buf);
            self.render_player(player_area, buf);
            self.render_log(log_area, buf);
        }
    }
}

impl App<'_> {
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
            Paragraph::new(concat!(
                "",
                "  q - quit the program",
                "  h - display this text",
                "  enter - play song/playlist",
                "  space - pause song/playlist",
                "  e - enter a playlist (see songs inside)",
                "  a - add song/playlist",
                "  n - remove song/playlist",
                "  f - skip song",
                "  l - add song globally",
                "  d - download video from YouTube as mp3",
                "  o - seek back 5 seconds",
                "  p - seek forward 5 seconds",
                "  up/down - select previous/next item",
                "  left/right - decrease/increase volume",
            ))
            .block(block)
            .render(area, buf);
            return;
        }

        if let Cursor::Playlist(_) | Cursor::NonePlaylist = self.cursor {
            Paragraph::new(
                self.playlists
                    .iter()
                    .map(|playlist| playlist.to_string())
                    .collect::<Vec<String>>()
                    .join("\n"),
            )
            .block(block)
            .render(area, buf);
        } else {
            let mut text = if let Cursor::OnBack(_) = self.cursor {
                Text::from("💲 [Back]".bold())
            } else {
                Text::from("   [Back]".bold())
            };

            self.songs
                .iter()
                .for_each(|song| text.push_line(song.to_string()));

            Paragraph::new(text).block(block).render(area, buf);
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
