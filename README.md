# Jake

A Discord bot that watches company career pages for internship and co-op
postings and pings you when something matching your profile shows up. Written
in Rust (poise/serenity + sqlx), backed by Postgres (Supabase in production),
and light enough to run 24/7 on a Raspberry Pi.

## What it does

- **Multi-user profiles.** Every Discord user has their own filters (seniority
  terms, domain keywords, locations) and their own list of tracked companies.
  A firmware student in Toronto and a software student in New York can share one
  bot and one database.
- **Generic job-board scrapers.** Point it at a careers URL and it works out the
  platform and talks to that platform's JSON API directly, no browser needed:
  - **Workday** (`*.myworkdayjobs.com`) via the `/wday/cxs/.../jobs` endpoints.
    Facets in the URL (e.g. `?jobFamilyGroup=...`) are honoured.
  - **Jibe / iCIMS** career sites (e.g. `careers.amd.com/careers-home/jobs`)
    via `/api/jobs`. Query filters such as `country=Canada` are honoured.
  - **Greenhouse**, **Lever** and **Ashby** boards via their public APIs.
  - Company pages that merely *embed* one of those boards (e.g.
    `asteralabs.com/careers`) are detected by scanning the page.
- **Hand-written scrapers** for Google, Amazon, Jane Street, DoorDash and Stripe
  carried over from the original version.
- **30-minute background scheduler.** Silent unless a new job matches someone's
  profile. `/scrape` runs the same pipeline on demand.
- **Anti-spam.** Jobs are stored once by URL; a `(job, user)` notification row is
  written when a posting is sent, so nobody is pinged twice about the same job.
- **Application tracking.** Each posting has Apply / Ignore buttons. Apply opens
  a thread; `/update` inside the thread moves it through
  ACTIVE → SUBMITTED / REJECTED / DELETED / IGNORED.

## Commands

| Command | Purpose |
| --- | --- |
| `/profile setup [preset] [channel]` | Create or update your profile. Alerts go to the chosen channel (default: where you ran it). `preset: hardware_canada` applies the hardware/firmware/FPGA filters for Toronto, Waterloo, Markham, Vancouver and tracks the seeded chip companies. |
| `/profile show` or `/status` | Show your channel, filters, tracked companies and how many jobs have been posted to you. |
| `/filters set [keywords] [locations] [seniority]` | Replace one or more comma-separated filter lists. Omitted fields are unchanged. |
| `/filters clear <field>` | Empty one list so it no longer restricts matches. |
| `/add-company <url> [name]` | Detect the board behind a URL, store the company and track it. |
| `/track-company <name>` / `/remove-company <name>` | Follow or unfollow a company that is already known. |
| `/companies` | List every known company, with the ones you track marked. |
| `/scrape` | Scrape your tracked companies now and post matches in the current channel. |
| `/update <state>` | Inside a job thread: record a status change. |

### How matching works

A stored job is posted to you when all of the following hold:

1. **Seniority:** the title contains one of your seniority terms as a whole
   word (default `Intern, Internship, Co-op, Apprentice`). Skipped for the
   hand-written scrapers, which already return only internships.
2. **Keywords:** empty means anything; otherwise at least one keyword appears
   as a whole word in the title or description.
3. **Locations:** empty means anywhere; otherwise one of your terms appears in
   the job's parsed locations (city, region or country, by code or full name).
   Jobs with no location listed do not pass a location filter.

Up to 25 postings per user per run are sent, so adding a company with a large
backlog trickles in over a few cycles rather than flooding the channel.

## Seeded companies

| Company | Board | Notes |
| --- | --- | --- |
| AMD | Jibe | Canada-only filter from the seed URL |
| Altera | Workday | Intern/Student job family |
| Intel | Workday | Intern job family (the unfiltered site has thousands of postings) |
| NVIDIA | Workday | University Employment job family |
| onsemi | Workday | Site was returning server errors when seeded; may need its URL corrected |
| Astera Labs | Greenhouse | |
| Etched | Ashby | |

Plus the original CUSTOM scrapers: Google, Amazon, Jane Street, DoorDash, Stripe.

## Local development

Requirements: Rust (stable), a Postgres database, `sqlx-cli`.

```sh
cargo install sqlx-cli --no-default-features --features postgres,native-tls
cp .env.example .env         # fill in DATABASE_URL and DISCORD_TOKEN
sqlx migrate run             # creates tables and seeds companies
cargo run
```

Environment variables (from `.env`):

| Variable | Required | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | yes | Postgres connection string. For Supabase use the IPv4 session pooler on port 5432 unless your network has IPv6. |
| `DISCORD_TOKEN` | yes | Bot token. The bot needs the Message Content intent enabled. |
| `SCRAPE_INTERVAL_MINUTES` | no | Scheduler period, default 30. |

Discord setup: create an application, enable **Message Content Intent** on the
Bot page, and invite it with the `bot` and `applications.commands` scopes plus
permissions to view channels, send messages, embed links, read history and
create public threads.

Tests:

```sh
cargo test                                            # unit tests, no network
cargo test --test live_boards -- --ignored --nocapture  # hit the real boards
BOARD=amd cargo test --test live_boards -- --ignored --nocapture  # one board
```

Infrastructure: `terraform/` provisions the Supabase project. Copy
`terraform.tfvars.example` values into `terraform/terraform.tfvars` (org id,
DB password, access token) and run `terraform init && terraform apply`.

## Raspberry Pi 5 deployment (Raspberry Pi OS, 64-bit)

The bot is a single static-ish binary; the Pi only needs Rust to build it once.
Plan on 15 to 30 minutes for the first release build.

1. **System packages**

   ```sh
   sudo apt update
   sudo apt install -y git build-essential pkg-config libssl-dev
   ```

2. **Rust toolchain**

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
   source "$HOME/.cargo/env"
   ```

3. **Clone and configure**

   ```sh
   git clone <your-repo-url> ~/jake
   cd ~/jake
   cp .env.example .env
   nano .env            # DATABASE_URL, DISCORD_TOKEN
   ```

4. **Database schema** (once per database; can also be done from your laptop)

   ```sh
   cargo install sqlx-cli --no-default-features --features postgres,native-tls
   sqlx migrate run
   ```

5. **Build**

   ```sh
   cargo build --release
   ./target/release/jake      # sanity check, Ctrl-C once you see "Logged in as"
   ```

6. **Run as a service**

   ```sh
   sudo cp deploy/jake.service /etc/systemd/system/jake.service
   # if your user or path differ from pi:/home/pi/jake, edit the unit first
   sudo systemctl daemon-reload
   sudo systemctl enable --now jake
   journalctl -u jake -f
   ```

   The unit restarts the bot 15 seconds after any crash and starts it on boot
   once the network is up.

7. **Updating**

   ```sh
   cd ~/jake && git pull && cargo build --release && sudo systemctl restart jake
   ```

## Adding a company by hand

`/add-company <url>` is the normal route. For a company with a bespoke careers
site that none of the generic scrapers understand, add a module under
`src/scrapers/`, a match arm in `legacy()` in `src/scrapers/mod.rs`, and a
migration inserting the company with `board_type = 'CUSTOM'`.

## Layout

```
src/
  main.rs          bot startup, button handler, scheduler launch
  db.rs            all SQL
  filter.rs        profile matching rules
  pipeline.rs      scrape -> store -> match -> post, shared by /scrape and the scheduler
  scheduler.rs     30-minute loop
  commands/        slash commands (scrape, update, company, profile)
  scrapers/
    detect.rs      URL / HTML -> which board platform
    workday.rs jibe.rs greenhouse.rs lever.rs ashby.rs
    location.rs    "Toronto, Ontario, Canada" -> (Toronto, ON, CA)
    text.rs        HTML stripping, term parsing, pay conversion
    google.rs amazon.rs jane_street.rs doordash.rs stripe.rs   hand-written
migrations/        sqlx migrations (schema + seed data)
deploy/            systemd unit
terraform/         Supabase project
tests/             live board integration test (ignored by default)
```

## Known limitations

- Workday rate-limits aggressive clients. The scraper paginates with a short
  delay and fetches at most four job details at once; don't run many manual
  scrapes back to back.
- Pay is only shown when the board publishes it (Jibe salary tags, Greenhouse
  pay transparency, Lever `salaryRange`, Ashby compensation summaries).
  Workday postings never include pay.
- Location parsing is heuristic. Unrecognised strings are kept as the city
  name so they still show in the embed and still match substring filters.
