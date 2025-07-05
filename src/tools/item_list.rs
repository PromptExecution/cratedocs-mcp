use anyhow::Result;
use reqwest;
use std::fs;
use std::path::Path;
use tar::Archive;
use flate2::read::GzDecoder;
use syn::{File, Item};
use tokio::fs as tokio_fs;

/// Represents filters for item listing.
#[derive(Debug)]
pub struct ItemListFilters {
    pub item_type: Option<String>,
    pub visibility: Option<String>,
    pub module: Option<String>,
}

/// Utility function to download and cache crate source.
async fn download_and_cache_crate(crate_name: &str, version: &str) -> Result<String> {
    let cache_dir = Path::new("./cache");
    let crate_dir = cache_dir.join(format!("{}-{}", crate_name, version));

    if crate_dir.exists() {
        return Ok(crate_dir.to_string_lossy().to_string());
    }

    let url = format!("https://crates.io/api/v1/crates/{}/{}/download", crate_name, version);
    let response = reqwest::get(&url).await?;
    let tarball = response.bytes().await?;

    fs::create_dir_all(&cache_dir)?;
    let tar_gz = GzDecoder::new(&*tarball);
    let mut archive = Archive::new(tar_gz);
    archive.unpack(&cache_dir)?;

    Ok(crate_dir.to_string_lossy().to_string())
}

/// Stub for the crate item enumeration tool.
/// This will use rust-analyzer to enumerate items in a crate.
pub async fn list_crate_items(
    crate_name: &str,
    version: &str,
    filters: Option<ItemListFilters>,
) -> Result<String> {
    let crate_path = download_and_cache_crate(crate_name, version).await?;
    let mut items = Vec::new();

    for entry in fs::read_dir(crate_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            let content = fs::read_to_string(&path)?;
            let parsed_file: File = syn::parse_file(&content)?;

            for item in parsed_file.items {
                match item {
                    Item::Struct(_) if filters.as_ref().map_or(true, |f| f.item_type.as_deref() == Some("struct")) => items.push(format!("{:?}", item)),
                    Item::Enum(_) if filters.as_ref().map_or(true, |f| f.item_type.as_deref() == Some("enum")) => items.push(format!("{:?}", item)),
                    Item::Trait(_) if filters.as_ref().map_or(true, |f| f.item_type.as_deref() == Some("trait")) => items.push(format!("{:?}", item)),
                    Item::Fn(_) if filters.as_ref().map_or(true, |f| f.item_type.as_deref() == Some("fn")) => items.push(format!("{:?}", item)),
                    _ => {}
                }
            }
        }
    }

    Ok(items.join("\n"))
}
