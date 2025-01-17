// MIT License
//
// Copyright (c) 2021-2022 Tilton Raccoon <tilton@tiltonraccoon.com>
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

#![macro_use]
extern crate env_logger;
extern crate log;

use clap::Parser;
use log::{error, info, warn};
use reqwest::Error;
use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio_stream::StreamExt;

const USER_AGENT: &str = "monosodium/1.0 (https://github.com/tiltonraccoon/monosodium)";

#[derive(Parser, Debug)]
#[clap(version = "1.0", author = "Tilton Raccoon <tilton@tiltonraccoon.com>")]
struct Opts {
    #[clap(short, long)]
    user_id: u32,
    #[clap(short, long)]
    directory: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct ApiResponse {
    posts: Vec<Post>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Post {
    id: u64,
    created_at: String,
    updated_at: String,
    file: FileData,
    tags: Tags,
    rating: String,
    flags: Flags,
}

#[derive(Serialize, Deserialize, Debug)]
struct FileData {
    width: u32,
    height: u32,
    ext: String,
    size: u32,
    md5: String,
    url: Option<String>, // May not be present if the file is deleted
}

#[derive(Serialize, Deserialize, Debug)]
struct Tags {
    general: Vec<String>,
    species: Vec<String>,
    character: Vec<String>,
    copyright: Vec<String>,
    artist: Vec<String>,
    invalid: Vec<String>,
    lore: Vec<String>,
    meta: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Flags {
    pending: bool,
    flagged: bool,
    deleted: bool,
}

fn archive_metadata(post: &Post, tags_path: PathBuf) {
    if let Ok(mut tags_file) = File::create(&tags_path) {
        let _ = tags_file.write_all(serde_json::to_string_pretty(&post).unwrap().as_bytes());
    }
}

async fn archive_post(post: &Post, out_path: PathBuf) -> Result<(), Error> {
    // This isn't really async, we block and download only one image
    // at a time.
    if let Some(url) = &post.file.url {
        match File::create(&out_path) {
            Ok(mut output) => {
                // Since this uses async code, and we don't want this function
                // to be async itself, we must spawn an async closure.
                let url = url.to_owned();
                info!("downloading {}", &url);
                match reqwest::get(&url).await {
                    Ok(response) => {
                        if let Ok(bytes) = response.bytes().await {
                            let _ = output.write_all(&bytes);
                        }
                    }
                    Err(e) => {
                        error!("Could not fetch url {}: {:?}", &url, e)
                    }
                }
                // Force a sleep, don't pound the server!
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
            Err(e) => {
                error!("{:?}", e);
            }
        }
    }

    Ok(())
}

fn favorites_url(user_id: u32, page: usize) -> String {
    format!(
        "https://e621.net/favorites.json?user_id={}&page={}",
        user_id, page
    )
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();

    let opts: Opts = Opts::parse();

    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;

    let directory = Path::new(&opts.directory);
    let metadata_dir = directory.join("metadata");
    create_dir_all(&metadata_dir).expect("Could not create metadata directory");

    let mut page: usize = 1;

    loop {
        info!("Checking favorites page {:2}", page);

        let url = favorites_url(opts.user_id, page);

        let response = client.get(&url).send().await?.json::<ApiResponse>().await?;

        if response.posts.is_empty() {
            break;
        }

        page += 1;

        let mut stream = tokio_stream::iter(response.posts);

        while let Some(post) = stream.next().await {
            if post.flags.deleted {
                warn!(
                    "favorite #{} has been marked deleted on e621.net, skipping.",
                    post.id
                );
            } else {
                let image_file = format!("{}.{}", post.file.md5, post.file.ext);
                let tags_file = format!("{}.json", post.file.md5);
                let image_path = directory.join(image_file);
                let tags_path = metadata_dir.join(tags_file);
                if !image_path.exists() {
                    archive_post(&post, image_path).await?;
                }
                // Always re-archive the JSON, since tags may have been updated since last time.
                archive_metadata(&post, tags_path);
            }
        }

        // Force a sleep between page fetches, don't pound the server!
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }

    println!("Done! Enjoy that offline archive!");

    Ok(())
}
