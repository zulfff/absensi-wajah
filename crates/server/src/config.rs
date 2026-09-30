//! Configuration loaded from environment / `.env`.

use clap::Parser;
use std::net::SocketAddr;

#[derive(Clone, Debug, Parser)]
#[command(name = "absensi-server", about = "Face attendance backend")]
pub struct Config {
    /// Postgres connection string.
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,

    /// Address to bind the HTTP server to.
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:8080")]
    pub bind_addr: SocketAddr,

    /// Secret used to sign admin session JWTs. MUST be set in production.
    #[arg(long, env = "JWT_SECRET", default_value = "dev-insecure-change-me")]
    pub jwt_secret: String,

    /// JWT lifetime in seconds (default 8 hours).
    #[arg(long, env = "JWT_TTL_SECONDS", default_value_t = 28800)]
    pub jwt_ttl_seconds: i64,

    /// Connection pool size.
    #[arg(long, env = "DB_POOL_SIZE", default_value_t = 16)]
    pub db_pool_size: u32,

    /// Bootstrap admin username (created on first run if no users exist).
    #[arg(long, env = "BOOTSTRAP_ADMIN_USER", default_value = "admin")]
    pub bootstrap_admin_user: String,

    /// Bootstrap admin password.
    #[arg(long, env = "BOOTSTRAP_ADMIN_PASSWORD", default_value = "admin")]
    pub bootstrap_admin_password: String,

    /// Force the deterministic (model-free) pipeline. Useful for dev/tests.
    #[arg(long, env = "USE_DETERMINISTIC_PIPELINE", default_value_t = false)]
    pub use_deterministic_pipeline: bool,

    /// Decision thresholds (see domain::decision::Thresholds).
    #[arg(long, env = "T_ACCEPT", default_value_t = 0.50)]
    pub t_accept: f32,
    #[arg(long, env = "T_MARGIN", default_value_t = 0.08)]
    pub t_margin: f32,
    #[arg(long, env = "CONSENSUS_REQUIRED", default_value_t = 5)]
    pub consensus_required: usize,
    #[arg(long, env = "CONSENSUS_WINDOW", default_value_t = 7)]
    pub consensus_window: usize,
    #[arg(long, env = "T_LIVENESS", default_value_t = 0.85)]
    pub t_liveness: f32,

    /// Attendance cooldown window in seconds.
    #[arg(long, env = "COOLDOWN_SECONDS", default_value_t = 300)]
    pub cooldown_seconds: i64,

    /// IANA timezone used to bucket attendance into local days (daily reports,
    /// "today" defaults). Attendance is a local-calendar concept: a check-in at
    /// 06:30 WIB belongs to that local day. Defaults to Asia/Jakarta.
    #[arg(long, env = "TZ", default_value = "Asia/Jakarta")]
    pub timezone: String,

    /// Extra CORS origins allowed to call the API (comma-separated), on top of
    /// same-origin. Empty is correct in production: the SPA is served from the
    /// same host as the API, so no cross-origin request is made. Only set this
    /// for the Vite dev server, e.g. `http://localhost:5173`.
    #[arg(long, env = "CORS_ALLOWED_ORIGINS", default_value = "")]
    pub cors_allowed_origins: String,
}

impl Config {
    pub fn load() -> Self {
        let _ = dotenvy::dotenv();
        Self::parse()
    }

    /// Parse `CORS_ALLOWED_ORIGINS` into a list, dropping blanks.
    pub fn cors_origins(&self) -> Vec<String> {
        self.cors_allowed_origins
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    }

    pub fn thresholds(&self) -> domain::decision::Thresholds {
        domain::decision::Thresholds {
            t_accept: self.t_accept,
            t_margin: self.t_margin,
            consensus_required: self.consensus_required,
            consensus_window: self.consensus_window,
            t_liveness: self.t_liveness,
        }
    }

    /// The reporting timezone, parsed. Falls back to Asia/Jakarta (with a
    /// warning) if the configured name is not a known IANA zone, so a typo
    /// cannot take the whole server down.
    pub fn tz(&self) -> chrono_tz::Tz {
        self.timezone.parse().unwrap_or_else(|_| {
            tracing::warn!(
                configured = %self.timezone,
                fallback = db::pool::DEFAULT_TIMEZONE,
                "unknown timezone; using default"
            );
            db::pool::DEFAULT_TIMEZONE
                .parse()
                .expect("valid default tz")
        })
    }

    /// Today's date in the configured reporting timezone.
    pub fn today_local(&self) -> chrono::NaiveDate {
        chrono::Utc::now().with_timezone(&self.tz()).date_naive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    /// Build a Config with only the fields under test set meaningfully.
    fn cfg(timezone: &str) -> Config {
        Config {
            database_url: String::new(),
            bind_addr: "0.0.0.0:0".parse().unwrap(),
            jwt_secret: String::new(),
            jwt_ttl_seconds: 0,
            db_pool_size: 1,
            bootstrap_admin_user: String::new(),
            bootstrap_admin_password: String::new(),
            use_deterministic_pipeline: true,
            t_accept: 0.5,
            t_margin: 0.08,
            consensus_required: 5,
            consensus_window: 7,
            t_liveness: 0.85,
            cooldown_seconds: 300,
            timezone: timezone.to_string(),
            cors_allowed_origins: String::new(),
        }
    }

    #[test]
    fn known_timezone_parses() {
        assert_eq!(cfg("Asia/Jakarta").tz(), chrono_tz::Asia::Jakarta);
    }

    #[test]
    fn unknown_timezone_falls_back_without_panicking() {
        assert_eq!(cfg("Mars/Olympus").tz(), chrono_tz::Asia::Jakarta);
    }

    #[test]
    fn local_day_differs_from_utc_near_midnight() {
        // 2024-06-01 23:30 UTC == 2024-06-02 06:30 WIB. The local day is June 2.
        let instant = Utc.with_ymd_and_hms(2024, 6, 1, 23, 30, 0).unwrap();
        let local = instant.with_timezone(&cfg("Asia/Jakarta").tz());
        assert_eq!(
            local.date_naive(),
            chrono::NaiveDate::from_ymd_opt(2024, 6, 2).unwrap()
        );
        // Whereas the UTC date is still June 1 — this is the bug the config fixes.
        assert_eq!(
            instant.date_naive(),
            chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap()
        );
    }
}
