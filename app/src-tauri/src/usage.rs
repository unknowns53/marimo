// usage-replay の試験用のビルドは資格情報も API も使わないので、使われない部分の警告を出さない。
#![cfg_attr(
    feature = "usage-replay",
    allow(dead_code, unused_variables, unused_assignments)
)]

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use marimo_core::time::now_ms;
use marimo_core::{MarimoHome, RateLimits, RateWindow, store};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::credentials::{self, Credentials, ReadError, Unusable};

// Claude の OAuth のトークンで利用制限を返すエンドポイント。公開された API ではなく文書もないので、
// 応答の形が変わったら書き込まずに次の周期を待つ。
const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
const TIMEOUT: Duration = Duration::from_secs(10);
const BODY_LIMIT: u64 = 1024 * 1024;

// Windows だけ rustls にする理由は app/src-tauri/Cargo.toml に書いてある。
#[cfg(windows)]
const TLS_PROVIDER: TlsProvider = TlsProvider::Rustls;
#[cfg(not(windows))]
const TLS_PROVIDER: TlsProvider = TlsProvider::NativeTls;

pub const STATUS_EVENT: &str = "usage-status";

pub const INTERVAL: Duration = Duration::from_secs(5 * 60);
pub const MAX_BACKOFF: Duration = Duration::from_secs(30 * 60);
// Retry-After が桁違いに大きくても、設定を入れ直さずに再開できるよう上限を設ける。
const RETRY_AFTER_CAP: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Updated,
    RateLimited { retry_after: Option<Duration> },
    Unauthorized(u16),
    Unusable(Unusable),
    NotFound,
    Failed(String),
}

/// 再試行の時刻は、フロントエンドが Date にそのまま渡せるよう Unix ミリ秒で持つ。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageStatus {
    Pending,
    Disabled,
    Ok,
    TokenExpired,
    MissingScope,
    NotFound,
    KeychainDenied,
    RateLimited { retry_at: u64 },
    Rejected { code: u16 },
    Failed { detail: String, retry_at: u64 },
}

impl UsageStatus {
    pub fn from_outcome(outcome: &Outcome, retry_at: u64) -> Self {
        match outcome {
            Outcome::Updated => Self::Ok,
            Outcome::RateLimited { .. } => Self::RateLimited { retry_at },
            Outcome::Unauthorized(code) => Self::Rejected { code: *code },
            Outcome::Unusable(Unusable::Expired) => Self::TokenExpired,
            Outcome::Unusable(Unusable::MissingScope) => Self::MissingScope,
            Outcome::NotFound => Self::NotFound,
            Outcome::Failed(detail) => Self::Failed {
                detail: detail.clone(),
                retry_at,
            },
        }
    }

    // 失敗が続くと再試行の時刻は周期ごとに変わるので、ログはそれ以外が変わったときだけ書く。
    fn same_state(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::RateLimited { .. }, Self::RateLimited { .. }) => true,
            (Self::Failed { detail: a, .. }, Self::Failed { detail: b, .. }) => a == b,
            _ => self == other,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Pending => "pending: waiting for the first result".to_owned(),
            Self::Disabled => "disabled: usage polling is turned off".to_owned(),
            Self::Ok => "ok: rate limits were updated".to_owned(),
            Self::TokenExpired => format!("token_expired: {}", Unusable::Expired.reason()),
            Self::MissingScope => format!("missing_scope: {}", Unusable::MissingScope.reason()),
            Self::NotFound => "not_found: no Claude Code credentials were found".to_owned(),
            Self::KeychainDenied => "keychain_denied: keychain access was denied; usage polling stops until the setting is turned on again".to_owned(),
            Self::RateLimited { retry_at } => format!(
                "rate_limited: usage API is rate limited (HTTP 429); next try at {}",
                local_time(*retry_at, "%H:%M")
            ),
            Self::Rejected { code } => format!("rejected: usage API rejected the token (HTTP {code})"),
            Self::Failed { detail, retry_at } => format!(
                "failed: {detail}; next try at {}",
                local_time(*retry_at, "%H:%M")
            ),
        }
    }
}

fn local_time(ms: u64, format: &str) -> String {
    Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map_or_else(|| ms.to_string(), |t| t.format(format).to_string())
}

/// 利用者がキーチェーンの確認を拒んだので、設定を入れ直すまで止める。
struct Halt;

/// 今の間隔と直前の結果から、次の間隔と次に呼ぶまでの待ち時間を決める。429 のときだけ間隔を倍に
/// 広げ、成功したら戻す。それ以外の失敗は今の間隔のまま次の周期を待つ。
pub fn next_wait(interval: Duration, outcome: &Outcome) -> (Duration, Duration) {
    match outcome {
        Outcome::Updated => (INTERVAL, INTERVAL),
        Outcome::RateLimited { retry_after } => {
            let next = (interval * 2).min(MAX_BACKOFF);
            let wait = retry_after.map_or(next, |r| next.max(r.min(RETRY_AFTER_CAP)));
            (next, wait)
        }
        _ => (interval, interval),
    }
}

/// HTTP の日付の形の Retry-After は扱わず、秒数のときだけ使う。
pub fn retry_after_seconds(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

pub fn unix_seconds(rfc3339: &str) -> Option<i64> {
    OffsetDateTime::parse(rfc3339, &Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

/// `utilization` は 0 から 100 の百分率で、statusLine の `used_percentage` と同じ尺度である。
pub fn rate_limits_from_usage(body: &str, now: u64) -> Option<RateLimits> {
    let v: Value = serde_json::from_str(body).ok()?;
    let window = |key: &str| {
        let w = v.get(key)?;
        Some(RateWindow {
            used_percentage: w.get("utilization")?.as_f64()?,
            resets_at: w
                .get("resets_at")
                .and_then(Value::as_str)
                .and_then(unix_seconds),
        })
    };
    let limits = RateLimits {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
        updated_at: now,
    };
    (limits.five_hour.is_some() || limits.seven_day.is_some()).then_some(limits)
}

struct Control {
    enabled: bool,
    halted: bool,
    generation: u64,
    status: UsageStatus,
}

struct Shared {
    control: Mutex<Control>,
    wake: Condvar,
}

#[derive(Clone)]
pub struct Poller {
    shared: Arc<Shared>,
}

impl Poller {
    pub fn new(enabled: bool) -> Self {
        let status = if enabled {
            UsageStatus::Pending
        } else {
            UsageStatus::Disabled
        };
        let shared = Arc::new(Shared {
            control: Mutex::new(Control {
                enabled,
                halted: false,
                generation: 0,
                status,
            }),
            wake: Condvar::new(),
        });
        Self { shared }
    }

    pub fn status(&self) -> UsageStatus {
        lock(&self.shared).status.clone()
    }

    /// 有効にすると、キーチェーンを拒まれて止めていた状態も解いて、すぐに一度取りに行く。
    pub fn set_enabled(&self, enabled: bool) {
        let mut c = lock(&self.shared);
        c.enabled = enabled;
        c.halted = false;
        c.generation += 1;
        self.shared.wake.notify_all();
    }

    pub fn spawn(&self, home: MarimoHome, app: AppHandle) {
        let shared = self.shared.clone();
        thread::spawn(move || {
            let reporter = Reporter::new(home.clone(), move |status: &UsageStatus| {
                let _ = app.emit(STATUS_EVENT, status);
            });
            run(&shared, &home, reporter);
        });
    }
}

fn lock(shared: &Shared) -> MutexGuard<'_, Control> {
    shared
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

struct Reporter<E> {
    home: MarimoHome,
    emit: E,
    logged: Option<UsageStatus>,
}

impl<E: Fn(&UsageStatus)> Reporter<E> {
    fn new(home: MarimoHome, emit: E) -> Self {
        Self {
            home,
            emit,
            logged: None,
        }
    }

    fn report(&mut self, shared: &Shared, status: UsageStatus) {
        {
            let mut c = lock(shared);
            if c.status == status {
                return;
            }
            c.status = status.clone();
        }
        (self.emit)(&status);
        // 一度も有効にしていない利用者のためにログのファイルを作らないよう、最初の無効は書かない。
        let quiet = match status {
            UsageStatus::Pending => true,
            UsageStatus::Disabled => self.logged.is_none(),
            _ => false,
        };
        if quiet || self.logged.as_ref().is_some_and(|l| l.same_state(&status)) {
            return;
        }
        let text = status.describe();
        if !matches!(status, UsageStatus::Ok | UsageStatus::Disabled) {
            eprintln!("marimo: {text}");
        }
        let line = format!("{} {text}", local_time(now_ms(), "%Y-%m-%d %H:%M:%S %:z"));
        if let Err(e) = store::append_usage_log(&self.home, &line) {
            eprintln!("marimo: cannot write the usage log: {e}");
        }
        self.logged = Some(status);
    }
}

fn run<E: Fn(&UsageStatus)>(shared: &Shared, home: &MarimoHome, mut reporter: Reporter<E>) {
    #[cfg(not(feature = "usage-replay"))]
    let agent = agent();
    // キーチェーンを読むたびに確認が出うるので、トークンは期限が切れるか拒まれるまで手元に持つ。
    // ファイルにもログにも出さない。
    let mut cached: Option<Credentials> = None;
    let mut interval = INTERVAL;
    loop {
        let mut resumed = false;
        let generation = loop {
            let idle = {
                let c = lock(shared);
                if c.enabled && !c.halted {
                    break c.generation;
                }
                !c.enabled
            };
            cached = None;
            if idle {
                reporter.report(shared, UsageStatus::Disabled);
            }
            let c = lock(shared);
            if !c.enabled || c.halted {
                drop(shared.wake.wait(c).unwrap_or_else(PoisonError::into_inner));
            }
            resumed = true;
        };
        // 止めていた間の理由を、次の結果が出るまで残さない。
        if resumed {
            reporter.report(shared, UsageStatus::Pending);
        }

        #[cfg(not(feature = "usage-replay"))]
        let polled = poll(&agent, home, &mut cached);
        #[cfg(feature = "usage-replay")]
        let polled = replay::poll();
        let outcome = match polled {
            Ok(outcome) => outcome,
            Err(Halt) => {
                let halted = {
                    let mut c = lock(shared);
                    if c.generation == generation {
                        c.halted = true;
                    }
                    c.halted
                };
                if halted {
                    reporter.report(shared, UsageStatus::KeychainDenied);
                }
                continue;
            }
        };
        let (next, wait) = next_wait(interval, &outcome);
        interval = next;
        reporter.report(
            shared,
            UsageStatus::from_outcome(&outcome, now_ms() + wait.as_millis() as u64),
        );
        #[cfg(feature = "usage-replay")]
        let wait = wait.min(replay::TICK);

        let deadline = Instant::now() + wait;
        let mut c = lock(shared);
        while c.generation == generation {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            c = shared
                .wake
                .wait_timeout(c, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

fn poll(
    agent: &ureq::Agent,
    home: &MarimoHome,
    cached: &mut Option<Credentials>,
) -> Result<Outcome, Halt> {
    let now = now_ms();
    let creds = match cached.take().filter(|c| c.check(now).is_ok()) {
        Some(c) => c,
        None => match credentials::read() {
            Ok(bytes) => match credentials::parse(&bytes) {
                Ok(c) => c,
                Err(reason) => return Ok(Outcome::Failed(reason.to_owned())),
            },
            Err(ReadError::Denied) => return Err(Halt),
            Err(ReadError::NotFound) => return Ok(Outcome::NotFound),
            Err(ReadError::Other(message)) => return Ok(Outcome::Failed(message)),
        },
    };
    if let Err(unusable) = creds.check(now) {
        return Ok(Outcome::Unusable(unusable));
    }

    let outcome = match fetch(agent, &creds) {
        Err(message) => Outcome::Failed(message),
        Ok(Response::Body(body)) => match rate_limits_from_usage(&body, now_ms()) {
            Some(limits) => match store::write_rate_limits(home, &limits) {
                Ok(()) => Outcome::Updated,
                Err(e) => Outcome::Failed(format!("cannot write rate limits: {e}")),
            },
            None => Outcome::Failed(
                "the usage API response had no usable five_hour or seven_day".to_owned(),
            ),
        },
        Ok(Response::Status {
            code: 429,
            retry_after,
        }) => Outcome::RateLimited { retry_after },
        // 拒まれたトークンは捨て、次の周期で読み直す。Claude Code が更新していれば新しい方が取れる。
        Ok(Response::Status {
            code: code @ (401 | 403),
            ..
        }) => return Ok(Outcome::Unauthorized(code)),
        Ok(Response::Status { code, .. }) => {
            Outcome::Failed(format!("usage API returned HTTP {code}"))
        }
    };
    *cached = Some(creds);
    Ok(outcome)
}

enum Response {
    Body(String),
    Status {
        code: u16,
        retry_after: Option<Duration>,
    },
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .https_only(true)
        // リダイレクト先へトークンを送らないよう、転送は追わない。
        .max_redirects(0)
        .user_agent(concat!("marimo/", env!("CARGO_PKG_VERSION")))
        // 信頼するルート証明書は OS のものでなく同梱の Mozilla のものにする。通信を解読する中間者型の
        // プロキシが OS に証明書を入れている環境で、トークンがそのプロキシに見えないようにするためである。
        .tls_config(
            TlsConfig::builder()
                .provider(TLS_PROVIDER)
                .root_certs(RootCerts::WebPki)
                .build(),
        )
        .build()
        .into()
}

// 誤りの文にはヘッダの値を入れない。ureq の誤りの文も要求のヘッダを含まない。
fn fetch(agent: &ureq::Agent, creds: &Credentials) -> Result<Response, String> {
    let mut auth =
        ureq::http::HeaderValue::from_str(&format!("Bearer {}", creds.access_token.expose()))
            .map_err(|_| "the Claude Code token cannot be sent as a header".to_owned())?;
    auth.set_sensitive(true);
    let mut response = agent
        .get(ENDPOINT)
        .header("authorization", auth)
        .header("anthropic-beta", BETA)
        .call()
        .map_err(|e| format!("usage API request failed: {e}"))?;
    let code = response.status().as_u16();
    if code != 200 {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(retry_after_seconds);
        return Ok(Response::Status { code, retry_after });
    }
    response
        .body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .read_to_string()
        .map(Response::Body)
        .map_err(|e| format!("cannot read the usage API response: {e}"))
}

#[cfg(feature = "usage-replay")]
mod replay {
    //! 試験用のビルドだけで使う。キーチェーンにも API にも触れずに利用制限の行に出す理由を確かめるため、
    //! MARIMO_USAGE_REPLAY が指すファイルの中身（ok、expired、scope、notfound、denied、`429 秒数`、
    //! 401、403、それ以外は失敗の文）を、周期ごとに問い合わせの結果の代わりに使う。ファイルを書き換えれば
    //! すぐに表示が変わるよう、周期も短くする。
    use std::time::Duration;

    use super::{Halt, Outcome, Unusable, retry_after_seconds};

    pub const TICK: Duration = Duration::from_secs(2);

    pub fn poll() -> Result<Outcome, Halt> {
        let text = std::env::var_os("MARIMO_USAGE_REPLAY")
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        let mut words = text.split_whitespace();
        Ok(match (words.next(), words.next()) {
            (Some("ok"), _) => Outcome::Updated,
            (Some("expired"), _) => Outcome::Unusable(Unusable::Expired),
            (Some("scope"), _) => Outcome::Unusable(Unusable::MissingScope),
            (Some("notfound"), _) => Outcome::NotFound,
            (Some("denied"), _) => return Err(Halt),
            (Some("429"), secs) => Outcome::RateLimited {
                retry_after: secs.and_then(retry_after_seconds),
            },
            (Some("401"), _) => Outcome::Unauthorized(401),
            (Some("403"), _) => Outcome::Unauthorized(403),
            _ => Outcome::Failed(format!("usage replay: {}", text.trim())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: &str = r#"{
        "five_hour": {"utilization": 23.0, "resets_at": "2026-09-27T19:00:00.279959+00:00", "limit_dollars": null, "used_dollars": null, "remaining_dollars": null, "locked_reason": null},
        "seven_day": {"utilization": 84.0, "resets_at": "2026-09-27T21:00:00.279991+00:00", "limit_dollars": null, "used_dollars": null, "remaining_dollars": null, "locked_reason": null},
        "seven_day_opus": null,
        "seven_day_sonnet": null,
        "iguana_necktie": {"utilization": 0.0, "resets_at": "2026-11-05T07:59:00+00:00", "limit_dollars": 250, "used_dollars": 0.0, "remaining_dollars": 250.0, "locked_reason": null},
        "extra_usage": {"is_enabled": false, "utilization": null},
        "limits": [{"kind": "session", "group": "session", "percent": 23, "resets_at": "2026-09-27T19:00:00.279959+00:00"}]
    }"#;

    // 2026-09-27T19:00:00Z
    const NINETEEN: i64 = 1_790_535_600;

    #[test]
    fn maps_the_real_response_shape() {
        let rl = rate_limits_from_usage(FIXTURE, 42).unwrap();
        assert_eq!(
            rl,
            RateLimits {
                five_hour: Some(RateWindow {
                    used_percentage: 23.0,
                    resets_at: Some(NINETEEN)
                }),
                seven_day: Some(RateWindow {
                    used_percentage: 84.0,
                    resets_at: Some(NINETEEN + 2 * 3600)
                }),
                updated_at: 42,
            }
        );
    }

    #[test]
    fn usage_response_tolerates_missing_parts() {
        let rl = rate_limits_from_usage(
            r#"{"five_hour": null, "seven_day": {"utilization": 5, "resets_at": null}}"#,
            1,
        )
        .unwrap();
        assert_eq!(rl.five_hour, None);
        assert_eq!(
            rl.seven_day,
            Some(RateWindow {
                used_percentage: 5.0,
                resets_at: None
            })
        );
        let rl = rate_limits_from_usage(
            r#"{"five_hour": {"utilization": 1.5, "resets_at": "soon"}}"#,
            1,
        )
        .unwrap();
        assert_eq!(rl.five_hour.unwrap().resets_at, None);

        for body in [
            r#"{"five_hour": null, "seven_day": null}"#,
            r#"{"five_hour": {"utilization": null}, "seven_day": {}}"#,
            r#"{"extra_usage": {"utilization": 3}}"#,
            "[]",
            "not json",
            "",
        ] {
            assert_eq!(rate_limits_from_usage(body, 1), None, "body {body:?}");
        }
    }

    #[test]
    fn next_wait_backoff() {
        let limited = Outcome::RateLimited { retry_after: None };
        let mut interval = INTERVAL;
        let mut waits = Vec::new();
        for _ in 0..4 {
            let (next, wait) = next_wait(interval, &limited);
            interval = next;
            waits.push(wait.as_secs() / 60);
        }
        assert_eq!(waits, [10, 20, 30, 30]);
        assert_eq!(next_wait(interval, &Outcome::Updated), (INTERVAL, INTERVAL));

        // 429 以外の失敗は、広げた間隔を戻さずに次の周期を待つ。
        for outcome in [
            Outcome::Unauthorized(401),
            Outcome::Unusable(Unusable::Expired),
            Outcome::NotFound,
            Outcome::Failed("io".to_owned()),
        ] {
            assert_eq!(next_wait(INTERVAL, &outcome), (INTERVAL, INTERVAL));
            assert_eq!(next_wait(MAX_BACKOFF, &outcome), (MAX_BACKOFF, MAX_BACKOFF));
        }
    }

    #[test]
    fn retry_after_is_honored_but_bounded() {
        assert_eq!(retry_after_seconds("120"), Some(Duration::from_secs(120)));
        assert_eq!(retry_after_seconds(" 7 "), Some(Duration::from_secs(7)));
        assert_eq!(retry_after_seconds("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(retry_after_seconds("-1"), None);

        let min = |m: u64| Duration::from_secs(m * 60);
        let with = |secs| Outcome::RateLimited {
            retry_after: Some(Duration::from_secs(secs)),
        };
        assert_eq!(next_wait(INTERVAL, &with(60 * 60)), (min(10), min(60)));
        assert_eq!(next_wait(INTERVAL, &with(30)), (min(10), min(10)));
        assert_eq!(
            next_wait(INTERVAL, &with(10 * 24 * 60 * 60)),
            (min(10), RETRY_AFTER_CAP)
        );
    }

    #[test]
    fn outcomes_map_to_the_status_the_panel_reads() {
        let cases = [
            (Outcome::Updated, json!({"kind": "ok"})),
            (
                Outcome::RateLimited { retry_after: None },
                json!({"kind": "rate_limited", "retry_at": 42}),
            ),
            (
                Outcome::Unauthorized(403),
                json!({"kind": "rejected", "code": 403}),
            ),
            (
                Outcome::Unusable(Unusable::Expired),
                json!({"kind": "token_expired"}),
            ),
            (
                Outcome::Unusable(Unusable::MissingScope),
                json!({"kind": "missing_scope"}),
            ),
            (Outcome::NotFound, json!({"kind": "not_found"})),
            (
                Outcome::Failed("io".to_owned()),
                json!({"kind": "failed", "detail": "io", "retry_at": 42}),
            ),
        ];
        for (outcome, expected) in cases {
            let status = UsageStatus::from_outcome(&outcome, 42);
            assert_eq!(
                serde_json::to_value(&status).unwrap(),
                expected,
                "{outcome:?}"
            );
        }
    }

    #[test]
    fn reports_changes_and_logs_only_when_more_than_the_retry_time_changes() {
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path());
        let shared = Poller::new(true).shared;
        let emitted = std::cell::RefCell::new(Vec::new());
        let mut reporter = Reporter::new(home.clone(), |s: &UsageStatus| {
            emitted.borrow_mut().push(s.clone())
        });
        let failed = |detail: &str, retry_at| UsageStatus::Failed {
            detail: detail.to_owned(),
            retry_at,
        };
        for status in [
            UsageStatus::Pending,
            UsageStatus::Ok,
            UsageStatus::Ok,
            failed("io", 1),
            failed("io", 2),
            failed("dns", 3),
            UsageStatus::RateLimited { retry_at: 4 },
            UsageStatus::RateLimited { retry_at: 5 },
            UsageStatus::TokenExpired,
        ] {
            reporter.report(&shared, status);
        }
        drop(reporter);
        assert_eq!(
            emitted.into_inner(),
            [
                UsageStatus::Ok,
                failed("io", 1),
                failed("io", 2),
                failed("dns", 3),
                UsageStatus::RateLimited { retry_at: 4 },
                UsageStatus::RateLimited { retry_at: 5 },
                UsageStatus::TokenExpired,
            ]
        );
        let log = std::fs::read_to_string(home.usage_log_file()).unwrap();
        let kinds: Vec<&str> = log
            .lines()
            .map(|l| l.split_whitespace().nth(3).unwrap())
            .collect();
        assert_eq!(
            kinds,
            [
                "ok:",
                "failed:",
                "failed:",
                "rate_limited:",
                "token_expired:"
            ]
        );
    }
}
