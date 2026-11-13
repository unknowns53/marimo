use serde_json::Value;

use crate::state::{RateLimits, RateWindow};
use crate::time::parse_rfc3339_utc_ms;

// Gemini Models のグループの 5 時間と週のウィンドウ。Gemini 4 Argon の週の枠（gemini-argon-wtu-weekly）や
// Claude と GPT のモデルの枠は、Claude Code のモデル別の枠を出さないのと同じく出さない。
const FIVE_HOUR_BUCKET: &str = "gemini-5h";
const WEEKLY_BUCKET: &str = "gemini-weekly";

/// `agy -p /usage --output-format json` の出力から、Gemini のモデルの利用制限を読む。この出力の形は
/// 文書に無く、agy 1.5.8 が書く形に従う。バケットは残りの割合（remaining_fraction）を持つので、
/// ほかのツールと同じ使った割合に直す。値はその場で問い合わせたものなので、読んだ時刻を更新の時刻にする。
pub fn rate_limits_from_usage(output: &Value, now_ms: u64) -> Option<RateLimits> {
    if output.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        return None;
    }
    let buckets: Vec<&Value> = output
        .pointer("/command/data/groups")?
        .as_array()?
        .iter()
        .filter_map(|g| g.get("buckets").and_then(Value::as_array))
        .flatten()
        .collect();
    let window = |id: &str| {
        let b = buckets
            .iter()
            .find(|b| b.get("id").and_then(Value::as_str) == Some(id))?;
        let remaining = b.get("remaining_fraction")?.as_f64()?;
        Some(RateWindow {
            used_percentage: ((1.0 - remaining) * 100.0).clamp(0.0, 100.0),
            resets_at: b
                .get("reset_time")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_utc_ms)
                .map(|ms| (ms / 1000) as i64),
        })
    };
    let limits = RateLimits {
        five_hour: window(FIVE_HOUR_BUCKET),
        seven_day: window(WEEKLY_BUCKET),
        updated_at: now_ms,
    };
    (limits.five_hour.is_some() || limits.seven_day.is_some()).then_some(limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_only_the_gemini_buckets() {
        let bucket = |id: &str, remaining: f64| {
            json!({"id": id, "window": "weekly", "remaining_fraction": remaining,
                   "reset_time": "2026-11-20T12:55:29Z"})
        };
        let output = |status: &str, groups: Value| {
            json!({"conversation_id": "", "status": status,
                   "command": {"name": "usage", "data": {"groups": groups}}})
        };
        let groups = json!([
            {"name": "Gemini Models", "buckets": [bucket("gemini-weekly", 0.75), bucket("gemini-5h", 0.9)]},
            {"name": "Claude and GPT models", "buckets": [bucket("3p-5h", 0.0)]},
            {"name": "Gemini 4 Argon", "buckets": [bucket("gemini-argon-wtu-weekly", 0.0)]},
        ]);
        let reset = Some(1_795_179_329);
        let cases = [
            (
                output("SUCCESS", groups.clone()),
                Some(RateLimits {
                    five_hour: Some(RateWindow {
                        used_percentage: (1.0 - 0.9) * 100.0,
                        resets_at: reset,
                    }),
                    seven_day: Some(RateWindow {
                        used_percentage: 25.0,
                        resets_at: reset,
                    }),
                    updated_at: 7,
                }),
            ),
            (output("ERROR", groups), None),
            (
                output(
                    "SUCCESS",
                    json!([{"name": "Gemini 4 Argon",
                    "buckets": [bucket("gemini-argon-wtu-weekly", 0.5)]}]),
                ),
                None,
            ),
            (
                json!({"status": "SUCCESS", "response": "no quota summary"}),
                None,
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(rate_limits_from_usage(&input, 7), expected, "{input}");
        }
    }
}
