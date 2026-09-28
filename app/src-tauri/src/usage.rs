use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use marimo_core::time::now_ms;
use marimo_core::{MarimoHome, RateLimits, RateWindow, store};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::credentials::{self, Credentials, ReadError};

// Claude の OAuth のトークンで利用制限を返すエンドポイント。公開された API ではなく文書もないので、
// 応答の形が変わったら書き込まずに次の周期を待つ。
const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
const TIMEOUT: Duration = Duration::from_secs(10);
const BODY_LIMIT: u64 = 1024 * 1024;

pub const INTERVAL: Duration = Duration::from_secs(5 * 60);
pub const MAX_BACKOFF: Duration = Duration::from_secs(30 * 60);
// Retry-After が桁違いに大きくても、設定を入れ直さずに再開できるよう上限を設ける。
const RETRY_AFTER_CAP: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Updated,
    RateLimited { retry_after: Option<Duration> },
    Unauthorized(u16),
    Skipped(&'static str),
    Failed(String),
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

#[derive(Default)]
struct Control {
    enabled: bool,
    halted: bool,
    generation: u64,
}

#[derive(Default)]
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
        let shared = Arc::new(Shared::default());
        lock(&shared).enabled = enabled;
        Self { shared }
    }

    /// 有効にすると、キーチェーンを拒まれて止めていた状態も解いて、すぐに一度取りに行く。
    pub fn set_enabled(&self, enabled: bool) {
        let mut c = lock(&self.shared);
        c.enabled = enabled;
        c.halted = false;
        c.generation += 1;
        self.shared.wake.notify_all();
    }

    pub fn spawn(&self, home: MarimoHome) {
        let shared = self.shared.clone();
        thread::spawn(move || run(&shared, &home));
    }
}

fn lock(shared: &Shared) -> MutexGuard<'_, Control> {
    shared
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn run(shared: &Shared, home: &MarimoHome) {
    let agent = agent();
    // キーチェーンを読むたびに確認が出うるので、トークンは期限が切れるか拒まれるまで手元に持つ。
    // ファイルにもログにも出さない。
    let mut cached: Option<Credentials> = None;
    let mut interval = INTERVAL;
    let mut last_note: Option<String> = None;
    loop {
        let generation = {
            let mut c = lock(shared);
            while !c.enabled || c.halted {
                cached = None;
                c = shared.wake.wait(c).unwrap_or_else(PoisonError::into_inner);
            }
            c.generation
        };

        let outcome = match poll(&agent, home, &mut cached) {
            Ok(outcome) => outcome,
            Err(Halt) => {
                let mut c = lock(shared);
                if c.generation == generation {
                    c.halted = true;
                }
                eprintln!(
                    "marimo: keychain access was denied; usage polling stops until the setting is turned on again"
                );
                last_note = None;
                continue;
            }
        };
        let (next, wait) = next_wait(interval, &outcome);
        interval = next;
        let note = match &outcome {
            Outcome::Updated => None,
            Outcome::RateLimited { .. } => Some(format!(
                "usage API is rate limited (HTTP 429); next try in {} min",
                wait.as_secs().div_ceil(60)
            )),
            Outcome::Unauthorized(code) => {
                Some(format!("usage API rejected the token (HTTP {code})"))
            }
            Outcome::Skipped(reason) => Some((*reason).to_owned()),
            Outcome::Failed(message) => Some(message.clone()),
        };
        if note.is_some() && note != last_note {
            eprintln!("marimo: {}", note.as_deref().unwrap_or_default());
        }
        last_note = note;

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
                Err(reason) => return Ok(Outcome::Skipped(reason)),
            },
            Err(ReadError::Denied) => return Err(Halt),
            Err(ReadError::NotFound) => {
                return Ok(Outcome::Skipped("no Claude Code credentials were found"));
            }
            Err(ReadError::Other(message)) => return Ok(Outcome::Failed(message)),
        },
    };
    if let Err(unusable) = creds.check(now) {
        return Ok(Outcome::Skipped(unusable.reason()));
    }

    let outcome = match fetch(agent, &creds) {
        Err(message) => Outcome::Failed(message),
        Ok(Response::Body(body)) => match rate_limits_from_usage(&body, now_ms()) {
            Some(limits) => match store::write_rate_limits(home, &limits) {
                Ok(()) => Outcome::Updated,
                Err(e) => Outcome::Failed(format!("cannot write rate limits: {e}")),
            },
            None => Outcome::Skipped("the usage API response had no usable five_hour or seven_day"),
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
                .provider(TlsProvider::NativeTls)
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

#[cfg(test)]
mod tests {
    use super::*;

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
            Outcome::Skipped("expired"),
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
}
