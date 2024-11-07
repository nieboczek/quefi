use crate::Config;
use rspotify::{clients::BaseClient, model::PlaylistId, ClientCredsSpotify, Credentials};

fn init_client(config: &Config) -> ClientCredsSpotify {
    ClientCredsSpotify::new(Credentials::new(
        &config.spotify_client_id,
        &config.spotify_client_secret,
    ))
}

fn get_playlist(client: ClientCredsSpotify, id: String) {
    client.playlist_items(PlaylistId::from_id(id).unwrap(), None, None);
}
