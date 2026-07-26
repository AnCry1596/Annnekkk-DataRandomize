//! Seeds the ./data dumps into a target MongoDB.
//!
//! Usage:
//!   cargo run --bin seed -- <uri> [db_name] [--force] [--dir <path>]
//!
//! Reads every data/<name>.json (newline-delimited canonical extended JSON, as
//! written by the `dump` binary) and inserts it into collection <name>.
//!
//! If the data directory is missing or empty, data.zip is downloaded from the
//! latest GitHub release and unpacked into it first.
//!
//! Non-empty target collections are skipped unless --force is given, so a
//! mistyped host cannot silently clobber a populated server.

use anyhow::{bail, Context, Result};
use mongodb::bson::{doc, Bson, Document};
use mongodb::Client;
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

/// Insert in batches — one 271k-document insert_many would exceed the BSON limit.
const BATCH: usize = 5_000;

/// Repo to pull data.zip from, as "owner/name". No default — this project has no
/// GitHub home yet, and a guessed slug would fail in a confusing way. Set
/// DATA_REPO (or DATA_URL for a direct link), or just keep data/ populated.
const REPO_ENV: &str = "AnCry1596/Annnekkk-DataRandomize";

/// Asset name looked for on the release.
const ASSET: &str = "data.zip";

struct Args {
    uri: String,
    db: String,
    force: bool,
    dir: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut uri = None;
    let mut db = None;
    let mut force = false;
    let mut dir = PathBuf::from("data");

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--force" => force = true,
            "--dir" => dir = PathBuf::from(it.next().context("--dir needs a path")?),
            "-h" | "--help" => {
                println!("usage: seed <uri> [db_name] [--force] [--dir <path>]");
                println!("  --force  reload collections that already have documents");
                println!("  --dir    source directory of *.json dumps (default: data)");
                println!();
                println!("If the source directory has no dumps, {} is downloaded from:", ASSET);
                println!("  DATA_REPO=owner/name   latest GitHub release of that repo");
                println!("  DATA_URL=https://…     a direct link to the archive");
                std::process::exit(0);
            }
            s if s.starts_with('-') => bail!("unknown flag: {}", s),
            s if uri.is_none() => uri = Some(s.to_string()),
            s if db.is_none() => db = Some(s.to_string()),
            s => bail!("unexpected argument: {}", s),
        }
    }

    Ok(Args {
        uri: uri.context("missing target URI\nusage: seed <uri> [db_name] [--force]")?,
        db: db.unwrap_or_else(|| "random_server".to_string()),
        force,
        dir,
    })
}

/// Collection name from a dump filename: data/bin_data.json -> bin_data
fn collection_name(path: &Path) -> Option<String> {
    if path.extension()? != "json" {
        return None;
    }
    Some(path.file_stem()?.to_string_lossy().to_string())
}

/// True when `dir` holds no *.json dumps (missing, empty, or only other files).
fn needs_download(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => !entries
            .filter_map(|e| e.ok())
            .any(|e| collection_name(&e.path()).is_some()),
        Err(_) => true,
    }
}

/// Browser-download URL of `ASSET` on the repo's latest release.
fn latest_asset_url(repo: &str) -> Result<String> {
    let api = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let body: serde_json::Value = ureq::get(&api)
        .header("User-Agent", "seed")
        .header("Accept", "application/vnd.github+json")
        .call()
        .with_context(|| format!("querying {}", api))?
        .body_mut()
        .read_json()?;

    let assets = body["assets"]
        .as_array()
        .context("release JSON has no assets array")?;

    for a in assets {
        if a["name"].as_str() == Some(ASSET) {
            return a["browser_download_url"]
                .as_str()
                .map(str::to_string)
                .context("asset has no browser_download_url");
        }
    }

    let names: Vec<&str> = assets.iter().filter_map(|a| a["name"].as_str()).collect();
    bail!(
        "no {} on the latest release of {} (found: {})",
        ASSET,
        repo,
        if names.is_empty() { "nothing".to_string() } else { names.join(", ") }
    )
}

/// Download data.zip from the latest release and unpack the *.json entries into `dir`.
fn download_data(dir: &Path) -> Result<()> {
    let announced = format!("no dumps in {} — fetching {}", dir.display(), ASSET);

    let url = match std::env::var("DATA_URL") {
        Ok(u) => {
            println!("{}", announced);
            u
        }
        Err(_) => {
            let repo = std::env::var(REPO_ENV).map_err(|_| {
                anyhow::anyhow!(
                    "{dir} has no *.json dumps and there is nowhere to fetch them from.\n\
                     Either populate it with `dump`, or point seed at an archive:\n  \
                     {repo_env}=owner/name   (downloads {asset} from that repo's latest release)\n  \
                     DATA_URL=https://…/{asset}",
                    dir = dir.display(),
                    repo_env = REPO_ENV,
                    asset = ASSET,
                )
            })?;
            println!("{}", announced);
            println!("  looking up latest release of {}", repo);
            latest_asset_url(&repo)?
        }
    };

    println!("  downloading {}", url);
    let mut buf = Vec::new();
    ureq::get(&url)
        .header("User-Agent", "seed")
        .call()
        .with_context(|| format!("downloading {}", url))?
        .body_mut()
        // ureq 3 caps in-memory bodies at 10MB by default; data.zip is right at
        // that line and only grows, so lift the limit well clear of it.
        .with_config()
        .limit(512 * 1024 * 1024)
        .reader()
        .read_to_end(&mut buf)?;
    println!("  got {:.1} MB, unpacking", buf.len() as f64 / 1e6);

    std::fs::create_dir_all(dir)?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(buf)).context("data.zip is not a zip")?;

    let mut n = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // Flatten and take the basename only, so a zip built with or without a
        // leading data/ directory both unpack correctly — and no entry can
        // escape `dir` via .. path traversal.
        let Some(name) = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|f| f.to_owned()))
        else {
            continue;
        };
        if collection_name(Path::new(&name)).is_none() {
            continue;
        }
        let out = dir.join(&name);
        let mut f = std::io::BufWriter::new(std::fs::File::create(&out)?);
        std::io::copy(&mut entry, &mut f)?;
        f.flush()?;
        n += 1;
    }

    if n == 0 {
        bail!("{} contained no *.json dumps", ASSET);
    }
    println!("  extracted {} files -> {}\n", n, dir.display());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    let args = parse_args()?;

    if needs_download(&args.dir) {
        download_data(&args.dir)?;
    }

    let mut files: Vec<PathBuf> = std::fs::read_dir(&args.dir)
        .with_context(|| format!("cannot read {}", args.dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| collection_name(p).is_some())
        .collect();
    files.sort();

    if files.is_empty() {
        bail!("no *.json dumps found in {}", args.dir.display());
    }

    let host = args.uri.split('@').next_back().unwrap_or("?");
    println!("target : {} / db={}", host, args.db);
    println!("source : {} ({} files)", args.dir.display(), files.len());
    println!("mode   : {}\n", if args.force { "FORCE (drop + reload)" } else { "skip non-empty" });

    let client = Client::with_uri_str(&args.uri).await?;
    let db = client.database(&args.db);
    // Fail fast on a bad host/credentials rather than mid-way through seeding.
    db.list_collection_names().await.context("cannot reach target server")?;

    let interactive = std::io::stdout().is_terminal();
    let mut inserted_total = 0u64;
    let mut skipped = Vec::new();

    for path in &files {
        let name = collection_name(path).unwrap();
        let col = db.collection::<Document>(&name);

        let existing = col.count_documents(doc! {}).await?;
        if existing > 0 {
            if !args.force {
                println!("  {:<20} {:>9} docs present -> SKIPPED (use --force)", name, existing);
                skipped.push(name);
                continue;
            }
            println!("  {:<20} dropping {} existing docs", name, existing);
            col.drop().await?;
        }

        // Streamed line-by-line: bin_data.json is ~58MB, no reason to hold it all.
        let file = std::fs::File::open(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let reader = std::io::BufReader::new(file);

        let mut batch: Vec<Document> = Vec::with_capacity(BATCH);
        let mut n = 0u64;

        for (i, line) in reader.lines().enumerate() {
            let line = line.with_context(|| format!("{}:{}", path.display(), i + 1))?;
            if line.trim().is_empty() {
                continue;
            }
            let json: serde_json::Value = serde_json::from_str(&line)
                .with_context(|| format!("{}:{} is not valid JSON", path.display(), i + 1))?;
            // Round-trips the extended JSON written by `dump`, so ObjectId and
            // i32/i64 land as their original BSON types rather than as strings.
            match Bson::try_from(json)? {
                Bson::Document(d) => batch.push(d),
                other => bail!("{}:{} is {:?}, expected a document", path.display(), i + 1, other.element_type()),
            }

            if batch.len() >= BATCH {
                n += batch.len() as u64;
                col.insert_many(std::mem::take(&mut batch)).await?;
                // ponytail: \r progress only when interactive — it turns a
                // redirected log into one unreadable line otherwise.
                if interactive {
                    print!("\r    {:<20} {:>9} docs", name, n);
                    std::io::stdout().flush().ok();
                }
            }
        }

        if !batch.is_empty() {
            n += batch.len() as u64;
            col.insert_many(batch).await?;
        }

        if interactive {
            print!("\r");
        }
        println!("  {:<20} {:>9} docs inserted", name, n);
        inserted_total += n;
    }

    println!("\ndone: {} docs inserted into {}/{}", inserted_total, host, args.db);
    if !skipped.is_empty() {
        println!("skipped {} non-empty collection(s): {}", skipped.len(), skipped.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name_from_dump_path() {
        assert_eq!(collection_name(Path::new("data/bin_data.json")).as_deref(), Some("bin_data"));
        assert_eq!(collection_name(Path::new("data/user_agents.json")).as_deref(), Some("user_agents"));
        assert_eq!(collection_name(Path::new("data/notes.txt")), None);
    }

    #[test]
    fn needs_download_detects_usable_dumps() {
        let base = std::env::temp_dir().join(format!("seedtest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        // Missing directory.
        assert!(needs_download(&base));

        // Exists but empty.
        std::fs::create_dir_all(&base).unwrap();
        assert!(needs_download(&base));

        // Has a non-dump file only.
        std::fs::write(base.join("README.txt"), b"x").unwrap();
        assert!(needs_download(&base));

        // Has a real dump.
        std::fs::write(base.join("comments.json"), b"{}\n").unwrap();
        assert!(!needs_download(&base));

        std::fs::remove_dir_all(&base).ok();
    }

    /// The dump writes canonical extended JSON; seeding must restore real BSON
    /// types, not stringly-typed copies, or _id lookups in /bin/* would miss.
    #[test]
    fn extended_json_round_trips_to_bson_types() {
        let line = r#"{"_id":{"$numberInt":"131840"},"cardType":"MASTERCARD"}"#;
        let json: serde_json::Value = serde_json::from_str(line).unwrap();
        let Bson::Document(d) = Bson::try_from(json).unwrap() else { panic!("not a doc") };
        assert_eq!(d.get_i32("_id").unwrap(), 131840);
        assert_eq!(d.get_str("cardType").unwrap(), "MASTERCARD");

        let oid = r#"{"_id":{"$oid":"69a4625fac599439b7083476"},"offset":{"$numberInt":"-660"}}"#;
        let json: serde_json::Value = serde_json::from_str(oid).unwrap();
        let Bson::Document(d) = Bson::try_from(json).unwrap() else { panic!("not a doc") };
        assert!(matches!(d.get("_id"), Some(Bson::ObjectId(_))));
        assert_eq!(d.get_i32("offset").unwrap(), -660);
    }
}
