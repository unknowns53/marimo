use std::fmt;

use serde::Deserialize;

// 利用量の API はこのスコープがないと応じない。
const REQUIRED_SCOPE: &str = "user:profile";

/// 表示やログに値が出ないよう、Debug では伏せる。
pub struct Token(String);

impl Token {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[derive(Debug)]
pub struct Credentials {
    pub access_token: Token,
    /// Unix ミリ秒。
    pub expires_at: i64,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unusable {
    Expired,
    MissingScope,
}

impl Unusable {
    pub fn reason(self) -> &'static str {
        match self {
            Unusable::Expired => {
                "the Claude Code token has expired; waiting for Claude Code to refresh it"
            }
            Unusable::MissingScope => "the Claude Code token lacks the user:profile scope",
        }
    }
}

// marimo はトークンを読むだけで、更新も保存もしない。更新するとリフレッシュトークンが入れ替わり、
// Claude Code が持っている方が使えなくなってログインが切れるためである。期限が切れていたら呼ばずに、
// Claude Code が更新するのを待つ。
impl Credentials {
    pub fn check(&self, now_ms: u64) -> Result<(), Unusable> {
        if self.expires_at <= now_ms as i64 {
            return Err(Unusable::Expired);
        }
        if !self.scopes.iter().any(|s| s == REQUIRED_SCOPE) {
            return Err(Unusable::MissingScope);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<OauthEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OauthEntry {
    access_token: String,
    expires_at: i64,
    #[serde(default)]
    scopes: Vec<String>,
}

// serde_json の誤りの文には入力の値がそのまま入ることがあるので、決まった文だけを返す。
const SHAPE_ERROR: &str = "the Claude Code credentials are not in the expected shape";

pub fn parse(bytes: &[u8]) -> Result<Credentials, &'static str> {
    let file: CredentialsFile = serde_json::from_slice(bytes).map_err(|_| SHAPE_ERROR)?;
    let entry = file.oauth.ok_or(SHAPE_ERROR)?;
    if entry.access_token.is_empty() {
        return Err(SHAPE_ERROR);
    }
    Ok(Credentials {
        access_token: Token(entry.access_token),
        expires_at: entry.expires_at,
        scopes: entry.scopes,
    })
}

#[derive(Debug)]
pub enum ReadError {
    /// 利用者がキーチェーンの確認を拒んだか取り消した。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Denied,
    NotFound,
    Other(String),
}

#[cfg(target_os = "macos")]
pub fn read() -> Result<Vec<u8>, ReadError> {
    use security_framework::item::{ItemClass, ItemSearchOptions, SearchResult};

    // Claude Code は macOS ではログインキーチェーンの汎用パスワードに資格情報を置き、アカウント名は
    // 環境によって違うのでサービス名だけで探す。security コマンドを経由すると確認の画面に
    // /usr/bin/security の名前が出て、許可がそのコマンドに与えられてしまうので、Security
    // フレームワークを直接呼んで marimo 自身の名前で確認させる。
    const SERVICE: &str = "Claude Code-credentials";
    const ERR_SEC_USER_CANCELED: i32 = -128;
    const ERR_SEC_AUTH_FAILED: i32 = -25293;
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    let results = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .load_data(true)
        .search()
        .map_err(|e| match e.code() {
            ERR_SEC_USER_CANCELED | ERR_SEC_AUTH_FAILED => ReadError::Denied,
            ERR_SEC_ITEM_NOT_FOUND => ReadError::NotFound,
            code => ReadError::Other(format!("keychain error {code}")),
        })?;
    // SearchResult の Debug は中身を出すので、形を確かめるだけにする。
    results
        .into_iter()
        .find_map(|r| match r {
            SearchResult::Data(bytes) => Some(bytes),
            _ => None,
        })
        .ok_or(ReadError::NotFound)
}

#[cfg(not(target_os = "macos"))]
pub fn read() -> Result<Vec<u8>, ReadError> {
    let path = file_path(
        std::env::var_os("CLAUDE_CONFIG_DIR"),
        marimo_core::paths::user_home(),
    )
    .ok_or(ReadError::NotFound)?;
    std::fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ReadError::NotFound,
        _ => ReadError::Other(format!("cannot read the credentials file: {}", e.kind())),
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn file_path(
    config_dir: Option<std::ffi::OsString>,
    home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    match config_dir.filter(|d| !d.is_empty()) {
        Some(dir) => Some(std::path::PathBuf::from(dir).join(".credentials.json")),
        None => home.map(|h| h.join(".claude").join(".credentials.json")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const FAKE: &str = "fake-access-token-for-tests";

    fn fixture(expires_at: i64, scopes: &str) -> String {
        format!(
            r#"{{
                "mcpOAuth": {{"some-server": {{"accessToken": "other"}}}},
                "claudeAiOauth": {{
                    "accessToken": "{FAKE}",
                    "refreshToken": "fake-refresh-token",
                    "expiresAt": {expires_at},
                    "refreshTokenExpiresAt": 1822086350472,
                    "scopes": {scopes},
                    "subscriptionType": "max",
                    "rateLimitTier": "default_claude_max_20x"
                }}
            }}"#
        )
    }

    #[test]
    fn parses_the_claude_ai_entry_and_ignores_the_rest() {
        let c =
            parse(fixture(1_790_550_350_472, r#"["user:inference", "user:profile"]"#).as_bytes())
                .unwrap();
        assert_eq!(c.access_token.expose(), FAKE);
        assert_eq!(c.expires_at, 1_790_550_350_472);
        assert_eq!(c.scopes, ["user:inference", "user:profile"]);
    }

    #[test]
    fn debug_output_never_contains_the_token() {
        let c = parse(fixture(1_790_550_350_472, r#"["user:profile"]"#).as_bytes()).unwrap();
        assert!(!format!("{c:?}").contains(FAKE));
    }

    #[test]
    fn malformed_credentials_give_a_fixed_message() {
        for content in [
            "{broken".to_owned(),
            r#"{"mcpOAuth": {}}"#.to_owned(),
            format!(r#"{{"claudeAiOauth": {{"accessToken": "{FAKE}"}}}}"#),
            format!(r#"{{"claudeAiOauth": {{"accessToken": "{FAKE}", "expiresAt": "{FAKE}"}}}}"#),
            r#"{"claudeAiOauth": {"accessToken": "", "expiresAt": 1}}"#.to_owned(),
        ] {
            let err = parse(content.as_bytes()).unwrap_err();
            assert_eq!(err, SHAPE_ERROR, "content {content:?}");
        }
    }

    #[test]
    fn expiry_and_scope_decide_whether_to_call() {
        let now = 1_790_509_325_123_u64;
        let ok = parse(fixture(now as i64 + 60_000, r#"["user:profile"]"#).as_bytes()).unwrap();
        assert_eq!(ok.check(now), Ok(()));
        let expired = parse(fixture(now as i64 - 1, r#"["user:profile"]"#).as_bytes()).unwrap();
        assert_eq!(expired.check(now), Err(Unusable::Expired));
        let no_scope =
            parse(fixture(now as i64 + 60_000, r#"["user:inference"]"#).as_bytes()).unwrap();
        assert_eq!(no_scope.check(now), Err(Unusable::MissingScope));
        let no_list = parse(
            format!(
                r#"{{"claudeAiOauth": {{"accessToken": "{FAKE}", "expiresAt": {}}}}}"#,
                now + 60_000
            )
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(no_list.check(now), Err(Unusable::MissingScope));
    }

    #[test]
    fn credentials_file_prefers_the_config_dir() {
        let home = Some(PathBuf::from("/home/u"));
        assert_eq!(
            file_path(Some("/cfg".into()), home.clone()),
            Some(PathBuf::from("/cfg/.credentials.json"))
        );
        assert_eq!(
            file_path(Some("".into()), home.clone()),
            Some(PathBuf::from("/home/u/.claude/.credentials.json"))
        );
        assert_eq!(
            file_path(None, home),
            Some(PathBuf::from("/home/u/.claude/.credentials.json"))
        );
        assert_eq!(file_path(None, None), None);
    }
}
