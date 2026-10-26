## 🏆 Credits & Acknowledgments

All credit for this project goes to the hard work and big brain of **Oliad Rundassa**. Thank you, sir! 

🔗 **GitHub:** [@oliad1](https://github.com/oliad1)

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
  - **Oracle Cloud HCM** candidate sites (`*.oraclecloud.com/hcmUI/CandidateExperience/...`,
    e.g. onsemi) via their REST API. A `?keyword=Intern` filter in the URL is honoured.
  - Company pages that merely *embed* one of those boards (e.g.
    `asteralabs.com/early-career-programs`) are detected by scanning the page.
  - Analytics junk in a pasted URL (`_gl=`, `utm_*`, `gclid`, ...) is dropped, so
    it is never sent to a board as a filter.
- **Hand-written scrapers** for Google, Amazon, Jane Street, DoorDash and Stripe
  carried over from the original version.
- **30-minute background scheduler.** Silent unless a new job matches someone's
  profile. `/scrape` runs the same pipeline on demand. Scrapes run on their own
  thread with their own database pool and HTTP client, so commands and buttons
  stay responsive while one is running; a run that hangs is cut off after 25
  minutes (`SCRAPE_TIMEOUT_MINUTES`).
- **Anti-spam.** Jobs are stored once by URL; a `(job, user)` notification row is
  written when a posting is sent, so nobody is pinged twice about the same job.
- **Application tracking.** Each posting has Apply / Ignore buttons. Apply opens
  a thread; `/update` inside the thread moves it through
  ACTIVE → SUBMITTED / REJECTED / DELETED / IGNORED.
- **Resume tailoring.** Upload your LaTeX resume once. Clicking **Apply** posts a
  copy tailored to that posting in the job's thread, as a PDF plus `.tex`, with
  a keyword report. See [Resume tailoring](#resume-tailoring).
- **Smarter alerts.** Each alert shows a short preview that starts at the job
  description rather than company boilerplate, then Location, Pay and Term
  side by side, plus **Apply** / **Ignore** / **Open posting** buttons. With a
  resume uploaded it adds a *Resume match* section (score, skills you have,
  skills you're missing), and the card's side bar turns green, amber or red by
  that score. Requirements that can rule you out (US citizenship/ITAR,
  clearance, graduation window, GPA, PhD-only, no sponsorship, French) are
  listed under *Check before applying*.
- **Batches of 10.** Alerts arrive 10 at a time, from both `/scrape` and the
  scheduler. If more matched, a **Show next 10** button posts the next batch
  when you're ready.
- **People to contact.** The Apply thread lists contacts named in the posting
  (recruiter names, emails, team, job ID) and buttons that open searches for
  campus recruiters, the team's leads and alumni of your school, plus a Google
  search. Optionally it shows likely recruiters inline. **Draft message**
  writes a LinkedIn note and a longer message from your resume, and
  `/outreach` tracks who you contacted and reminds you to follow up. See
  [Outreach](#outreach).

## Commands

| Command | Purpose |
| --- | --- |
| `/profile setup [preset] [channel]` | Create or update your profile. Alerts go to the chosen channel (default: where you ran it). `preset: hardware_canada` applies the hardware/firmware/FPGA filters for Toronto, Waterloo, Markham, Vancouver and tracks the seeded chip companies. |
| `/profile show` or `/status` | Show your channel, filters, tracked companies and how many jobs have been posted to you. |
| `/filters set [keywords] [exclude] [locations] [seniority] [keyword_scope]` | Replace one or more comma-separated filter lists. `keyword_scope` is `title_and_description` (default) or `title_only`. Omitted fields are unchanged. |
| `/filters clear <field>` | Empty one list (`keywords`, `exclude`, `locations`, `seniority`) so it no longer restricts matches. |
| `/add-company <url> [name]` | Detect the board behind a URL, store the company and track it. |
| `/track-company <name>` / `/remove-company <name>` | Follow or unfollow a company that is already known. |
| `/companies` | List every known company, with the ones you track marked. |
| `/scrape` | Scrape your tracked companies now and post matches in the current channel. |
| `/update <state>` | Inside a job thread: record a status change. |
| `/resume upload <file>` | Store your resume's `.tex` source (Jake's Resume template or anything using the same macros). It's checked, compiled once and summarised. |
| `/resume show` | What the bot understood: sections, bullets, detected skills. Attaches your stored `.tex`. |
| `/resume tailor [url] [pages]` | Tailor your resume to a job, either inside its thread or by passing the job URL from an alert. |
| `/resume gaps` | The skills your recent job matches ask for most, and which of them your resume doesn't mention. |
| `/resume settings [pages] [auto_tailor]` | Page limit (1 to 3, default 1) and whether Apply tailors automatically (default on). |
| `/resume remove` | Delete your stored resume. |
| `/outreach draft [name]` | Inside a job thread: a LinkedIn note (200 characters max) and a longer message, built from your resume and the posting. Only you see them. |
| `/outreach sent <name> [link] [follow_up_days]` | Inside a job thread: log that you messaged someone. A follow-up reminder with a draft is posted in the thread after 5 days (or your choice), then once more a week later. |
| `/outreach replied [name]` | Inside a job thread: stop reminders for someone who answered. |
| `/outreach list` | Everyone you're still waiting on, with links to each job thread. |

### How matching works

A stored job is posted to you when all of the following hold:

1. **Exclusions:** the title contains none of your exclude terms as a whole
   word (e.g. `Senior, PhD, Sales`). Only the title is checked, since words
   like these appear in nearly every description.
2. **Seniority:** the title contains one of your seniority terms as a whole
   word (default `Intern, Internship, Co-op, Apprentice`). Skipped for the
   hand-written scrapers, which already return only internships.
3. **Keywords:** empty means anything; otherwise at least one keyword appears
   as a whole word in the title or description. With
   `keyword_scope: title_only` the description is ignored, which cuts out
   postings that only mention your field in passing.
4. **Locations:** empty means anywhere; otherwise one of your terms appears in
   the job's parsed locations (city, region or country, by code or full name).
   Jobs with no location listed do not pass a location filter.

Matches are posted 10 at a time per user. Anything beyond that waits behind a
**Show next 10** button in the same channel, and the next scheduled run also
sends the next 10, so a company with a big backlog never floods the channel.

Each profile remembers the newest job it has already been checked against, so
a scheduled run only loads jobs stored since the previous one. Changing your
filters or tracking a company resets this, and the next run re-checks every
stored job against the new settings.

## Resume tailoring

Keep one **master resume** containing every bullet and project you have, even
if it runs to two or three pages. For each job, the bot:

1. Reads the posting and pulls out the skills it asks for, from a built-in list
   of about 140 hardware, firmware and software skills plus anything listed on
   your resume. A skill under "Requirements" or "Minimum qualifications" counts
   more than one under "Nice to have", and a skill in the job title counts most.
2. Scores each of your bullets on those skills, shared words and phrases with
   the posting, and whether it has numbers.
3. Within each job, orders bullets most-relevant first. Projects are reordered
   too, but a project's first bullet stays first since it says what the project
   is. Matching skills move to the front of each skills line.
4. Compiles with `pdflatex`. If the result is over your page limit, it removes
   the least relevant content (whole off-topic projects first, then weak
   bullets from the longest lists) until it fits, using as few removals as
   possible.
5. Posts the PDF, the `.tex` and a report: keyword match before and after
   tailoring, required skills you're missing, and what was moved or cut.

It only reorders and removes your own content; it never writes new text or
claims skills you don't have. The "Not on your resume" list is for you: if
you really have that skill, add it to a bullet in your master copy.

Guaranteed rules: Education is never changed, each job keeps at least two
bullets, each project keeps its first bullet, and at least two projects stay.

**With Gemini.** If `GEMINI_API_KEY` is set, steps 1 to 4 are done by Gemini
instead, and it goes further than the built-in scorer. The bot sends the master
`.tex`, the posting and the list of keywords it extracted (required first), and
asks Gemini to optimise the resume for that posting the way an ATS specialist
would: work the posting's exact keywords and phrasing into the skills lines and
bullets wherever the resume has evidence for them, rewrite bullets to lead with
what the posting cares about, reorder bullets, projects, skills and (where the
template allows) sections by relevance, then trim to the page limit. The hard
limits stay: nothing is invented, a keyword is only added where the master
resume shows the candidate used it or a direct equivalent, and the header and
Education are untouched. The returned `.tex` goes through the same safety checks
and is compiled in the sandbox. If the result is over the page limit or doesn't
compile, Gemini is told and asked again, up to three times. The report compares
keyword coverage before and after and lists the keywords that are newly
mentioned, so you can check each one is true of you before sending. The
"moved/cut" details only exist for the built-in scorer. If the API call fails,
the built-in tailoring runs and the report says so.

Optional markers, written as LaTeX comments so they never show in the PDF:

```latex
% @keep
\resumeItem{Wrote C drivers for I2C and SPI sensors on an STM32}   % never removed
% @tags: UVM, Verification
\resumeItem{Built a constrained-random testbench reaching 98\% coverage}  % extra keywords
```

`@keep` and `@tags:` also work on the line above a `\resumeProjectHeading`.
[docs/master_resume_example.tex](docs/master_resume_example.tex) is a complete
two-page master you can copy from.

**Security.** Uploaded LaTeX is untrusted code compiled on the bot's host, so
pdflatex runs inside a [bubblewrap](https://github.com/containers/bubblewrap)
sandbox. Only the TeX installation (read-only) and a scratch folder exist
inside it: no home directory, no `.env`, no network. It also runs with shell
escape disabled, a 45 s timeout and one compile at a time. Commands that read
files or run programs (`\write18`, `\input` with a path, `\InputIfFileExists`,
`\includegraphics`, ...) are rejected at upload with a clear message. TeX's own
`openin_any=p` setting is also applied, but on its own it did *not* stop file
reads in testing, which is why the sandbox is required. Without bubblewrap the
bot won't compile resumes (you still get the tailored `.tex` and the report)
unless you set `RESUME_SANDBOX=off`, which is only reasonable if you're the
bot's only user.

## Outreach

Clicking **Apply** opens a thread with three things:

1. **Application started:** the steps to follow (tailored resume, apply on the
   company site, reach out, `/update SUBMITTED`).
2. **People to contact:**
   - **From the posting:** recruiter names ("Recruiter: Jane Doe"), non-generic
     emails, the team ("join the Power Solutions Group") and the job ID, from
     the text or the Workday URL. None of the supported job boards publish the
     recruiter in their data, so the text is the only source.
   - **Search buttons** that open in *your* browser, logged in as you: campus
     recruiters at the company, leads for the role's area ("Embedded Firmware"
     from "Embedded Firmware Developer Intern"), alumni of the school on your
     resume who work there, and a Google search limited to LinkedIn profiles in
     the job's province or country. The bot never scrapes LinkedIn and never
     sends messages for you.
   - **Likely recruiters (optional):** with `BRAVE_API_KEY` set, one Brave
     Search API query per company and region lists up to five profiles inline.
     Results are cached for 14 days, so it costs about one query per company
     every two weeks. Only name, headline and URL are stored.
   - **Draft message:** a LinkedIn connection note within the 200-character
     limit for free accounts, and a longer message for InMail or email. The
     drafts mention the job ID, your most relevant resume bullet ("At onsemi, I
     architected bare-metal C drivers…") and the skills the posting asks for that
     you have. Everything comes from your resume; edit it and send it yourself.
3. **Your tailored resume** (if you've uploaded one).

After messaging someone, run `/outreach sent name:<who>` in the thread. If you
haven't marked a reply with `/outreach replied` within 5 days (configurable),
the bot posts a reminder with a short follow-up draft. It reminds once more a
week later, then stops. `/outreach list` shows everyone you're waiting on.

## Seeded companies

| Company | Board | Notes |
| --- | --- | --- |
| AMD | Jibe | Canada-only filter from the seed URL |
| Altera | Workday | Intern/Student job family |
| Intel | Workday | Intern job family (the unfiltered site has thousands of postings) |
| NVIDIA | Workday | University Employment job family |
| onsemi | Oracle HCM | `keyword=Intern` search on onsemi's Oracle careers site (they left Workday) |
| Astera Labs | Greenhouse | Separate early-career board (`asteraearlycareer2027`): interns and new grads, Toronto, Vancouver, Ottawa |
| Etched | Ashby | |
| Marvell | Workday | Intern category; Toronto, Ottawa |
| Cadence | Workday | Intern category; Ontario, Quebec |
| Ciena | Workday | Intern category; Ottawa HQ |
| Analog Devices | Workday | Co-op/Intern category; Toronto, Ottawa, Vancouver |
| Microchip | Workday | Student/Intern category; Burnaby, Ottawa, Toronto |
| General Motors | Workday | Intern and Co-op categories; Markham technical centre |
| BlackBerry | Workday | Whole board (small); QNX in Ottawa and Waterloo |
| Tenstorrent | Greenhouse | Toronto; the university page lists the same board |
| Lightmatter | Greenhouse | Toronto; most postings list pay |
| Cerebras | Ashby | Toronto |
| Kepler Communications | Lever | Toronto |

The `hardware_canada` preset tracks all of these. Profiles that already used
the preset start tracking the newer ones automatically when the migration runs.

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
| `SCRAPE_TIMEOUT_MINUTES` | no | A scrape running longer than this is abandoned, default 25. |
| `SCRAPER_THREADS` | no | Worker threads for the scraper runtime, default 1. Raise it on a machine with spare cores. |
| `PDFLATEX` | no | Path to `pdflatex` for resume tailoring; defaults to `pdflatex` on `PATH`. Without it, tailored resumes come as `.tex` only. |
| `RESUME_SANDBOX` | no | Set to `off` to compile resumes without bubblewrap. Only do this if you're the bot's only user. |
| `BWRAP` | no | Path to `bwrap` if it isn't on `PATH`. |
| `GEMINI_API_KEY` | no | [Gemini API](https://ai.google.dev/) key. When set, resume tailoring is done by Gemini (see below); without it the built-in scorer is used. |
| `GEMINI_MODEL` | no | Gemini model name, default `gemini-3.5-flash`. |
| `BRAVE_API_KEY` | no | [Brave Search API](https://brave.com/search/api/) key. Shows likely recruiters inline in Apply threads; without it you still get the search buttons. (Google's Custom Search API isn't an option: it's closed to new customers and shuts down January 1, 2027.) |

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
   # LaTeX + sandbox for resume tailoring (covers everything Jake's Resume template uses)
   sudo apt install -y --no-install-recommends texlive-latex-base texlive-latex-recommended \
       texlive-latex-extra texlive-fonts-recommended bubblewrap
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
  flags.rs         eligibility red flags (citizenship, clearance, grad year, GPA...)
  contacts.rs      contacts in postings, people-search links, Brave search + cache
  outreach.rs      LinkedIn note / message / follow-up drafts
  pipeline.rs      scrape -> store -> match -> post, shared by /scrape and the scheduler
  scheduler.rs     30-minute loop
  commands/        slash commands (scrape, update, company, profile, resume, outreach)
  resume/
    latex.rs       parse Jake's Resume .tex; reorder/remove bullets, projects, skills
    skills.rs      skill dictionary and posting skill extraction
    compile.rs     sandboxed pdflatex runner, page counting
    mod.rs         scoring, fit-to-page search, keyword report, gap analysis
  scrapers/
    detect.rs      URL / HTML -> which board platform
    workday.rs jibe.rs greenhouse.rs lever.rs ashby.rs
    location.rs    "Toronto, Ontario, Canada" -> (Toronto, ON, CA)
    text.rs        HTML stripping, term parsing, pay conversion
    google.rs amazon.rs jane_street.rs doordash.rs stripe.rs   hand-written
migrations/        sqlx migrations (schema + seed data)
docs/              example master resume
deploy/            systemd unit
terraform/         Supabase project
tests/             live board integration test (ignored by default)
```

## Known limitations

- Companies are scraped four at a time and the database pool is capped at five
  connections, which keeps a Raspberry Pi and the Supabase pooler comfortable.
- Workday rate-limits aggressive clients. The scraper paginates with a short
  delay and fetches at most four job details at once; don't run many manual
  scrapes back to back.
- Pay is only shown when the board publishes it (Jibe salary tags, Greenhouse
  pay transparency, Lever `salaryRange`, Ashby compensation summaries).
  Workday postings never include pay.
- Location parsing is heuristic. Unrecognised strings are kept as the city
  name so they still show in the embed and still match substring filters.
