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
    Ok(sqlx::query_as("SELECT user_id, display_name, channel_id, keywords, seniority, locations FROM profiles ORDER BY user_id")
        .fetch_all(pool)
        .await?)
}

pub async fn profile(pool: &PgPool, user_id: i64) -> Result<Option<Profile>, Error> {
    Ok(sqlx::query_as("SELECT user_id, display_name, channel_id, keywords, seniority, locations FROM profiles WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn upsert_profile(pool: &PgPool, user_id: i64, display_name: &str, channel_id: i64) -> Result<Profile, Error> {
    Ok(sqlx::query_as(
        r#"
        INSERT INTO profiles (user_id, display_name, channel_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (user_id) DO UPDATE
            SET display_name = EXCLUDED.display_name,
                channel_id = EXCLUDED.channel_id,
                update_time = NOW()
        RETURNING user_id, display_name, channel_id, keywords, seniority, locations
        "#,
    )
    .bind(user_id)
    .bind(display_name)
    .bind(channel_id)
    .fetch_one(pool)
    .await?)
}

/// Replaces whichever filter lists are `Some`; `None` leaves the current value alone.
pub async fn set_profile_filters(
    pool: &PgPool,
    user_id: i64,
    keywords: Option<&[String]>,
    seniority: Option<&[String]>,
    locations: Option<&[String]>,
) -> Result<(), Error> {
    sqlx::query(
        r#"
        UPDATE profiles
        SET keywords = COALESCE($2, keywords),
            seniority = COALESCE($3, seniority),
            locations = COALESCE($4, locations),
            update_time = NOW()
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .bind(keywords)
    .bind(seniority)
    .bind(locations)
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

/// Returns true when the company was newly added to the profile.
pub async fn track_company(pool: &PgPool, user_id: i64, company_id: i64) -> Result<bool, Error> {
    let result = sqlx::query("INSERT INTO profile_companies (user_id, company_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
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

/// Jobs from the given companies that this user has never been notified about.
pub async fn candidates_for_profile(pool: &PgPool, user_id: i64, company_ids: &[i64]) -> Result<Vec<Candidate>, Error> {
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
          AND NOT EXISTS (
            SELECT 1 FROM notifications AS n
            WHERE n.application_id = a.id AND n.user_id = $1
          )
        ORDER BY a.id DESC
        "#,
    )
    .bind(user_id)
    .bind(company_ids)
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
