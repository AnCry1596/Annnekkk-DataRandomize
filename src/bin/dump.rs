//! One-off dump tool: exports the collections the two ported endpoints read
//! into ./data as newline-delimited JSON (one BSON doc per line).
//!
//! Usage: cargo run --bin dump

use anyhow::Result;
use futures::TryStreamExt;
use mongodb::bson::Document;
use mongodb::Client;
use std::io::Write;

/// Collections referenced by /bin/* and /randomdatav2/new.
const NEEDED: &[&str] = &[
    "bin_data",       // GET|POST /bin/*
    "first_name",     // randomdatav2: names
    "last_name",
    "words",          // randomdatav2: email username patterns
    "domains",        // randomdatav2: email domain
    "addresses",      // randomdatav2: address
    "countries",      // randomdatav2: country/state resolution
    "phone_prefixes", // randomdatav2: phone NPA/NXX
    "timezone",       // randomdatav2: timezone + offset
    "comments",       // randomdatav2: misc.comment
    "user_agents",    // randomdatav2: browser.userAgent
];

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let uri = std::env::args().nth(1).unwrap_or_else(|| {
        std::env::var("MONGODB_URI").expect("pass a URI as argv[1] or set MONGODB_URI")
    });
    let db_name = std::env::var("MONGODB_DB").unwrap_or_else(|_| "random_server".to_string());

    println!("connecting to {} / db={}", uri.split('@').next_back().unwrap_or("?"), db_name);
    let client = Client::with_uri_str(&uri).await?;
    let db = client.database(&db_name);

    let present = db.list_collection_names().await?;
    println!("\nserver has {} collections:", present.len());
    for c in &present {
        let n = db.collection::<Document>(c).count_documents(mongodb::bson::doc! {}).await?;
        let needed = if NEEDED.contains(&c.as_str()) { "NEEDED" } else { "-" };
        println!("  {:<8} {:<20} {:>10} docs", needed, c, n);
    }

    let missing: Vec<_> = NEEDED.iter().filter(|c| !present.contains(&c.to_string())).collect();
    if !missing.is_empty() {
        println!("\nWARNING: needed but absent on server: {:?}", missing);
    }

    std::fs::create_dir_all("data")?;
    println!("\ndumping to ./data ...");
    let mut total = 0u64;

    for name in NEEDED {
        if !present.contains(&name.to_string()) {
            continue;
        }
        let col = db.collection::<Document>(name);
        let mut cursor = col.find(mongodb::bson::doc! {}).await?;

        let path = format!("data/{}.json", name);
        let file = std::fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(file);

        let mut n = 0u64;
        while let Some(doc) = cursor.try_next().await? {
            // Extended JSON keeps types (ObjectId, i64) round-trippable via mongoimport.
            writeln!(w, "{}", mongodb::bson::Bson::Document(doc).into_canonical_extjson())?;
            n += 1;
            if n.is_multiple_of(50_000) {
                println!("    {} … {} docs", name, n);
            }
        }
        w.flush()?;
        total += n;
        println!("  {:<20} {:>10} docs -> {}", name, n, path);
    }

    println!("\ndone: {} docs across {} files in ./data", total, NEEDED.len());
    Ok(())
}
