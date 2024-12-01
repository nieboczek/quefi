use crate::{get_quefi_dir, Error, DLP_EXECUTABLE_NAME};
use regex::{Match, Regex};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fmt::Write,
    io,
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{fs::File, io::copy, process::Command};

#[cfg(not(target_os = "windows"))]
use tokio::fs::OpenOptions;

#[cfg(target_os = "windows")]
async fn create_file() -> io::Result<File> {
    File::create("yt-dlp.exe").await
}

#[cfg(not(target_os = "windows"))]
async fn create_file() -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        //.mode(0o744) // this might affect A LOT
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

#[derive(Serialize)]
struct Body<'a> {
    query: &'a str,
    params: &'a str,
    client_id: Option<String>,
    context: Value,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub video_id: String,
    pub duration_ms: u32,
}

pub async fn download_dlp(client: &Client) -> Result<(), Error> {
    let response = client
        .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
        .header("User-Agent", "nieboczek/quefi")
        .send()
        .await?;

    let release: Release = match response.json().await {
        Ok(release) => release,
        Err(err) => return Err(Error::Http(err)),
    };

    let url = release
        .assets
        .into_iter()
        .find(|asset| asset.name == DLP_EXECUTABLE_NAME)
        .map(|asset| asset.browser_download_url)
        .expect("Didn't find the correct dlp in releases");

    let response = client.get(url).send().await?.error_for_status()?;
    let mut file = create_file().await?;

    copy(&mut response.bytes().await?.as_ref(), &mut file).await?;
    Ok(())
}

pub async fn download_song(dlp_path: &str, yt_link: &str) -> Result<(), Error> {
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

    child.wait().await?;
    Ok(())
}

fn get_timestamp() -> String {
    let now = SystemTime::now();
    let duration_since_epoch = now.duration_since(UNIX_EPOCH).unwrap();

    let days_since_epoch = duration_since_epoch.as_secs() / 86_400;
    let unix_epoch_day = 719_163;
    let days = days_since_epoch + unix_epoch_day;

    let (year, month, day) = days_to_date(days);

    let mut date_string = String::new();
    write!(date_string, "{:04}{:02}{:02}", year, month, day).unwrap();
    date_string
}

fn days_to_date(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1;
    while days >= year_length(year) {
        days -= year_length(year);
        year += 1;
    }
    let mut month = 1;
    while days >= month_length(year, month) {
        days -= month_length(year, month);
        month += 1;
    }
    (year, month, days + 1)
}

fn year_length(year: u64) -> u64 {
    if is_leap_year(year) {
        366
    } else {
        365
    }
}

fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn month_length(year: u64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => panic!("Invalid month"),
    }
}

pub async fn get_visitor_id(client: &Client) -> Result<String, Error> {
    let response = client
        .get("https://music.youtube.com")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:88.0) Gecko/20100101 Firefox/88.0",
        )
        .header("Accept", "*/*")
        .header("Content-Type", "application/json")
        .header("Accept-Encoding", "gzip, deflate")
        .header("Content-Encoding", "gzip")
        .header("Origin", "https://music.youtube.com")
        .send()
        .await?;

    let text = response.text().await?;
    let matches = Regex::new(r"ytcfg\.set\s*\(\s*(\{.+?\})\s*\)\s*;")
        .unwrap()
        .find_iter(&text)
        .collect::<Vec<Match>>();

    if !matches.is_empty() {
        let json = matches[0]
            .as_str()
            .strip_prefix("ytcfg.set(")
            .unwrap()
            .strip_suffix(");")
            .unwrap();

        let ytcfg: Value = serde_json::from_str(json).unwrap();
        let visitor_data = ytcfg.get("VISITOR_DATA").unwrap().clone();
        Ok(visitor_data.as_str().unwrap().to_string())
    } else {
        Err(Error::YtMusicError)
    }
}

async fn send_request<'a>(
    client: &Client,
    visitor_id: &str,
    body: Body<'a>,
) -> Result<Value, Error> {
    let response = client
        .post("https://music.youtube.com/youtubei/v1/search?alt=json")
        .json(&body)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:88.0) Gecko/20100101 Firefox/88.0",
        )
        .header("Accept", "*/*")
        .header("Content-Type", "application/json")
        .header("Content-Encoding", "gzip")
        .header("Origin", "https://music.youtube.com")
        .header("X-Goog-Visitor-Id", visitor_id)
        .send()
        .await?;

    response.error_for_status_ref()?;

    let text = response.text().await?;
    Ok(serde_json::from_str(&text).unwrap())
}

fn parse_search_result(value: &Value) -> SearchResult {
    let mut result = if let Some(video_id) = value
        .get("overlay")
        .and_then(|v| v.get("musicItemThumbnailOverlayRenderer"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.get("musicPlayButtonRenderer"))
        .and_then(|v| v.get("playNavigationEndpoint"))
        .and_then(|v| v.get("watchEndpoint"))
        .and_then(|v| v.get("videoId"))
    {
        SearchResult {
            video_id: video_id.as_str().unwrap().to_string(),
            duration_ms: 0,
        }
    } else {
        SearchResult {
            video_id: String::new(),
            duration_ms: 0,
        }
    };

    let runs =
        &value["flexColumns"][1]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"];

    let text = runs[0]["text"].as_str().unwrap().to_lowercase();
    let that_thing = [
        "album", "artist", "playlist", "song", "video", "station", "profile", "podcast", "episode",
    ]
    .contains(&text.as_str());

    let runs_offset = if runs[0].as_object().unwrap().len() == 1 && that_thing {
        2
    } else {
        0
    };

    let (_, runs) = runs.as_array().unwrap().split_at(runs_offset);
    let mut i: u16 = 0;
    for run in runs {
        if i % 2 == 1 {
            i += 1;
            continue;
        }

        let text = run["text"].as_str().unwrap();
        if run.get("navigationEndpoint").is_none()
            && Regex::new(r"^(\d+:)*\d+:\d+$")
                .unwrap()
                .is_match(text)
        {
            result.duration_ms = parse_duration(text);
        }
        i += 1;
    }
    result
}

fn parse_duration(duration: &str) -> u32 {
    let duration = duration.trim();
    if duration.is_empty() {
        return 0;
    }

    let duration_split: Vec<&str> = duration.split(':').collect();
    if duration_split.iter().any(|&d| d.parse::<u32>().is_err()) {
        return 0;
    }

    let increments = [1000, 60000, 3600000];
    let milliseconds: u32 = duration_split
        .iter()
        .rev()
        .zip(increments.iter())
        .map(|(time, &multiplier)| time.parse::<u32>().unwrap() * multiplier)
        .sum();

    milliseconds
}

pub async fn search(client: &Client, visitor_id: &str, query: &str) -> Result<SearchResult, Error> {
    let body = Body {
        query,
        client_id: None,
        params: "EgWKAQIIAUICCAFqDBAOEAoQAxAEEAkQBQ%3D%3D",
        context: json!({
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": format!("1.{}.01.00", get_timestamp()),
            },
            "user": {},
        }),
    };

    let json = send_request(client, visitor_id, body).await?;

    if let Some(contents) = json.get("contents") {
        let results = if let Some(renderer) = contents.get("tabbedSearchResultsRenderer") {
            &renderer["tabs"][0]["tabRenderer"]["content"]
        } else {
            contents
        };

        let section_list = &results["sectionListRenderer"]["contents"];
        let has_renderer = section_list.get("itemSectionRenderer").is_some();

        if section_list.as_array().unwrap().len() == 1 && has_renderer {
            return Err(Error::YtMusicError);
        }

        let mut shelf_contents: &Vec<Value> = &Vec::new();
        for res in section_list.as_array().unwrap() {
            if let Some(renderer) = res.get("musicCardShelfRenderer") {
                if let Some(contents) = renderer.get("contents") {
                    shelf_contents = contents.as_array().unwrap();
                }
            } else if let Some(renderer) = res.get("musicShelfRenderer") {
                shelf_contents = renderer["contents"].as_array().unwrap();
            }
        }
        return Ok(parse_search_result(
            &shelf_contents[0]["musicResponsiveListItemRenderer"],
        ));
    }
    Err(Error::YtMusicError)
}
