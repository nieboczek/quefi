use crate::{get_quefi_dir, Error, DLP_PATH};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::{
    fs::File,
    io::{self, copy},
    process::{Command, Stdio},
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
fn create_file() -> io::Result<File> {
    File::create("yt-dlp.exe")
}
#[cfg(not(target_os = "windows"))]
fn create_file() -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        //.mode(0o744) // this might affect A LOT
        .open("yt-dlp")
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    browser_download_url: String,
    name: String,
}

pub fn download_dlp(client: &Client) -> Result<(), Error> {
    let response = client
        .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
        .header("User-Agent", "quefi")
        .send()?;
    let release: Release = match response.json() {
        Ok(release) => release,
        Err(err) => return Err(Error::Http(err)),
    };
    let url = release
        .assets
        .into_iter()
        .find(|asset| asset.name == DLP_PATH)
        .map(|asset| asset.browser_download_url)
        .ok_or_else(|| {
            panic!("Didn't find {DLP_PATH} in releases");
        })
        .expect("This should never print");
    let mut content_response = client.get(url).send()?.error_for_status()?;
    let mut file = create_file()?;
    copy(&mut content_response, &mut file)?;
    Ok(())
}

pub fn download_song(dlp_path: String, yt_link: &str) -> Result<(), Error> {
    let dir = get_quefi_dir();
    #[cfg(not(target_os = "windows"))]
    let mut child = Command::new(dlp_path)
        .current_dir(dir.join("songs"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "-q",
            "-x",
            "--audio-format",
            "mp3",
            yt_link,
            "-o",
            "temp.mp3",
        ])
        .spawn()?;
    #[cfg(target_os = "windows")]
    let mut child = Command::new(dlp_path)
        .creation_flags(0x08000000) // Create no window
        .current_dir(dir.join("songs"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "-q",
            "-x",
            "--audio-format",
            "mp3",
            yt_link,
            "-o",
            "temp.mp3",
        ])
        .spawn()?;
    child.wait()?;
    Ok(())
}
