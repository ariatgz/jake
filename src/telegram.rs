//! Optional copies of job alerts on Telegram, through a bot the host creates
//! with @BotFather and configures with `TELEGRAM_BOT_TOKEN`.

use crate::Error;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

pub fn token() -> Option<String> {
    std::env::var("TELEGRAM_BOT_TOKEN").ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

#[derive(Deserialize)]
struct Reply<T> {
    ok: bool,
    description: Option<String>,
    result: Option<T>,
}

#[derive(Deserialize)]
struct Me {
    username: Option<String>,
}

async fn call<T: for<'de> Deserialize<'de>>(client: &Client, token: &str, method: &str, body: serde_json::Value) -> Result<Option<T>, Error> {
    // The bot token is part of the URL, so errors are stripped of it before they can be shown or logged.
    let response = client
        .post(format!("https://api.telegram.org/bot{token}/{method}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.without_url())?;
    let reply: Reply<T> = response.json().await.map_err(|e| e.without_url())?;
    if !reply.ok {
        return Err(format!("Telegram said: {}", reply.description.unwrap_or_else(|| "request failed".into())).into());
    }
    Ok(reply.result)
}

/// Sends an HTML-formatted message (only `<b>`, `<i>`, `<a href>` and friends are allowed).
pub async fn send(client: &Client, token: &str, chat_id: i64, html: &str) -> Result<(), Error> {
    let body = json!({
        "chat_id": chat_id,
        "text": html,
        "parse_mode": "HTML",
        "link_preview_options": { "is_disabled": true },
    });
    call::<serde_json::Value>(client, token, "sendMessage", body).await?;
    Ok(())
}

/// The bot's @username, for pointing users at it.
pub async fn bot_username(client: &Client, token: &str) -> Option<String> {
    call::<Me>(client, token, "getMe", json!({})).await.ok().flatten().and_then(|me| me.username)
}

/// Escapes text for Telegram's HTML parse mode.
pub fn escape(text: &str) -> String {
    html_escape::encode_text(text).into_owned()
}

/// Escapes a value for a double-quoted HTML attribute such as `href`.
pub fn escape_attr(text: &str) -> String {
    html_escape::encode_double_quoted_attribute(text).into_owned()
}
