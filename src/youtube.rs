use crate::{get_quefi_dir, Config, Error, DLP_PATH};
use reqwest::Client;
use serde::Deserialize;
#[cfg(target_os = "windows")]
use std::{
    process::{exit, Stdio},
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::timeout,
};

#[warn(clippy::all)]
#[cfg(target_os = "windows")]
async fn create_file() -> tokio::io::Result<File> {
    File::create("yt-dlp.exe").await
}
#[cfg(not(target_os = "windows"))]
async fn create_file() -> tokio::io::Result<File> {
    use tokio::fs::OpenOptions;

    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o744)
        .open("yt-dlp")
        .await
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

pub async fn get_dlp(client: &Client) -> Result<(), Error> {
    let response = client
        .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
        .header("User-Agent", "quefi")
        .send()
        .await?;
    let release: Release = match response.json().await {
        Ok(release) => release,
        Err(err) => return Err(Error::Http(err)),
    };
    let url = release
        .assets
        .into_iter()
        .find(|release| release.name == DLP_PATH)
        .map(|release| release.browser_download_url)
        .ok_or_else(|| {
            eprintln!("Didn't find {DLP_PATH} in releases");
            exit(1);
        })
        .unwrap();
    let mut response = match client.get(url).send().await.unwrap().error_for_status() {
        Ok(response) => response,
        Err(err) => {
            eprintln!("{err}");
            exit(1);
        }
    };
    let mut file = create_file().await?;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await.unwrap();
    }
    Ok(())
}

pub async fn download(config: Config, yt_link: &str, idx: u32) -> Result<(), Error> {
    let dir = get_quefi_dir();
    #[cfg(not(target_os = "windows"))]
    let mut child = Command::new(config.dlp_path)
        .current_dir(dir.join("songs"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "-x",
            "--audio-format",
            "mp3",
            yt_link,
            "-o",
            format!("{idx}.mp3").as_str(),
        ])
        .spawn()?;
    #[cfg(target_os = "windows")]
    let mut child = Command::new(config.dlp_path)
        .creation_flags(0x08000000) // Create no window
        .current_dir(dir.join("songs"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "-x",
            "--audio-format",
            "mp3",
            yt_link,
            "-o",
            format!("{idx}.mp3").as_str(),
        ])
        .spawn()?;
    match timeout(Duration::from_secs(config.timeout), child.wait()).await {
        Ok(result) => result?,
        Err(_) => {
            child.kill().await?;
            return Err(Error::Timeout);
        }
    };
    let mut stderr = vec![];
    if let Some(mut reader) = child.stderr {
        reader.read_to_end(&mut stderr).await?;
    }
    println!("STDERR:\n{}", String::from_utf8_lossy(&stderr));
    let mut stdout = vec![];
    if let Some(mut reader) = child.stdout {
        reader.read_to_end(&mut stdout).await?;
    }
    println!("STDOUT:\n{}", String::from_utf8_lossy(&stdout));
    Ok(())
}
