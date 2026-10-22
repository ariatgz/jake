//! All database access. Runtime-checked queries keep this file independent of a
//! live database at compile time.

use crate::{City, Error, ScrapeEvent, Term};
use sqlx::PgPool;
use std::collections::HashSet;

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Company {
    pub id: i64,
    pub display_name: String,
    pub url: String,
    pub hex_code: String,
    pub icon_url: String,
    pub board_type: String, // CUSTOM, WORKDAY, JIBE, GREENHOUSE, LEVER, ASHBY
    pub board_url: Option<String>,
}

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Profile {
    pub user_id: i64,
    pub display_name: String,
    pub channel_id: i64,
    pub keywords: Vec<String>,
    pub seniority: Vec<String>,
    pub locations: Vec<String>,
    pub exclude_keywords: Vec<String>,
    pub keywords_title_only: bool,
    /// Jobs with an id at or below this have already been checked against the filters.
    pub evaluated_through: i64,
    /// Bumped whenever `evaluated_through` is reset, so a run that started before
    /// the reset doesn't advance it again.
    pub filters_version: i64,
}

/// Changes to apply to a profile's filters; `None` leaves a field unchanged.
#[derive(Default)]
pub struct FilterUpdate<'a> {
    pub keywords: Option<&'a [String]>,
    pub exclude_keywords: Option<&'a [String]>,
    pub seniority: Option<&'a [String]>,
    pub locations: Option<&'a [String]>,
    pub keywords_title_only: Option<bool>,
}

macro_rules! profile_columns {
    () => {
        "user_id, display_name, channel_id, keywords, seniority, locations, exclude_keywords, keywords_title_only, evaluated_through, filters_version"
    };
}

/// A stored job that a given profile has not been notified about yet.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub application_id: i64,
    pub company_id: i64,
    pub board_type: String,
    pub job_title: String,
    pub url: String,
    pub page_content: String,
    pub lower_wage_cents: Option<i16>,
    pub upper_wage_cents: Option<i16>,
    pub currency: String,
    pub cities: Vec<City>,
    pub terms: Vec<Term>,
}

#[derive(sqlx::FromRow)]
struct CandidateRow {
    application_id: i64,
    company_id: i64,
    board_type: String,
    job_title: String,
    url: String,
    page_content: String,
    lower_wage_cents: Option<i16>,
    upper_wage_cents: Option<i16>,
    currency: String,
    cities: serde_json::Value,
    terms: serde_json::Value,
}

// Profiles -----------------------------------------------------------------

pub async fn all_profiles(pool: &PgPool) -> Result<Vec<Profile>, Error> {
    Ok(sqlx::query_as(concat!("SELECT ", profile_columns!(), " FROM profiles ORDER BY user_id"))
        .fetch_all(pool)
        .await?)
}

pub async fn profile(pool: &PgPool, user_id: i64) -> Result<Option<Profile>, Error> {
    Ok(sqlx::query_as(concat!("SELECT ", profile_columns!(), " FROM profiles WHERE user_id = $1"))
        .bind(user_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn upsert_profile(pool: &PgPool, user_id: i64, display_name: &str, channel_id: i64) -> Result<Profile, Error> {
    Ok(sqlx::query_as(concat!(
        r#"
        INSERT INTO profiles (user_id, display_name, channel_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (user_id) DO UPDATE
            SET display_name = EXCLUDED.display_name,
                channel_id = EXCLUDED.channel_id,
                update_time = NOW()
        RETURNING "#,
        profile_columns!()
    ))
    .bind(user_id)
    .bind(display_name)
    .bind(channel_id)
    .fetch_one(pool)
    .await?)
}

/// Applies the `Some` fields of `update` and makes the next run re-check every
/// stored job against the new filters.
pub async fn set_profile_filters(pool: &PgPool, user_id: i64, update: &FilterUpdate<'_>) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE profiles
        SET keywords = COALESCE($2, keywords),
            exclude_keywords = COALESCE($3, exclude_keywords),
            seniority = COALESCE($4, seniority),
            locations = COALESCE($5, locations),
            keywords_title_only = COALESCE($6, keywords_title_only),
            evaluated_through = 0,
            filters_version = filters_version + 1,
            update_time = NOW()
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .bind(update.keywords)
    .bind(update.exclude_keywords)
    .bind(update.seniority)
    .bind(update.locations)
    .bind(update.keywords_title_only)
    .execute(pool)
    .await?;
    Ok(())
}

/// Records that every job up to `through` has been checked for this profile.
/// Does nothing if the filters were reset since `filters_version` was read.
pub async fn advance_evaluated_through(pool: &PgPool, user_id: i64, filters_version: i64, through: i64) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE profiles
        SET evaluated_through = GREATEST(evaluated_through, $3)
        WHERE user_id = $1 AND filters_version = $2
        "#,
    )
    .bind(user_id)
    .bind(filters_version)
    .bind(through)
    .execute(pool)
    .await?;
    Ok(())
}

// Companies ----------------------------------------------------------------

pub async fn all_companies(pool: &PgPool) -> Result<Vec<Company>, Error> {
    Ok(sqlx::query_as("SELECT id, display_name, url, hex_code, icon_url, board_type, board_url FROM companies WHERE state = 'ACTIVE' ORDER BY display_name")
        .fetch_all(pool)
        .await?)
}

pub async fn tracked_companies(pool: &PgPool, user_id: i64) -> Result<Vec<Company>, Error> {
    Ok(sqlx::query_as(r#"
        SELECT id, display_name, url, hex_code, icon_url, board_type, board_url
        FROM companies AS c
        JOIN profile_companies AS pc ON pc.company_id = c.id
        WHERE pc.user_id = $1 AND c.state = 'ACTIVE'
        ORDER BY c.display_name
        "#)
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// Every active company tracked by at least one of the given profiles.
pub async fn companies_for_profiles(pool: &PgPool, user_ids: &[i64]) -> Result<Vec<Company>, Error> {
    Ok(sqlx::query_as(r#"
        SELECT DISTINCT id, display_name, url, hex_code, icon_url, board_type, board_url
        FROM companies AS c
        JOIN profile_companies AS pc ON pc.company_id = c.id
        WHERE pc.user_id = ANY($1) AND c.state = 'ACTIVE'
        ORDER BY display_name
        "#)
    .bind(user_ids)
    .fetch_all(pool)
    .await?)
}

pub async fn company_by_board_url(pool: &PgPool, board_url: &str) -> Result<Option<Company>, Error> {
    Ok(sqlx::query_as("SELECT id, display_name, url, hex_code, icon_url, board_type, board_url FROM companies WHERE board_url = $1")
        .bind(board_url)
        .fetch_optional(pool)
        .await?)
}

pub async fn company_by_name(pool: &PgPool, name: &str) -> Result<Option<Company>, Error> {
    Ok(sqlx::query_as("SELECT id, display_name, url, hex_code, icon_url, board_type, board_url FROM companies WHERE LOWER(display_name) = LOWER($1) LIMIT 1")
        .bind(name)
        .fetch_optional(pool)
        .await?)
}

pub async fn insert_company(
    pool: &PgPool,
    display_name: &str,
    url: &str,
    hex_code: &str,
    icon_url: &str,
    board_type: &str,
    board_url: &str,
) -> Result<Company, Error> {
    Ok(sqlx::query_as(r#"
        INSERT INTO companies (display_name, url, hex_code, icon_url, board_type, board_url)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (url) DO UPDATE
            SET board_type = EXCLUDED.board_type,
                board_url = EXCLUDED.board_url,
                state = 'ACTIVE'
        RETURNING id, display_name, url, hex_code, icon_url, board_type, board_url
        "#)
    .bind(display_name)
    .bind(url)
    .bind(hex_code)
    .bind(icon_url)
    .bind(board_type)
    .bind(board_url)
    .fetch_one(pool)
    .await?)
}

/// Returns true when the company was newly added to the profile. A new company
/// may already have stored jobs, so the profile's evaluation watermark is reset.
pub async fn track_company(pool: &PgPool, user_id: i64, company_id: i64) -> Result<bool, Error> {
    let result = sqlx::query(
        r#"
        WITH inserted AS (
            INSERT INTO profile_companies (user_id, company_id) VALUES ($1, $2)
            ON CONFLICT DO NOTHING
            RETURNING user_id
        )
        UPDATE profiles
        SET evaluated_through = 0, filters_version = filters_version + 1
        WHERE user_id IN (SELECT user_id FROM inserted)
        "#,
    )
    .bind(user_id)
    .bind(company_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn untrack_company(pool: &PgPool, user_id: i64, company_id: i64) -> Result<bool, Error> {
    let result = sqlx::query("DELETE FROM profile_companies WHERE user_id = $1 AND company_id = $2")
        .bind(user_id)
        .bind(company_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// Applications -------------------------------------------------------------

pub async fn seen_urls(pool: &PgPool, company_id: i64) -> Result<HashSet<String>, Error> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT url FROM applications WHERE company_id = $1")
        .bind(company_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(url,)| url).collect())
}

/// Stores a scraped job with its cities and terms. Returns `None` when the URL
/// was already stored (nothing is written in that case).
pub async fn insert_application(pool: &PgPool, event: &ScrapeEvent) -> Result<Option<i64>, Error> {
    let app = &event.application;
    let terms = event.terms.clone().unwrap_or_default();

    let row: Option<(i64,)> = sqlx::query_as(
        r#"
        WITH application_row AS (
            INSERT INTO applications (
                company_id, job_title, url, page_content,
                lower_wage_cents, upper_wage_cents, currency
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (url) DO NOTHING
            RETURNING id
        ),
        input_cities AS (
            SELECT display_name, region, country
            FROM jsonb_to_recordset($8::jsonb) AS t(display_name text, region text, country text)
        ),
        new_cities AS (
            INSERT INTO cities (display_name, region, country)
            SELECT display_name, region, country FROM input_cities
            ON CONFLICT (display_name, region, country) DO UPDATE
                SET display_name = EXCLUDED.display_name
            RETURNING id
        ),
        new_application_cities AS (
            INSERT INTO application_cities (application_id, city_id)
            SELECT ar.id, nc.id FROM application_row AS ar CROSS JOIN new_cities AS nc
        ),
        input_terms AS (
            SELECT display_name
            FROM jsonb_to_recordset($9::jsonb) AS t(display_name text)
        ),
        new_terms AS (
            INSERT INTO terms (display_name)
            SELECT display_name FROM input_terms
            ON CONFLICT (display_name) DO UPDATE
                SET display_name = EXCLUDED.display_name
            RETURNING id
        ),
        new_application_terms AS (
            INSERT INTO application_terms (application_id, term_id)
            SELECT ar.id, nt.id FROM application_row AS ar CROSS JOIN new_terms AS nt
        )
        INSERT INTO application_events (application_id, after_state)
        SELECT ar.id, 'ACTIVE' FROM application_row AS ar
        RETURNING application_id
        "#,
    )
    .bind(app.company_id)
    .bind(&app.job_title)
    .bind(&app.url)
    .bind(&app.page_content)
    .bind(app.lower_wage_cents)
    .bind(app.upper_wage_cents)
    .bind(&app.currency)
    .bind(serde_json::to_value(&event.cities)?)
    .bind(serde_json::to_value(&terms)?)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(id,)| id))
}

/// Jobs from the given companies, stored after `after_id`, that this user has
/// never been notified about.
pub async fn candidates_for_profile(pool: &PgPool, user_id: i64, company_ids: &[i64], after_id: i64) -> Result<Vec<Candidate>, Error> {
    let rows: Vec<CandidateRow> = sqlx::query_as(
        r#"
        SELECT
            a.id AS application_id,
            a.company_id,
            c.board_type,
            a.job_title,
            a.url,
            a.page_content,
            a.lower_wage_cents,
            a.upper_wage_cents,
            a.currency,
            COALESCE((
                SELECT json_agg(json_build_object('display_name', ci.display_name, 'region', ci.region, 'country', ci.country))
                FROM application_cities AS ac
                JOIN cities AS ci ON ci.id = ac.city_id
                WHERE ac.application_id = a.id
            ), '[]'::json) AS cities,
            COALESCE((
                SELECT json_agg(json_build_object('display_name', t.display_name))
                FROM application_terms AS at
                JOIN terms AS t ON t.id = at.term_id
                WHERE at.application_id = a.id
            ), '[]'::json) AS terms
        FROM applications AS a
        JOIN companies AS c ON c.id = a.company_id
        WHERE a.company_id = ANY($2)
          AND a.id > $3
          AND NOT EXISTS (
            SELECT 1 FROM notifications AS n
            WHERE n.application_id = a.id AND n.user_id = $1
          )
        ORDER BY a.id DESC
        "#,
    )
    .bind(user_id)
    .bind(company_ids)
    .bind(after_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(Candidate {
                application_id: r.application_id,
                company_id: r.company_id,
                board_type: r.board_type,
                job_title: r.job_title,
                url: r.url,
                page_content: r.page_content,
                lower_wage_cents: r.lower_wage_cents,
                upper_wage_cents: r.upper_wage_cents,
                currency: r.currency,
                cities: serde_json::from_value(r.cities)?,
                terms: serde_json::from_value(r.terms)?,
            })
        })
        .collect()
}

// Notifications ------------------------------------------------------------

pub async fn record_notification(pool: &PgPool, application_id: i64, user_id: i64, channel_id: i64, message_id: i64) -> Result<(), Error> {
    sqlx::query(
        r#"
        INSERT INTO notifications (application_id, user_id, channel_id, message_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (application_id, user_id) DO NOTHING
        "#,
    )
    .bind(application_id)
    .bind(user_id)
    .bind(channel_id)
    .bind(message_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn notification_owner(pool: &PgPool, message_id: u64) -> Result<Option<i64>, Error> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM notifications WHERE message_id = $1")
        .bind(message_id as i64)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(id,)| id))
}

/// Moves a posted job to a new state and logs the transition. Returns the
/// previous state, or `None` when no posting matches the message/thread id.
pub async fn update_notification_state(pool: &PgPool, message_id: u64, after_state: &str) -> Result<Option<String>, Error> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        WITH old AS (
            SELECT id, application_id, user_id, state
            FROM notifications
            WHERE message_id = $1
        ),
        updated AS (
            UPDATE notifications AS n
            SET state = $2
            FROM old
            WHERE n.id = old.id
        )
        INSERT INTO application_events (application_id, before_state, after_state, user_id)
        SELECT application_id, state, $2, user_id FROM old
        RETURNING before_state
        "#,
    )
    .bind(message_id as i64)
    .bind(after_state)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(state,)| state))
}

pub async fn notification_counts(pool: &PgPool, user_id: i64) -> Result<Vec<(String, i64)>, Error> {
    Ok(sqlx::query_as("SELECT state, COUNT(*) FROM notifications WHERE user_id = $1 GROUP BY state ORDER BY state")
        .bind(user_id)
        .fetch_all(pool)
        .await?)
}

// Resumes ------------------------------------------------------------------

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct StoredResume {
    pub file_name: String,
    pub tex: String,
    pub max_pages: i16,
    pub auto_tailor: bool,
}

pub async fn resume(pool: &PgPool, user_id: i64) -> Result<Option<StoredResume>, Error> {
    Ok(sqlx::query_as("SELECT file_name, tex, max_pages, auto_tailor FROM resumes WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn save_resume(pool: &PgPool, user_id: i64, file_name: &str, tex: &str) -> Result<(), Error> {
    sqlx::query(
        r#"
        INSERT INTO resumes (user_id, file_name, tex) VALUES ($1, $2, $3)
        ON CONFLICT (user_id) DO UPDATE
            SET file_name = EXCLUDED.file_name, tex = EXCLUDED.tex, update_time = NOW()
        "#,
    )
    .bind(user_id)
    .bind(file_name)
    .bind(tex)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_resume_settings(pool: &PgPool, user_id: i64, max_pages: Option<i16>, auto_tailor: Option<bool>) -> Result<bool, Error> {
    let result = sqlx::query(
        "UPDATE resumes SET max_pages = COALESCE($2, max_pages), auto_tailor = COALESCE($3, auto_tailor) WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(max_pages)
    .bind(auto_tailor)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_resume(pool: &PgPool, user_id: i64) -> Result<bool, Error> {
    let result = sqlx::query("DELETE FROM resumes WHERE user_id = $1").bind(user_id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}

/// A stored posting, with what's needed to tailor a resume or find contacts for it.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct JobDetail {
    pub job_title: String,
    pub company_name: String,
    pub url: String,
    pub page_content: String,
    pub cities: sqlx::types::Json<Vec<City>>,
}

/// The posting behind a job message (or the thread opened from it), plus the
/// user it was posted for.
pub async fn job_for_message(pool: &PgPool, message_id: u64) -> Result<Option<(JobDetail, i64)>, Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        job: JobDetail,
        user_id: i64,
    }
    let row: Option<Row> = sqlx::query_as(
        r#"
        SELECT a.job_title, c.display_name AS company_name, a.url, a.page_content, n.user_id,
            COALESCE((
                SELECT jsonb_agg(jsonb_build_object('display_name', ci.display_name, 'region', ci.region, 'country', ci.country))
                FROM application_cities AS ac JOIN cities AS ci ON ci.id = ac.city_id
                WHERE ac.application_id = a.id
            ), '[]'::jsonb) AS cities
        FROM notifications AS n
        JOIN applications AS a ON a.id = n.application_id
        JOIN companies AS c ON c.id = a.company_id
        WHERE n.message_id = $1
        "#,
    )
    .bind(message_id as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.job, r.user_id)))
}

pub async fn job_by_url(pool: &PgPool, url: &str) -> Result<Option<JobDetail>, Error> {
    Ok(sqlx::query_as(
        r#"
        SELECT a.job_title, c.display_name AS company_name, a.url, a.page_content,
            COALESCE((
                SELECT jsonb_agg(jsonb_build_object('display_name', ci.display_name, 'region', ci.region, 'country', ci.country))
                FROM application_cities AS ac JOIN cities AS ci ON ci.id = ac.city_id
                WHERE ac.application_id = a.id
            ), '[]'::jsonb) AS cities
        FROM applications AS a
        JOIN companies AS c ON c.id = a.company_id
        WHERE a.url = $1
        "#,
    )
    .bind(url.trim())
    .fetch_optional(pool)
    .await?)
}

/// Titles and descriptions of the most recent jobs posted to a user, skipping
/// ones they ignored. Used for the skills-gap report.
pub async fn recent_posted_jobs(pool: &PgPool, user_id: i64, limit: i64) -> Result<Vec<(String, String)>, Error> {
    Ok(sqlx::query_as(
        r#"
        SELECT a.job_title, a.page_content
        FROM notifications AS n
        JOIN applications AS a ON a.id = n.application_id
        WHERE n.user_id = $1 AND n.state <> 'IGNORED'
        ORDER BY n.id DESC
        LIMIT $2
        "#,
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

// Outreach -----------------------------------------------------------------

/// The notification a job thread belongs to, and who it was posted for.
pub async fn notification_for_thread(pool: &PgPool, thread_id: u64) -> Result<Option<(i64, i64)>, Error> {
    Ok(sqlx::query_as("SELECT id, user_id FROM notifications WHERE message_id = $1")
        .bind(thread_id as i64)
        .fetch_optional(pool)
        .await?)
}

/// Logs a message the user sent; returns the first reminder time (unix seconds).
pub async fn insert_outreach(
    pool: &PgPool,
    notification_id: i64,
    user_id: i64,
    contact_name: &str,
    contact_url: Option<&str>,
    follow_up_days: i32,
) -> Result<i64, Error> {
    let (at,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO outreach (notification_id, user_id, contact_name, contact_url, follow_up_at)
        VALUES ($1, $2, $3, $4, NOW() + make_interval(days => $5))
        RETURNING EXTRACT(EPOCH FROM follow_up_at)::BIGINT
        "#,
    )
    .bind(notification_id)
    .bind(user_id)
    .bind(contact_name)
    .bind(contact_url)
    .bind(follow_up_days)
    .fetch_one(pool)
    .await?;
    Ok(at)
}

/// Marks the user's most recent open outreach in this thread (optionally only
/// to contacts whose name contains `name`) as replied. Returns the contact name.
pub async fn mark_outreach_replied(pool: &PgPool, notification_id: i64, user_id: i64, name: Option<&str>) -> Result<Option<String>, Error> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        UPDATE outreach SET state = 'REPLIED', follow_up_at = NULL
        WHERE id = (
            SELECT id FROM outreach
            WHERE notification_id = $1 AND user_id = $2 AND state = 'WAITING'
              AND ($3::TEXT IS NULL OR contact_name ILIKE '%' || $3 || '%')
            ORDER BY sent_at DESC
            LIMIT 1
        )
        RETURNING contact_name
        "#,
    )
    .bind(notification_id)
    .bind(user_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(n,)| n))
}

#[derive(sqlx::FromRow, Debug)]
pub struct OpenOutreach {
    pub contact_name: String,
    pub contact_url: Option<String>,
    pub job_title: String,
    pub company_name: String,
    pub thread_id: i64,
    pub sent_unix: i64,
    pub follow_up_unix: Option<i64>,
}

pub async fn open_outreach(pool: &PgPool, user_id: i64) -> Result<Vec<OpenOutreach>, Error> {
    Ok(sqlx::query_as(
        r#"
        SELECT o.contact_name, o.contact_url, a.job_title, c.display_name AS company_name, n.message_id AS thread_id,
            EXTRACT(EPOCH FROM o.sent_at)::BIGINT AS sent_unix,
            EXTRACT(EPOCH FROM o.follow_up_at)::BIGINT AS follow_up_unix
        FROM outreach AS o
        JOIN notifications AS n ON n.id = o.notification_id
        JOIN applications AS a ON a.id = n.application_id
        JOIN companies AS c ON c.id = a.company_id
        WHERE o.user_id = $1 AND o.state = 'WAITING'
        ORDER BY o.follow_up_at NULLS LAST, o.sent_at
        LIMIT 25
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

#[derive(sqlx::FromRow, Debug)]
pub struct DueFollowUp {
    pub id: i64,
    pub user_id: i64,
    pub thread_id: i64,
    pub contact_name: String,
    pub job_title: String,
    pub company_name: String,
    pub follow_ups_sent: i16,
    pub days_since: i32,
    pub sender: String,
}

pub async fn due_follow_ups(pool: &PgPool) -> Result<Vec<DueFollowUp>, Error> {
    Ok(sqlx::query_as(
        r#"
        SELECT o.id, o.user_id, n.message_id AS thread_id, o.contact_name, a.job_title,
            c.display_name AS company_name, o.follow_ups_sent,
            EXTRACT(DAY FROM NOW() - o.sent_at)::INT AS days_since,
            COALESCE(p.display_name, '') AS sender
        FROM outreach AS o
        JOIN notifications AS n ON n.id = o.notification_id
        JOIN applications AS a ON a.id = n.application_id
        JOIN companies AS c ON c.id = a.company_id
        LEFT JOIN profiles AS p ON p.user_id = o.user_id
        WHERE o.state = 'WAITING' AND o.follow_up_at <= NOW()
        ORDER BY o.follow_up_at
        LIMIT 20
        "#,
    )
    .fetch_all(pool)
    .await?)
}

/// After a reminder: one more a week later, then stop.
pub async fn record_follow_up_reminder(pool: &PgPool, id: i64) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE outreach
        SET follow_ups_sent = follow_ups_sent + 1,
            follow_up_at = CASE WHEN follow_ups_sent + 1 >= 2 THEN NULL ELSE NOW() + INTERVAL '7 days' END
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn cached_search(pool: &PgPool, query: &str, max_age_days: i32) -> Result<Option<serde_json::Value>, Error> {
    let row: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT results FROM contact_search_cache WHERE query = $1 AND fetched_at > NOW() - make_interval(days => $2)",
    )
    .bind(query)
    .bind(max_age_days)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(v,)| v))
}

pub async fn store_search(pool: &PgPool, query: &str, results: &serde_json::Value) -> Result<(), Error> {
    sqlx::query(
        r#"
        INSERT INTO contact_search_cache (query, results) VALUES ($1, $2)
        ON CONFLICT (query) DO UPDATE SET results = EXCLUDED.results, fetched_at = NOW()
        "#,
    )
    .bind(query)
    .bind(results)
    .execute(pool)
    .await?;
    Ok(())
}
