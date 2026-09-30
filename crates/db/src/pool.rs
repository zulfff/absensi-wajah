//! Connection pool and migration.

use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use std::str::FromStr;
use std::time::Duration;

pub type Db = PgPool;

/// Default reporting timezone when the operator sets nothing.
///
/// Attendance is a *local-calendar* concept: a check-in at 06:30 in Jakarta
/// (UTC+7) belongs to that local day, even though its UTC instant is on the
/// previous day. All date bucketing must therefore use the school's zone, not
/// UTC. This is the one place that default is decided.
pub const DEFAULT_TIMEZONE: &str = "Asia/Jakarta";

/// Connect to PostgreSQL, retrying for a short while (useful in compose where
/// the DB may still be starting).
///
/// Every physical connection is pinned to `timezone` via `SET TIME ZONE`, so
/// `timestamp::date` casts (the daily attendance queries) bucket by the
/// school's local calendar rather than the server's default UTC.
pub async fn connect(
    database_url: &str,
    max_connections: u32,
    timezone: &str,
) -> Result<Db, sqlx::Error> {
    let options = PgConnectOptions::from_str(database_url)?;
    let mut last_err = None;
    for attempt in 0..10 {
        match PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .after_connect({
                let timezone = timezone.to_string();
                move |conn, _meta| {
                    let timezone = timezone.clone();
                    Box::pin(async move {
                        // `SET TIME ZONE` takes no bind parameters; the value is
                        // interpolated. Guard it so an operator cannot inject
                        // SQL through the timezone config.
                        if !is_valid_timezone_name(&timezone) {
                            return Err(sqlx::Error::Configuration(
                                format!("invalid timezone name: {timezone:?}").into(),
                            ));
                        }
                        // The value is interpolated (SET TIME ZONE takes no
                        // bind parameter), which is why `is_valid_timezone_name`
                        // above rejects anything but `[A-Za-z0-9_+/-]`. With that
                        // guard the string is safe, so we assert it explicitly.
                        let stmt = sqlx::AssertSqlSafe(format!("SET TIME ZONE '{timezone}'"));
                        sqlx::query(stmt).execute(&mut *conn).await?;
                        Ok(())
                    })
                }
            })
            .connect_with(options.clone())
            .await
        {
            Ok(pool) => {
                tracing::info!(attempt, %timezone, "connected to database");
                return Ok(pool);
            }
            Err(e) => {
                tracing::warn!(attempt, error = %e, "database not ready, retrying");
                last_err = Some(e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    Err(last_err.expect("at least one attempt"))
}

/// A conservative guard for the timezone value: IANA names, abbreviations and
/// UTC offsets are `[A-Za-z0-9_+\-/]`. Anything else (quotes, semicolons,
/// spaces) is rejected before it reaches the `SET TIME ZONE` statement.
fn is_valid_timezone_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'))
}

/// Run embedded migrations from `./migrations`.
pub async fn migrate(pool: &Db) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await?;
    tracing::info!("migrations applied");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_valid_timezone_name;

    #[test]
    fn accepts_iana_and_offset_names() {
        assert!(is_valid_timezone_name("Asia/Jakarta"));
        assert!(is_valid_timezone_name("UTC"));
        assert!(is_valid_timezone_name("Etc/GMT-7"));
        assert!(is_valid_timezone_name("UTC+7"));
    }

    #[test]
    fn rejects_unsafe_names() {
        assert!(!is_valid_timezone_name(""));
        assert!(!is_valid_timezone_name("UTC'; DROP TABLE attendance; --"));
        assert!(!is_valid_timezone_name("Asia/Jakarta'"));
        assert!(!is_valid_timezone_name("Asia Jakarta"));
    }
}
