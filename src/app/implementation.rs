use crate::{
    get_quefi_dir, make_safe_filename,
    spotify::{
        create_token, fetch_playlist_info, fetch_track_info, validate_spotify_link, SpotifyLink,
    },
    youtube::{self, download_song, search_ytmusic},
    Error, SearchFor, TaskResult, TaskReturn,
};
use ratatui::{
    backend::Backend,
    crossterm::event::{self, poll, Event, KeyCode, KeyEventKind},
    style::{Style, Stylize},
    symbols::border,
    widgets::Block,
    Terminal,
};
use rodio::{Decoder, Source};
use std::{fs::File, io, path::Path, time::Duration};
use tui_textarea::{CursorMove, Input, Key};

use super::{
    App, Download, Focused, InputMode, Mode, Playing, Playlist, ProcessingPlaylistSongs,
    QueuedSong, Repeat, Selected, SerializablePlaylist, SerializableSong, Song, Window,
};

const PRELOAD_SONG_COUNT: usize = 2;

impl App<'_> {
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
                            KeyCode::Char('m') => self.move_item(),
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
            Ok(TaskReturn::PlaylistInfo(id, playlist_info)) => {
                self.downloads.insert(
                    id,
                    Download::ProcessingPlaylistSongs(ProcessingPlaylistSongs {
                        playlist_name: playlist_info.name.clone(),
                        searching_songs: Vec::new(),
                        downloading_songs: Vec::new(),
                        total_to_search: playlist_info.tracks.len(),
                        total_to_download: 0,
                        downloaded: 0,
                        searched: 0,
                    }),
                );

                let tracks_len = playlist_info.tracks.len();

                self.save_data.playlists.push(SerializablePlaylist {
                    songs: vec![String::new(); tracks_len],
                    name: playlist_info.name.clone(),
                });

                self.playlists.push(Playlist {
                    songs: vec![
                        Song {
                            selected: Selected::None,
                            name: String::new(),
                            path: String::new(),
                            playing: false,
                        };
                        tracks_len
                    ],
                    selected: Selected::None,
                    playing: false,
                    name: playlist_info.name,
                });

                let playlist_idx = self.save_data.playlists.len() - 1;

                for (idx, track) in playlist_info.tracks.into_iter().enumerate() {
                    let client = self.client.clone();

                    if let Download::ProcessingPlaylistSongs(processing) =
                        self.downloads.get_mut(&id).unwrap()
                    {
                        processing.searching_songs.push(track.name.clone());
                    }

                    self.join_handles.push(tokio::spawn(async move {
                        search_ytmusic(
                            id,
                            &client,
                            &track.query,
                            SearchFor::Playlist(playlist_idx, track.name, idx),
                        )
                        .await
                    }));
                }
            }
            Ok(TaskReturn::TrackInfo(id, track_info)) => {
                self.downloads
                    .insert(id, Download::SearchingForSong(track_info.query.clone()));

                let client = self.client.clone();

                self.join_handles.push(tokio::spawn(async move {
                    search_ytmusic(
                        id,
                        &client,
                        &track_info.query,
                        SearchFor::GlobalSong(track_info.name),
                    )
                    .await
                }));
            }
            Ok(TaskReturn::SearchResult(
                id,
                search_result,
                SearchFor::Playlist(idx, song_name, song_idx),
            )) => {
                if let Download::ProcessingPlaylistSongs(processing) =
                    self.downloads.get_mut(&id).unwrap()
                {
                    processing.searching_songs.retain(|song| song != &song_name);
                    processing.downloading_songs.push(song_name.clone());
                    processing.total_to_download += 1;
                    processing.searched += 1;
                } else {
                    panic!("Expected Download::ProcessingPlaylistSongs");
                }

                let filename = make_safe_filename(&song_name);
                let dlp_path = self.save_data.dlp_path.clone();

                self.join_handles.push(tokio::spawn(async move {
                    download_song(
                        id,
                        &dlp_path,
                        &format!("https://youtube.com/watch?v={}", search_result.video_id),
                        &filename,
                        SearchFor::Playlist(idx, song_name, song_idx),
                    )
                    .await
                }));
            }
            Ok(TaskReturn::SearchResult(id, search_result, SearchFor::GlobalSong(song_name))) => {
                self.downloads
                    .insert(id, Download::DownloadingSong(song_name.clone()));

                let filename = make_safe_filename(&song_name);
                let dlp_path = self.save_data.dlp_path.clone();

                self.join_handles.push(tokio::spawn(async move {
                    download_song(
                        id,
                        &dlp_path,
                        &format!("https://youtube.com/watch?v={}", search_result.video_id),
                        &filename,
                        SearchFor::GlobalSong(song_name),
                    )
                    .await
                }));
            }
            Ok(TaskReturn::SongDownloaded(id, SearchFor::Playlist(idx, song_name, song_idx))) => {
                if let Download::ProcessingPlaylistSongs(processing) =
                    self.downloads.get_mut(&id).unwrap()
                {
                    processing
                        .downloading_songs
                        .retain(|song| song != &song_name);
                    processing.downloaded += 1;

                    if processing.downloaded as usize == processing.total_to_search {
                        self.downloads.remove(&id);
                    }
                } else {
                    panic!("Expected Download::ProcessingPlaylistSongs");
                }

                let serializable_song = SerializableSong {
                    path: get_quefi_dir()
                        .join("songs")
                        .join(format!("{}.mp3", make_safe_filename(&song_name)))
                        .to_string_lossy()
                        .to_string(),
                    name: song_name.clone(),
                };

                let song = Song {
                    path: serializable_song.path.clone(),
                    name: song_name.clone(),
                    playing: false,
                    selected: Selected::None,
                };

                self.global_songs.push(song.clone());
                self.save_data.playlists[idx].songs[song_idx] = song_name.clone();
                self.save_data.songs.push(serializable_song.clone());

                self.playlists[idx].songs[song_idx] = song;
            }
            Ok(TaskReturn::SongDownloaded(id, SearchFor::GlobalSong(name))) => {
                self.log = format!("{name} downloaded!");
                self.downloads.remove(&id);

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
            Ok(TaskReturn::Token(id, token, link)) => {
                self.save_data.last_valid_token = token;
                self.handle_link(id, link);
            }
            Err(err) => {
                if let Error::SpotifyBadAuth(id, link) = err {
                    self.recreate_spotify_token(id, link);
                } else {
                    self.log = err.to_string();
                }
            }
        }
    }

    fn recreate_spotify_token(&mut self, id: u8, link: SpotifyLink) {
        self.downloads.insert(id, Download::FetchingSpotifyToken);

        let client_id = self.save_data.spotify_client_id.clone();
        let client_secret = self.save_data.spotify_client_secret.clone();
        let client = self.client.clone();

        self.join_handles.push(tokio::spawn(async move {
            create_token(id, &client, &client_id, &client_secret, link).await
        }));
    }

    fn preload_songs(&mut self, start_idx: usize) {
        let idx = self.playlist_list_state.selected().unwrap();

        let song = self.playlists[idx].songs[start_idx].clone();
        self.play_path(&song.name, &song.path);

        let next_idx = start_idx + 1;

        let song_idx = if next_idx >= self.playlists[idx].songs.len() {
            if self.repeat == Repeat::All {
                0
            } else {
                return;
            }
        } else {
            next_idx
        };

        let song = self.playlists[idx].songs[song_idx].clone();
        self.play_path(&song.name, &song.path);
    }

    fn preload_song(&mut self, song_idx: usize) {
        let idx = self.playlist_list_state.selected().unwrap();

        let song = self.playlists[idx].songs[song_idx].clone();
        self.play_path(&song.name, &song.path);
    }

    fn update_song_queue(&mut self) {
        if self.sink.len() != self.last_queue_length {
            // TODO: Implement Repeat::One

            if !self.song_queue.is_empty() {
                self.song_queue.remove(0);

                if self.repeat == Repeat::One {
                    if let Playing::Playlist(_, song_idx) = self.playing {
                        self.preload_song(song_idx);
                    }
                } else if let Playing::Playlist(playlist_idx, idx) = self.playing {
                    let mut song_idx = idx + PRELOAD_SONG_COUNT;

                    let out_of_bounds = song_idx >= self.playlists[playlist_idx].songs.len();
                    if !out_of_bounds {
                        self.log = format!("Preloading a song from idx {song_idx}...");
                        self.preload_song(song_idx);
                    } else if self.repeat == Repeat::All {
                        song_idx %= self.playlists[playlist_idx].songs.len();
                        self.log = format!("Preloading a song from idx {song_idx}...");
                        self.preload_song(song_idx);
                    }

                    let out_of_bounds = idx + 1 >= self.playlists[playlist_idx].songs.len();
                    let new_idx = if out_of_bounds {
                        if self.repeat == Repeat::All {
                            0
                        } else {
                            self.playlists[playlist_idx].songs[idx].playing = false;
                            return;
                        }
                    } else {
                        idx + 1
                    };

                    self.playlists[playlist_idx].songs[idx].playing = false;
                    self.playlists[playlist_idx].songs[new_idx].playing = true;
                    self.playing = Playing::Playlist(playlist_idx, new_idx);
                }
            } else if self.repeat == Repeat::One {
                if let Playing::Playlist(_, song_idx) = self.playing {
                    self.preload_song(song_idx);
                }
            } else {
                self.log = String::from("Queue is empty");
            }

            self.last_queue_length = self.sink.len();
        }
    }

    fn move_item(&mut self) {
        if self.focused == Focused::Left {
            let idx = self.playlist_list_state.selected().unwrap();

            if self.playlists[idx].selected == Selected::Moving {
                self.playlists[idx].selected = Selected::Focused;
            } else {
                self.playlists[idx].selected = Selected::Moving;
            }
        } else {
            match self.window {
                Window::Songs => {
                    let playlist_idx = self.playlist_list_state.selected().unwrap();
                    let idx = self.song_list_state.selected().unwrap();

                    if self.playlists[playlist_idx].songs[idx].selected == Selected::Moving {
                        self.playlists[playlist_idx].songs[idx].selected = Selected::Focused;
                    } else {
                        self.playlists[playlist_idx].songs[idx].selected = Selected::Moving;
                    }
                }
                Window::GlobalSongs => {
                    let idx = self.global_song_list_state.selected().unwrap();

                    if self.global_songs[idx].selected == Selected::Moving {
                        self.global_songs[idx].selected = Selected::Focused;
                    } else {
                        self.global_songs[idx].selected = Selected::Moving;
                    }
                }
                _ => {}
            }
        }
    }

    fn toggle_repeat(&mut self) {
        self.repeat = match self.repeat {
            Repeat::None => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::None,
        };

        self.save_data.last_repeat_mode = match self.repeat {
            Repeat::None => 0,
            Repeat::All => 1,
            Repeat::One => 2,
        };
    }

    fn select_left_window(&mut self) {
        if self.focused == Focused::Left {
            self.see_songs_in_playlist();
            return;
        }

        let playlist_idx = self.playlist_list_state.selected().unwrap();

        self.playlists[playlist_idx].selected = Selected::Focused;
        self.focused = Focused::Left;

        match self.window {
            Window::Songs => {
                let idx = self.song_list_state.selected().unwrap();
                moving_warning!(self.playlists[playlist_idx].songs[idx], self.log);

                self.playlists[playlist_idx].songs[idx].selected = Selected::Unfocused;
            }
            Window::GlobalSongs => {
                let idx = self.global_song_list_state.selected().unwrap();
                moving_warning!(self.global_songs[idx], self.log);

                self.global_songs[idx].selected = Selected::Unfocused;
            }
            Window::DownloadManager => {}
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
    }

    fn select_right_window(&mut self) {
        if self.focused == Focused::Right {
            return;
        }

        let playlist_idx = self.playlist_list_state.selected().unwrap();
        moving_warning!(self.playlists[playlist_idx], self.log);

        self.playlists[playlist_idx].selected = Selected::Unfocused;
        self.focused = Focused::Right;

        match self.window {
            Window::Songs => {
                for (i, song) in self.playlists[playlist_idx].songs.iter_mut().enumerate() {
                    if song.selected == Selected::Unfocused {
                        self.song_list_state.select(Some(i));
                        song.selected = Selected::Focused;
                        return;
                    }
                }
                // If couldn't find a song with Selected::Unfocused, select first
                self.playlists[playlist_idx].songs[0].selected = Selected::Focused;
            }
            Window::GlobalSongs => {
                let idx = self.global_song_list_state.selected().unwrap();
                self.global_songs[idx].selected = Selected::Focused;
            }
            Window::DownloadManager => {}
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
        self.window = Window::Songs;
        self.song_list_state.select_first();
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
            self.log = String::from("Volume can't be negative");
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
                super::is_valid_youtube_link(&self.text_area.lines()[0])
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
                let playlist_idx = self.playlist_list_state.selected().unwrap();
                let song_name = self.text_area.lines()[0].clone();
                let was_empty = self.playlists[playlist_idx].songs.is_empty();

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

                self.playlists[playlist_idx].songs.insert(
                    idx,
                    Song {
                        selected: Selected::None,
                        name: song_name,
                        path: song_path,
                        playing: false,
                    },
                );

                if was_empty {
                    select!(self.playlists[playlist_idx].songs, self.song_list_state, 0);
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
                let id = self.downloads.len() as u8;

                self.downloads.insert(id, Download::Empty);
                self.handle_link(id, link);
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

    fn handle_link(&mut self, download_id: u8, link: SpotifyLink) {
        match link.clone() {
            SpotifyLink::Playlist(id) => {
                if self.save_data.last_valid_token.is_empty() {
                    self.recreate_spotify_token(download_id, link);
                    return;
                }

                let last_valid_token = self.save_data.last_valid_token.clone();
                let client = self.client.clone();

                self.downloads
                    .insert(download_id, Download::FetchingPlaylistInfo);
                self.join_handles.push(tokio::spawn(async move {
                    fetch_playlist_info(download_id, &client, &id, &last_valid_token).await
                }));
            }
            SpotifyLink::Track(id) => {
                if self.save_data.last_valid_token.is_empty() {
                    self.recreate_spotify_token(download_id, link);
                    return;
                }

                let last_valid_token = self.save_data.last_valid_token.clone();
                let client = self.client.clone();

                self.downloads
                    .insert(download_id, Download::FetchingTrackInfo);
                self.join_handles.push(tokio::spawn(async move {
                    fetch_track_info(download_id, &client, &id, &last_valid_token).await
                }));
            }
            SpotifyLink::Invalid => {
                let dlp_path = self.save_data.dlp_path.clone();
                let input = self.text_area.lines()[0].clone();

                self.downloads
                    .insert(download_id, Download::DownloadingYoutubeSong);
                self.join_handles.push(tokio::spawn(async move {
                    download_song(
                        download_id,
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
            Playing::Playlist(idx, song_idx) if !self.playlists.is_empty() => {
                self.playlists[idx].songs[song_idx].playing = false;
                self.playlists[idx].playing = false;
            }
            Playing::GlobalSong(idx) if !self.global_songs.is_empty() => {
                self.global_songs[idx].playing = false;
            }
            Playing::None => panic!("Tried to stop playing Playing::None"),
            _ => {}
        }
        self.playing = Playing::None;
        self.song_queue.clear();
        self.sink.stop();
    }

    fn play_current(&mut self) {
        let playlist_idx = self.playlist_list_state.selected().unwrap();

        if self.focused == Focused::Left {
            match self.playing {
                Playing::Playlist(playing_idx, _) => {
                    self.stop_playing_current();
                    if playing_idx == playlist_idx {
                        return;
                    }
                }
                Playing::GlobalSong(_) => self.stop_playing_current(),
                Playing::None => {}
            }

            self.playlists[playlist_idx].songs[0].playing = true;
            self.playlists[playlist_idx].playing = true;
            self.playing = Playing::Playlist(playlist_idx, 0);
            self.preload_songs(0);

            self.last_queue_length = self.sink.len();
            self.sink.play();
        } else {
            match self.window {
                Window::Songs => {
                    let idx = self.song_list_state.selected().unwrap();

                    match self.playing {
                        Playing::Playlist(_, song_idx) => {
                            self.stop_playing_current();
                            if song_idx == idx {
                                return;
                            }
                        }
                        Playing::GlobalSong(_) => self.stop_playing_current(),
                        Playing::None => {}
                    }

                    self.playlists[playlist_idx].playing = true;
                    self.playlists[playlist_idx].songs[idx].playing = true;

                    self.playing = Playing::Playlist(playlist_idx, idx);
                    self.preload_songs(idx);

                    self.last_queue_length = self.sink.len();
                    self.sink.play();
                }
                Window::GlobalSongs => {
                    let idx = self.global_song_list_state.selected().unwrap();

                    match self.playing {
                        Playing::Playlist(_, _) => self.stop_playing_current(),
                        Playing::GlobalSong(playing_idx) => {
                            self.stop_playing_current();
                            if playing_idx == idx {
                                return;
                            }
                        }
                        Playing::None => {}
                    }

                    self.global_songs[idx].playing = true;
                    self.playing = Playing::GlobalSong(idx);
                    self.song_queue.clear();
                    self.play_path(
                        &self.global_songs[idx].name.clone(),
                        &self.global_songs[idx].path.clone(),
                    );

                    self.last_queue_length = self.sink.len();
                    self.sink.play();
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
                            _ => self.log = String::from("Index out of range for config menu"),
                        }
                    }
                }
            }
        }
    }

    fn select_next(&mut self) {
        if self.focused == Focused::Left {
            select_next!(
                self.playlists,
                self.playlist_list_state,
                self.save_data.playlists
            );
            self.see_songs_in_playlist();
        } else {
            match self.window {
                Window::Songs => {
                    let idx = self.playlist_list_state.selected().unwrap();

                    select_next!(
                        self.playlists[idx].songs,
                        self.song_list_state,
                        self.save_data.playlists[idx].songs
                    );
                }
                Window::GlobalSongs => {
                    select_next!(
                        self.global_songs,
                        self.global_song_list_state,
                        self.save_data.songs
                    );
                }
                Window::DownloadManager => {}
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
            select_previous!(
                self.playlists,
                self.playlist_list_state,
                self.save_data.playlists
            );
            self.see_songs_in_playlist();
        } else {
            match self.window {
                Window::Songs => {
                    let idx = self.playlist_list_state.selected().unwrap();

                    select_previous!(
                        self.playlists[idx].songs,
                        self.song_list_state,
                        self.save_data.playlists[idx].songs
                    );
                }
                Window::GlobalSongs => {
                    select_previous!(
                        self.global_songs,
                        self.global_song_list_state,
                        self.save_data.songs
                    );
                }
                Window::DownloadManager => {}
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
        let file = match File::open(path) {
            Ok(file) => file,
            Err(err) => {
                self.log = format!("Failed to open file: {}", err);
                return;
            }
        };

        let source = match Decoder::new(file) {
            Ok(source) => source,
            Err(err) => {
                self.log = format!("Failed to decode file: {}", err);
                return;
            }
        };

        if let Some(duration) = source.total_duration() {
            let queued_song = self.song_queue.last();
            if let Some(last_song) = queued_song {
                self.song_queue.push(QueuedSong {
                    name: song_name.to_string(),
                    song_idx: last_song.song_idx + 1,
                    duration,
                });
            } else if let Playing::Playlist(_, idx) = self.playing {
                self.song_queue.push(QueuedSong {
                    name: song_name.to_string(),
                    song_idx: idx,
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
            let idx = self.playlist_list_state.selected().unwrap();

            self.log = format!("Remove playlist idx {idx}");
            self.playlists.remove(idx);
            self.save_data.playlists.remove(idx);

            if let Playing::Playlist(playing_idx, _) = self.playing {
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
        } else {
            match self.window {
                Window::Songs => {
                    let playlist_idx = self.playlist_list_state.selected().unwrap();
                    let idx = self.song_list_state.selected().unwrap();

                    self.log = format!("Remove song idx {idx}");

                    self.playlists[playlist_idx].songs.remove(idx);
                    self.save_data.playlists[playlist_idx].songs.remove(idx);

                    if let Playing::Playlist(playlist_idx, playing_idx) = self.playing {
                        if playing_idx == idx {
                            self.playing = Playing::Playlist(playlist_idx, playing_idx - 1);
                            self.preload_songs(playing_idx - 1);
                        }
                    }

                    if !self.playlists[playlist_idx].songs.is_empty() {
                        if idx == self.playlists[playlist_idx].songs.len() {
                            select!(
                                self.playlists[playlist_idx].songs,
                                self.song_list_state,
                                idx - 1
                            );
                        } else if idx == 0 {
                            select!(self.playlists[playlist_idx].songs, self.song_list_state, 0);
                        }
                    }
                }
                Window::GlobalSongs => {
                    let idx = self.global_song_list_state.selected().unwrap();

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
                Window::DownloadManager => {}
                Window::ConfigurationMenu => {}
            }
        }
    }

    pub(crate) fn init(&mut self) -> Result<(), Error> {
        let mut first = true;

        for playlist in &self.save_data.playlists {
            let songs = playlist
                .songs
                .iter()
                .filter_map(|song_name| {
                    self.save_data.songs.iter().find_map(|song| {
                        if &song.name == song_name {
                            Some(Song {
                                selected: Selected::None,
                                name: song.name.clone(),
                                path: song.path.clone(),
                                playing: false,
                            })
                        } else {
                            None
                        }
                    })
                })
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

        if !Path::new(&self.save_data.dlp_path).exists() {
            self.enter_input_mode(InputMode::GetDlp);
        }

        self.sink.set_volume(self.save_data.last_volume);
        self.repeat = match self.save_data.last_repeat_mode {
            0 => Repeat::None,
            1 => Repeat::All,
            2 => Repeat::One,
            _ => return Err(Error::BadSerialization),
        };
        Ok(())
    }

    fn enter_input_mode(&mut self, input_mode: InputMode) {
        self.mode = Mode::Input(input_mode);
        self.validate_input();
    }

    fn exit_input_mode(&mut self) {
        // Delete everything from the text area
        self.text_area.move_cursor(CursorMove::Head);
        self.text_area.delete_line_by_end();

        self.mode = Mode::Normal;
    }
}
