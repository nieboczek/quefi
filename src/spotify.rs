use base64::{prelude::BASE64_STANDARD, Engine};
use reqwest::Client;
use serde::Deserialize;

use crate::Error;

#[derive(Deserialize, Debug)]
struct ApiPlaylistMetadata {
    tracks: ApiTracks,
}

#[derive(Deserialize, Debug)]
struct ApiTracks {
    items: Vec<ApiTrackItem>,
}

#[derive(Deserialize, Debug)]
struct ApiTrackItem {
    track: ApiTrackMetadata,
}

#[derive(Deserialize, Debug, Clone)]
struct ApiTrackMetadata {
    name: String,
    artists: Vec<ApiArtist>,
    duration_ms: u32,
}

#[derive(Deserialize, Debug, Clone)]
struct ApiArtist {
    name: String,
}

#[derive(Deserialize)]
struct ApiTokenResponse {
    access_token: String,
}

#[derive(Debug)]
pub struct TrackInfo {
    pub query: String,
    pub duration_ms: u32,
}

#[derive(PartialEq)]
pub enum SpotifyLink<'a> {
    Track(&'a str),
    Playlist(&'a str),
    Invalid,
}

pub fn validate_spotify_link(link: &str) -> SpotifyLink {
    if let Some(track_id) = link.strip_prefix("https://open.spotify.com/track/") {
        SpotifyLink::Track(track_id)
    } else if let Some(playlist_id) = link.strip_prefix("https://open.spotify.com/playlist/") {
        SpotifyLink::Playlist(playlist_id)
    } else {
        SpotifyLink::Invalid
    }
}

fn transform_track_metadata(metadata: &ApiTrackMetadata) -> TrackInfo {
    TrackInfo {
        query: format!(
            "{} - {}",
            metadata
                .artists
                .iter()
                .map(|artist| artist.name.clone())
                .collect::<Vec<String>>()
                .join(", "),
            metadata.name
        ),
        duration_ms: metadata.duration_ms,
    }
}

pub async fn fetch_track_info(track_id: &str, access_token: &str) -> Result<TrackInfo, Error> {
    let client = Client::new();
    let url = format!("https://api.spotify.com/v1/tracks/{}", track_id);

    let res = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;

    let metadata: ApiTrackMetadata = res.json().await?;
    Ok(transform_track_metadata(&metadata))
}

pub async fn fetch_playlist_tracks(
    playlist_id: &str,
    access_token: &str,
) -> Result<Vec<TrackInfo>, Error> {
    let client = Client::new();
    let url = format!("https://api.spotify.com/v1/playlists/{}", playlist_id);

    let res = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;

    let metadata: ApiPlaylistMetadata = res.json().await?;
    Ok(metadata
        .tracks
        .items
        .iter()
        .map(|track| transform_track_metadata(&track.track))
        .collect::<Vec<TrackInfo>>())
}

pub async fn create_token(
    client: &Client,
    client_id: &str,
    client_secret: &str,
) -> Result<String, Error> {
    let auth = format!("{}:{}", client_id, client_secret);
    let auth_encoded = BASE64_STANDARD.encode(auth);

    let res = client
        .post("https://accounts.spotify.com/api/token")
        .header("Authorization", format!("Basic {}", auth_encoded))
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await?;

    let token: ApiTokenResponse = res.json().await?;
    Ok(token.access_token)
}

// TODO: Make a function to access all track of playlist (the fetch_playlist_tracks only lists the first 100)
