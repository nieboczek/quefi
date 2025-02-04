use reqwest::Client;
use serde::Deserialize;

use crate::{Error, TaskResult, TaskReturn};

#[derive(Debug, Deserialize)]
struct ApiPlaylistMetadata {
    name: String,
    tracks: ApiTracks,
}

#[derive(Debug, Deserialize)]
struct ApiTracks {
    items: Vec<ApiTrackItem>,
}

#[derive(Debug, Deserialize)]
struct ApiTrackItem {
    track: ApiTrackMetadata,
}

#[derive(Debug, Deserialize, Clone)]
struct ApiTrackMetadata {
    name: String,
    artists: Vec<ApiArtist>,
    duration_ms: u32,
}

#[derive(Debug, Deserialize, Clone)]
struct ApiArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiTokenResponse {
    access_token: String,
}

#[derive(Debug)]
pub struct TrackInfo {
    // TODO: Use the duration to make searches more accurate
    _duration_ms: u32,
    pub query: String,
    pub name: String,
}

#[derive(Debug)]
pub struct PlaylistInfo {
    pub tracks: Vec<TrackInfo>,
    pub name: String,
}

#[derive(Debug, PartialEq, Clone)]
pub enum SpotifyLink {
    Track(String),
    Playlist(String),
    Invalid,
}

pub fn validate_spotify_link(link: &str) -> SpotifyLink {
    if let Some(track_id) = link.strip_prefix("https://open.spotify.com/track/") {
        if let Some((id, _)) = track_id.split_once('?') {
            SpotifyLink::Track(id.to_string())
        } else {
            SpotifyLink::Track(track_id.to_string())
        }
    } else if let Some(playlist_id) = link.strip_prefix("https://open.spotify.com/playlist/") {
        if let Some((id, _)) = playlist_id.split_once('?') {
            SpotifyLink::Playlist(id.to_string())
        } else {
            SpotifyLink::Playlist(playlist_id.to_string())
        }
    } else {
        SpotifyLink::Invalid
    }
}

fn transform_track_metadata(metadata: ApiTrackMetadata) -> TrackInfo {
    TrackInfo {
        query: format!(
            "{} - {}",
            metadata
                .artists
                .into_iter()
                .map(|artist| artist.name)
                .collect::<Vec<String>>()
                .join(", "),
            &metadata.name
        ),
        name: metadata.name,
        _duration_ms: metadata.duration_ms,
    }
}

pub async fn fetch_track_info(id: u8, client: &Client, track_id: &str, token: &str) -> TaskResult {
    let url = format!("https://api.spotify.com/v1/tracks/{}", track_id);

    let result = client.get(&url).bearer_auth(token).send().await;

    match result {
        Ok(res) => {
            let metadata: ApiTrackMetadata = res.json().await?;
            Ok(TaskReturn::TrackInfo(
                id,
                transform_track_metadata(metadata),
            ))
        }
        Err(err) => {
            if err.status().unwrap().as_u16() == 401 {
                Err(Error::SpotifyBadAuth(
                    id,
                    SpotifyLink::Track(track_id.to_string()),
                ))
            } else {
                Err(Error::Http(err))
            }
        }
    }
}

pub async fn fetch_playlist_info(
    id: u8,
    client: &Client,
    playlist_id: &str,
    token: &str,
) -> TaskResult {
    let url = format!("https://api.spotify.com/v1/playlists/{}?fields=name,tracks.items(track(name,artists(name),duration_ms))", playlist_id);

    let result = client.get(&url).bearer_auth(token).send().await;

    match result {
        Ok(res) => {
            if res.status().as_u16() == 401 {
                return Err(Error::SpotifyBadAuth(
                    id,
                    SpotifyLink::Playlist(playlist_id.to_string()),
                ));
            }

            let metadata: ApiPlaylistMetadata = res.json().await?;
            Ok(TaskReturn::PlaylistInfo(
                id,
                PlaylistInfo {
                    tracks: metadata
                        .tracks
                        .items
                        .into_iter()
                        .map(|track| transform_track_metadata(track.track))
                        .collect::<Vec<TrackInfo>>(),
                    name: metadata.name,
                },
            ))
        }
        Err(err) => Err(Error::Http(err)),
    }
}

pub async fn create_token(
    id: u8,
    client: &Client,
    client_id: &str,
    client_secret: &str,
    link: SpotifyLink,
) -> TaskResult {
    let res = client
        .post("https://accounts.spotify.com/api/token")
        .basic_auth(client_id, Some(client_secret))
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await?;

    let token: ApiTokenResponse = res.json().await?;
    Ok(TaskReturn::Token(id, token.access_token, link))
}

// TODO: Make a function to access all track of playlist (fetch_playlist_info only lists the first 100)
