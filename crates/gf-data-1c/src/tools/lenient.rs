//! Лояльный приём списков.
//!
//! Слабые модели заворачивают JSON-массив в строку — `"resources": "[\"СуммаОборотДт\"]"` —
//! и строгая схема отвечает сырым отказом валидатора, который модель прочитать не умеет:
//! 18 отказов одной природы за одну живую сессию 02.09.2026. Сервер строится для слабых
//! моделей, поэтому строку с массивом внутри принимает и разбирает сам.

use serde::{Deserialize, Deserializer};

/// `Vec<String>`, принимающий три формы: массив строк, строку с JSON-массивом внутри и
/// одиночную строку (список из одного элемента).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringList(pub Vec<String>);

impl StringList {
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for StringList {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            List(Vec<String>),
            One(String),
        }

        Ok(match Raw::deserialize(d)? {
            Raw::List(v) => StringList(v),
            Raw::One(s) => {
                let trimmed = s.trim();
                if trimmed.starts_with('[') {
                    match serde_json::from_str::<Vec<String>>(trimmed) {
                        Ok(v) => StringList(v),
                        // Строка похожа на массив, но им не является — берём как есть:
                        // отказ здесь был бы придиркой к форме, а не к смыслу.
                        Err(_) => StringList(vec![s]),
                    }
                } else {
                    StringList(vec![s])
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn разобрать(json: &str) -> StringList {
        serde_json::from_str(json).expect("разбор не удался")
    }

    #[test]
    fn массив_строк_принимается() {
        assert_eq!(
            разобрать(r#"["Количество","Сумма"]"#).0,
            vec!["Количество", "Сумма"]
        );
    }

    #[test]
    fn массив_завёрнутый_в_строку_разбирается() {
        assert_eq!(
            разобрать(r#""[\"СуммаОборотДт\"]""#).0,
            vec!["СуммаОборотДт"],
            "ровно эта форма дала 18 отказов за одну живую сессию"
        );
    }

    #[test]
    fn одиночная_строка_это_список_из_одного() {
        assert_eq!(разобрать(r#""Количество""#).0, vec!["Количество"]);
    }

    #[test]
    fn пустой_массив_остаётся_пустым() {
        assert!(разобрать("[]").is_empty());
    }

    #[test]
    fn похожая_на_массив_но_битая_строка_берётся_как_есть() {
        // Отказ здесь был бы придиркой к форме: пусть база скажет, что не так со смыслом.
        assert_eq!(разобрать(r#""[не json""#).0, vec!["[не json"]);
    }
}
