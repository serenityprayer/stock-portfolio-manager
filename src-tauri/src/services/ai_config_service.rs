use crate::db::Database;
use crate::models::ai_config::AiConfig;
use crate::services::secret_store;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

/// Marker used when the front-end echoes back a masked key. Never stored.
pub const MASK_PREFIX: &str = "****";

pub fn mask_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{MASK_PREFIX}{tail}")
}

fn is_masked(key: &str) -> bool {
    key.starts_with(MASK_PREFIX)
}

/// Read the USABLE (decrypted) config. Takes `&Connection` so it can be called
/// from inside an open transaction without re-locking the Database mutex.
pub fn read_raw_config(conn: &Connection) -> AiConfig {
    let stored: Option<(String, String, String, Option<String>, String)> = conn
        .query_row(
            "SELECT provider, api_key, model, base_url, system_prompt FROM ai_config WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .ok()
        .flatten();
    match stored {
        Some((provider, key, model, base_url, system_prompt)) => AiConfig {
            provider,
            api_key: secret_store::decrypt(&key),
            model,
            base_url,
            system_prompt,
        },
        None => AiConfig::default(),
    }
}

pub fn get_raw_config(db: &Database) -> Result<AiConfig, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    Ok(read_raw_config(&conn))
}

/// What the UI sees: the key is masked, never sent to the renderer in clear.
pub fn get_ai_config(db: &Database) -> Result<AiConfig, String> {
    let mut config = get_raw_config(db)?;
    config.api_key = mask_key(&config.api_key);
    Ok(config)
}

pub fn update_ai_config(db: &Database, config: &AiConfig) -> Result<bool, String> {
    let conn = db.conn.lock().unwrap();
    let previous = read_raw_config(&conn);
    // A masked key echoed back from the settings form must not overwrite the
    // stored secret. An explicitly empty string means "clear the key".
    let plain_key = if is_masked(&config.api_key) {
        previous.api_key
    } else {
        config.api_key.clone()
    };
    let stored_key = if plain_key.is_empty() {
        String::new()
    } else {
        secret_store::encrypt(&plain_key)
    };
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO ai_config (id, provider, api_key, model, base_url, system_prompt, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
           provider = excluded.provider,
           api_key = excluded.api_key,
           model = excluded.model,
           base_url = excluded.base_url,
           system_prompt = excluded.system_prompt,
           updated_at = excluded.updated_at",
        params![
            config.provider,
            stored_key,
            config.model,
            config.base_url,
            config.system_prompt,
            now
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_keeps_only_last_four() {
        assert_eq!(mask_key("sk-1234567890"), "****7890");
        assert_eq!(mask_key(""), "");
    }

    #[test]
    fn masked_key_is_recognised() {
        assert!(is_masked("****7890"));
        assert!(!is_masked("sk-real-key"));
    }
}
