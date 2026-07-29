# annnekkk_random_server

A small Rust HTTP service on [warp](https://github.com/seanmonstar/warp) that serves
card BIN lookups and synthetic identity data from MongoDB.

- **BIN lookup** — issuer, country, card type and category for a 6-digit BIN, with an
  in-process cache in front of Mongo.
- **Random identity** — a coherent fake person: name, email, password, phone with a
  real area code for the address, user-agent, timezone and postal address.

## Quick setup (Windows)

Installs MongoDB if none is running, downloads the latest release, seeds the
database and registers the server to start at logon:

```powershell
irm https://raw.githubusercontent.com/AnCry1596/Annnekkk-DataRandomize/main/setup.ps1 | iex
```

Run it from an **Administrator** PowerShell if you need MongoDB installed —
that step uses Chocolatey and requires elevation. Everything else works
unelevated, and re-running is safe: each step is skipped when already done.

Useful options — download the script first to pass any of these:

```powershell
irm https://raw.githubusercontent.com/AnCry1596/Annnekkk-DataRandomize/main/setup.ps1 -OutFile setup.ps1

.\setup.ps1 -Port 9000                              # different port
.\setup.ps1 -MongoUri 'mongodb://user:pass@host/'   # use an existing MongoDB
.\setup.ps1 -InstallDir D:\tools\randomserver       # different location
.\setup.ps1 -NoAutoStart                            # skip the startup task
.\setup.ps1 -Force                                  # re-seed populated collections
```

To remove the startup entry:

```powershell
Unregister-ScheduledTask -TaskName AnnnekkkRandomServer -Confirm:$false
```

## From source

`.env.example` points at a local MongoDB, so with one running on the default
port no edit is needed:

```sh
cp .env.example .env      # edit MONGODB_URI for a remote server
cargo run --bin seed -- mongodb://localhost:27017/   # first run only
cargo run                 # http://127.0.0.1:8080
```

## API

### `GET /bin/{bin}` · `POST /bin/{bin}`

BINs longer than 6 digits are truncated; non-numeric input is rejected.

```sh
curl localhost:8080/bin/444444
```

```json
{
  "success": true,
  "bin": "444444",
  "cardInfo": { "type": "VISA", "subType": "CREDIT", "category": "GOLD", "regulated": "N" },
  "binInfo": { "category": "CONSUMER", "length": "Unknown" },
  "issuer": { "bank": "CREDIT AGRICOLE BANK POLSKA S.A.", "countryCode": "PL", "country": "Poland" },
  "metadata": { "cached": false, "processingTime": "274ms", "cacheStats": "miss", "timestamp": "…", "credits": "…" }
}
```

Repeat requests are served from cache (`"cacheStats": "hit"`, sub-millisecond) for one
hour, up to 10,000 BINs.

### `POST /bin`

Same response, BIN in the body as either a string or a number:

```sh
curl -X POST localhost:8080/bin -H 'content-type: application/json' -d '{"bin":"411111"}'
curl -X POST localhost:8080/bin -H 'content-type: application/json' -d '{"bin":411111}'
```

### `GET /randomdatav2/new?country=XX`

`country` defaults to `US`.

```sh
curl 'localhost:8080/randomdatav2/new?country=CA'
```

```json
{
  "success": true,
  "personal": {
    "first": "Rialey", "last": "Scheirman", "fullname": "Rialey Scheirman",
    "email": "rialeyscheirman98@hotmail.es",
    "phone": "5068556856",
    "phoneFormatted": {
      "parentheses": "(506) 855-6856", "dashes": "506-855-6856",
      "dots": "506.855.6856", "international": "+15068556856"
    }
  },
  "security": { "password": "2ZVF0LrZ77ci/!" },
  "browser": {
    "userAgent": "Mozilla/5.0 …", "language": "en-US", "colorDepth": 24,
    "screen": { "width": 1366, "height": 768, "type": "laptop" }
  },
  "location": {
    "timeZone": "Pacific/Guadalcanal", "offset": 660,
    "address": {
      "address1": "11 Borden St", "address2": "", "city": "Moncton",
      "state": "NB", "state_name": "New Brunswick", "region": "New Brunswick",
      "regionId": 70, "postalCode": "E1B 3N7",
      "country_id": 38, "country_code": "CA", "country_name": "Canada"
    }
  },
  "misc": { "comment": "Give it a shot" },
  "metadata": { "generatedAt": "…", "processingTime": "…", "clientIp": "…", "version": "2.2-rust", "format": "structured", "cached": false, "credits": "…" }
}
```

The phone area code is derived from the generated city and state, so it matches the
address rather than being random. `timeZone` is sampled independently and will **not**
correlate with the address, as in the example above.

Client IP is read from `CF-Connecting-IP`, then the first entry of `X-Forwarded-For`,
then the socket peer — put it behind a proxy that sets one of those.

**Errors** — `400` on a non-numeric BIN, `404` when a BIN is not in the database, `500`
on a database failure. All share one shape:

```json
{ "success": false, "error": "Invalid BIN format. Must be numeric." }
```

## Configuration

Read from the environment, or a `.env` file in the working directory.

| Variable       | Default                     | Purpose                     |
|----------------|-----------------------------|-----------------------------|
| `MONGODB_URI`  | `mongodb://localhost:27017` | Connection string           |
| `MONGODB_DB`   | `random_server`             | Database name               |
| `HOST`         | `127.0.0.1`                 | Bind address                |
| `PORT`         | `8080`                      | Bind port                   |
| `RUST_LOG`     | `info`                      | Log filter (`env_logger`)   |

The server verifies the database is reachable at startup and exits if it is not, so a
bad URI fails immediately rather than on the first request.

## Database

Eleven collections, all read-only at runtime:

| Collection       | Used by            | Contents                        |
|------------------|--------------------|---------------------------------|
| `bin_data`       | `/bin/*`           | BIN records keyed by `_id`      |
| `first_name`     | `/randomdatav2`    | Given names                     |
| `last_name`      | `/randomdatav2`    | Surnames                        |
| `words`          | `/randomdatav2`    | Email username fragments        |
| `domains`        | `/randomdatav2`    | Email domains                   |
| `addresses`      | `/randomdatav2`    | Street addresses by country     |
| `countries`      | `/randomdatav2`    | Country and state/region names  |
| `phone_prefixes` | `/randomdatav2`    | NPA/NXX by city, state, country |
| `timezone`       | `/randomdatav2`    | Timezone names and offsets      |
| `comments`       | `/randomdatav2`    | Filler comment strings          |
| `user_agents`    | `/randomdatav2`    | Browser user-agent strings      |

`addresses` currently holds US and CA rows only. Requesting a country with no rows
returns a record with empty address fields — the lookup deliberately does not
substitute a different country.

### Moving data between servers

Two helper binaries. Both take the URI as the first argument and read `MONGODB_DB`
(default `random_server`).

```sh
# export the eleven collections -> ./data/<collection>.json
cargo run --bin dump -- 'mongodb://user:pass@host:27017/'

# dump takes the source database from MONGODB_DB, so override it to read
# from a differently-named source
MONGODB_DB=some_other_db cargo run --bin dump -- 'mongodb://user:pass@host:27017/'

# load ./data into another server; seed takes the target database as argv[2]
cargo run --bin seed -- 'mongodb://user:pass@newhost:27017/' random_server
```

`seed` skips any collection that already has documents unless you pass `--force`, so a
mistyped host cannot quietly overwrite a populated server. `--dir <path>` reads dumps
from somewhere other than `./data`.

If the data directory has no `*.json` dumps, `seed` fetches `data.zip` and unpacks it
first — so a fresh checkout can populate a database in one command:

```sh
DATA_REPO=owner/name cargo run --bin seed -- 'mongodb://…' random_server
# no dumps in data — fetching data.zip
#   looking up latest release of owner/name
#   downloading https://github.com/…/data.zip
#   got 10.7 MB, unpacking
#   extracted 11 files -> data
```

`DATA_REPO` takes the `data.zip` attached to that repo's **latest release**;
`DATA_URL` points at an archive directly and skips the release lookup. Neither has a
default, so `seed` never silently reaches for the network — with `./data` present
(as it is in this repo) it goes straight to seeding.

Dumps are newline-delimited [canonical extended JSON][extjson], so BSON types survive
a round-trip (`{"$oid":…}`, `{"$numberInt":…}`) and the files also load with
`mongoimport`.

[extjson]: https://www.mongodb.com/docs/manual/reference/mongodb-extended-json/

## Development

```sh
cargo test                              # 17 tests, no database needed
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The tests cover routing, status codes and BIN parsing via `warp::test`, plus phone
formatting and the dump/seed JSON round-trip. None of them touch a real database, so
they run anywhere.

## Releases

[`.github/workflows/release.yml`](.github/workflows/release.yml) runs on every push:
tests and clippy on Linux and Windows, then release builds for
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`.

A fourth job zips `data/` into `data.zip` (~10 MB from 134 MB of JSON). All of it is
uploaded as build artifacts per target, downloadable from the run's summary page.

Pushing a `v*` tag also publishes a GitHub Release with the per-target
`.tar.gz`/`.zip` archives **and `data.zip`** — the asset `seed` looks for when a
checkout has no `data/`:

```sh
git tag v1.0.1 && git push origin v1.0.1
```

`data/` is committed, so the release job always has something to zip. Refresh it with
`cargo run --bin dump` before tagging if the source database has changed.

### Release fails with 403 "Resource not accessible by integration"

The publishing jobs declare `permissions: contents: write`, which is what
`GITHUB_TOKEN` needs to create a Release. If a tag build still 403s, the cause is a
setting outside the workflow file:

1. **Settings → Actions → General → Workflow permissions** — must be *Read and write
   permissions*. If it is set to read-only, that ceiling applies no matter what the
   workflow requests.
2. **Organisation-level policy** — an org can force read-only for all its repos, which
   overrides the per-repo setting above.
3. **Tag protection / rulesets** — a rule covering `v*` can block the release.

A failed release still leaves the tag pushed, so re-running means deleting and
re-pushing it:

```sh
git push --delete origin v1.0.0
git tag -d v1.0.0
git tag v1.0.0
git push origin v1.0.0
```
