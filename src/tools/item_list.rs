use anyhow::Result;
use reqwest;
use std::fs;
use std::path::Path;
use tar::Archive;
use flate2::read::GzDecoder;
use syn::{File, Item};

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

    // Most crates have their source in a "src" subdirectory
    let src_path = Path::new(&crate_path).join("src");

    fn visit_rs_files<F: FnMut(&Path)>(dir: &Path, cb: &mut F) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit_rs_files(&path, cb);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                    cb(&path);
                }
            }
        }
    }

    visit_rs_files(&src_path, &mut |path: &Path| {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(parsed_file) = syn::parse_file(&content) {
                for item in parsed_file.items {
                    if let Item::Struct(s) = &item {
                        if filters.as_ref().map_or(true, |f| f.item_type.as_deref().map_or(true, |ty| ty == "struct")) {
                            items.push(("Structs", format!("{}", s.ident)));
                        }
                    }
                    if let Item::Enum(e) = &item {
                        if filters.as_ref().map_or(true, |f| f.item_type.as_deref().map_or(true, |ty| ty == "enum")) {
                            items.push(("Enums", format!("{}", e.ident)));
                        }
                    }
                    if let Item::Trait(t) = &item {
                        if filters.as_ref().map_or(true, |f| f.item_type.as_deref().map_or(true, |ty| ty == "trait")) {
                            items.push(("Traits", format!("{}", t.ident)));
                        }
                    }
                    if let Item::Fn(f) = &item {
                        if filters.as_ref().map_or(true, |f| f.item_type.as_deref().map_or(true, |ty| ty == "fn")) {
                            items.push(("Functions", format!("{}", f.sig.ident)));
                        }
                    }
                }
            }
        }
    });

    use std::collections::BTreeMap;
    let mut grouped: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (kind, name) in items {
        grouped.entry(kind).or_default().push(name);
    }

    let mut output = String::new();
    for (kind, names) in grouped {
        output.push_str(&format!("## {}\n", kind));
        for name in names {
            output.push_str(&format!("- {}\n", name));
        }
        output.push('\n');
    }

    Ok(output)
}
