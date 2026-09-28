use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn rfc3339_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let millis = ms % 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// `YYYY-MM-DD` の形の UTC の日付。
pub fn utc_date(ms: u64) -> String {
    let (y, m, d) = civil_from_days((ms / 86_400_000) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DDTHH:MM:SS` に小数秒と `Z` が続く UTC の時刻を、Unix エポックからのミリ秒にする。
/// Codex の rollout の各行の timestamp がこの形で書かれる。時差の付いた形は受け付けない。
pub fn parse_rfc3339_utc_ms(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let mut t = hms.splitn(3, ':').map(|p| p.parse::<u64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let millis = format!("{frac:0<3}").get(..3)?.parse::<u64>().ok()?;
    let days = u64::try_from(days_from_civil(y, m as u32, day as u32)).ok()?;
    Some(((days * 86_400 + hh * 3600 + mm * 60 + ss) * 1000) + millis)
}

// Howard Hinnant の civil_from_days アルゴリズム。日付だけのために chrono を
// 依存に加えると、フックの起動時間とバイナリサイズが増えるので自前で持つ。
// 出典の https://howardhinnant.github.io/date_algorithms.html で、著者はこれらの
// アルゴリズムをパブリックドメインに供すると明記している。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// 同じ出典の days_from_civil アルゴリズム。
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting_of_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(rfc3339_utc(1_790_509_325_123), "2026-09-27T11:42:05.123Z");
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_399_999), "2000-02-28");
        assert_eq!(utc_date(951_782_400_000), "2000-02-29");
        assert_eq!(utc_date(1_790_509_325_123), "2026-09-27");
    }

    #[test]
    fn utc_parsing_inverts_formatting() {
        for ms in [0, 951_782_400_000, 1_790_509_325_123] {
            assert_eq!(parse_rfc3339_utc_ms(&rfc3339_utc(ms)), Some(ms));
        }
        assert_eq!(
            parse_rfc3339_utc_ms("2026-09-27T11:42:05Z"),
            Some(1_790_509_325_000)
        );
        assert_eq!(
            parse_rfc3339_utc_ms("2026-09-27T11:42:05.1234Z"),
            Some(1_790_509_325_123)
        );
        for bad in [
            "",
            "2026-09-27",
            "2026-09-27T11:42:05+09:00",
            "2026-13-01T00:00:00Z",
            "x-y-zT0:0:0Z",
        ] {
            assert_eq!(parse_rfc3339_utc_ms(bad), None, "{bad}");
        }
    }
}
