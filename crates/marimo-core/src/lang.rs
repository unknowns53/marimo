use std::fs;

use serde::{Deserialize, Serialize};

use crate::MarimoHome;

/// 画面、吹き出し、パネルの要約、CLI の出力に使う言語。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    /// 英語版より前から書かれてきたテストが日本語の出力を前提にしているので、組み立てただけの入力は日本語にする。
    #[default]
    Ja,
    En,
}

/// display.json の `language` に保存する選択。Auto は OS の言語に従う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LangSetting {
    #[default]
    Auto,
    Ja,
    En,
}

impl LangSetting {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "ja" => Some(Self::Ja),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ja => "ja",
            Self::En => "en",
        }
    }

    pub fn resolve(self) -> Lang {
        match self {
            Self::Auto => Lang::os(),
            Self::Ja => Lang::Ja,
            Self::En => Lang::En,
        }
    }

    /// 無いとき、読めないとき、知らない値のときは Auto にする。アプリとフックとスクリプトが同じ規則で読む。
    pub fn load(home: &MarimoHome) -> Self {
        fs::read(home.display_file())
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("language")?.as_str().and_then(Self::parse))
            .unwrap_or_default()
    }
}

impl Lang {
    /// sys-locale は macOS では利用者の優先する言語の先頭を、Windows では表示言語を返し、環境変数の LANG を見ない。
    /// ターミナルから呼ばれたフックとデスクトップアプリから呼ばれたフックで、言語が食い違わないようにするためである。
    pub fn os() -> Self {
        sys_locale::get_locale().map_or(Self::En, |tag| Self::from_tag(&tag))
    }

    /// `ja`、`ja-JP`、`ja_JP.UTF-8` のような BCP 47 か POSIX の名前を受ける。日本語以外はすべて英語にする。
    pub fn from_tag(tag: &str) -> Self {
        let primary = tag.split(['-', '_', '.']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("ja") {
            Self::Ja
        } else {
            Self::En
        }
    }

    pub fn resolve(home: &MarimoHome) -> Self {
        LangSetting::load(home).resolve()
    }

    pub fn pick<T>(self, ja: T, en: T) -> T {
        match self {
            Self::Ja => ja,
            Self::En => en,
        }
    }

    pub fn code(self) -> &'static str {
        self.pick("ja", "en")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_choose_japanese_only_for_ja() {
        for (tag, want) in [
            ("ja", Lang::Ja),
            ("ja-JP", Lang::Ja),
            ("JA_jp.UTF-8", Lang::Ja),
            ("en-JP", Lang::En),
            ("en_US.UTF-8", Lang::En),
            ("jav", Lang::En),
            ("", Lang::En),
        ] {
            assert_eq!(Lang::from_tag(tag), want, "{tag:?}");
        }
    }

    #[test]
    fn setting_falls_back_to_auto() {
        let dir = tempfile::tempdir().unwrap();
        let home = MarimoHome::at(dir.path());
        assert_eq!(LangSetting::load(&home), LangSetting::Auto);
        for (content, want) in [
            (r#"{"language": "en"}"#, LangSetting::En),
            (r#"{"language": "ja", "stage_scale": 1.2}"#, LangSetting::Ja),
            (r#"{"language": "auto"}"#, LangSetting::Auto),
            (r#"{"language": "fr"}"#, LangSetting::Auto),
            (r#"{"language": 1}"#, LangSetting::Auto),
            ("{broken", LangSetting::Auto),
        ] {
            fs::write(home.display_file(), content).unwrap();
            assert_eq!(LangSetting::load(&home), want, "{content}");
        }
        fs::write(home.display_file(), r#"{"language": "en"}"#).unwrap();
        assert_eq!(Lang::resolve(&home), Lang::En);
    }
}
