//! Реестр баз: посмотреть, добавить, поправить, убрать.
//!
//! Единственный инструмент, которому не нужен параметр `base`: он не ходит в базу,
//! а управляет списком. С него начинают, когда не знают, какие базы есть.

use serde::Deserialize;

use crate::refusal::{Kind, Refusal};
use crate::registry::{Base, Patch, Registry};
use crate::secret;

use super::Set;

/// Параметры инструмента управления реестром баз.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct BasesInput {
    /// Что сделать с реестром: `list` (умолчание), `add`, `update`, `remove`.
    pub action: String,
    /// Короткий ключ базы, которым она называется в параметре `base`.
    pub name: String,
    /// Адрес HTTP-сервиса базы.
    pub url: String,
    /// Пользователь 1С для HTTP-сервиса.
    pub user: String,
    /// Пароль пользователя 1С. Хранится защищённым, в открытом виде в файл не попадает.
    pub password: String,
    /// Человекочитаемое название базы для списка.
    pub title: String,
    /// При `update`: снять название базы. Пустой `title` означает «не менять», поэтому
    /// очистка передаётся отдельным признаком.
    pub clear_title: bool,
    /// Способ аутентификации: `basic` (умолчание) или `ntlm` для доменной учётки.
    /// Логин вида `ДОМЕН\пользователь` опознаётся как доменный сам.
    pub auth: String,
}

/// Имя инструмента для агента.
pub const NAME: &str = "bases";

/// Описание инструмента для агента.
pub const DESCRIPTION: &str = "Список зарегистрированных баз 1С и управление реестром: \
посмотреть, какие базы доступны (action=list), зарегистрировать новую по адресу её \
HTTP-сервиса (action=add), поправить существующую (action=update: пустое поле значит \
«не менять», пароль при этом остаётся прежним), убрать (action=remove). Имя базы из этого \
списка передаётся остальным инструментам параметром base — он обязателен у всех, базы по \
умолчанию у сервера нет. Начинай с action=list, когда не знаешь, какие базы есть.";

impl Set {
    /// Обслуживает реестр баз.
    pub fn bases(&self, input: &BasesInput) -> Result<String, Refusal> {
        let mut reg = self.registry()?;

        let action = input.action.trim().to_lowercase();
        let action = if action.is_empty() { "list" } else { &action };

        match action {
            "list" => Ok(list_bases(&reg)),

            "add" => {
                reg.add(Base {
                    name: input.name.clone(),
                    title: input.title.clone(),
                    url: input.url.clone(),
                    user: input.user.clone(),
                    password: input.password.clone(),
                    auth: input.auth.clone(),
                })?;
                Ok(format!(
                    "База {:?} добавлена в реестр ({}).\n\n{}",
                    input.name,
                    reg.path().display(),
                    list_bases(&reg)
                ))
            }

            "update" => {
                let (base, changed) = reg.update(&Patch {
                    name: input.name.clone(),
                    title: input.title.clone(),
                    url: input.url.clone(),
                    user: input.user.clone(),
                    password: input.password.clone(),
                    auth: input.auth.clone(),
                    clear_title: input.clear_title,
                })?;
                if changed.is_empty() {
                    // «Ничего не изменилось» — это ответ, а не успех: иначе правка,
                    // не доехавшая из-за опечатки в поле, выглядит применённой.
                    Ok(format!(
                        "База {:?}: изменений нет — все переданные поля совпадают с текущими.\n\n{}",
                        base.name,
                        list_bases(&reg)
                    ))
                } else {
                    Ok(format!(
                        "База {:?} изменена ({}).\n\n{}",
                        base.name,
                        changed.join(", "),
                        list_bases(&reg)
                    ))
                }
            }

            "remove" => {
                reg.remove(&input.name)?;
                Ok(format!(
                    "База {:?} убрана из реестра.\n\n{}",
                    input.name,
                    list_bases(&reg)
                ))
            }

            _ => {
                let mut r = Refusal::new(
                    Kind::BadRequest,
                    "действие не распознано",
                    format!("action={:?}", input.action),
                )
                .hint("допустимо: list, add, update, remove");
                if action == "set_default" {
                    // Старый вызов обязан объяснить, что механизм убран, а не выглядеть
                    // опечаткой.
                    r = r.hint(
                        "базы по умолчанию больше нет: параметр base обязателен у всех \
                         инструментов данных",
                    );
                }
                Err(r)
            }
        }
    }
}

/// Печатает реестр. Пароли не показываются никогда — ни маской, ни длиной.
fn list_bases(reg: &Registry) -> String {
    if reg.bases.is_empty() {
        return format!(
            "Реестр баз пуст.\nДобавьте базу: action=add, name=<ключ>, url=<адрес HTTP-сервиса>, \
             user, password.\nФайл реестра: {}",
            reg.path().display()
        );
    }

    let mut out = format!(
        "Баз в реестре: {}. Файл: {}\n\n",
        reg.bases.len(),
        reg.path().display()
    );
    for base in &reg.bases {
        out.push_str(&format!("  {}", base.name));
        if !base.title.is_empty() {
            out.push_str(&format!(" — {}", base.title));
        }
        out.push_str(&format!("\n    адрес: {}\n", base.url));
        if !base.user.is_empty() {
            out.push_str(&format!("    пользователь: {}", base.user));
            if secret::is_protected(&base.password) {
                out.push_str(", пароль защищён");
            } else if !base.password.is_empty() {
                out.push_str(", пароль ОТКРЫТЫМ ТЕКСТОМ (будет защищён при первом сохранении)");
            }
            out.push('\n');
        }
        if !base.auth.is_empty() {
            out.push_str(&format!("    аутентификация: {}\n", base.auth));
        }
    }
    out.push_str("\nБазы по умолчанию нет: любой инструмент данных требует base явно.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn набор(имя: &str) -> Set {
        let dir = std::env::temp_dir().join(format!("gfdata-bases-{}-{имя}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bases.json");
        let _ = std::fs::remove_file(&path);
        let mut s = Set::new("0.1.0");
        s.registry_path = Some(path);
        s
    }

    #[test]
    fn пустой_реестр_говорит_что_он_пуст_и_как_добавить() {
        let s = набор("пусто");
        let out = s.bases(&BasesInput::default()).unwrap();
        assert!(out.contains("Реестр баз пуст"), "{out}");
        assert!(
            out.contains("action=add"),
            "отказ обязан звать дальше: {out}"
        );
    }

    #[test]
    fn действие_по_умолчанию_это_список() {
        let s = набор("умолчание");
        let out = s.bases(&BasesInput::default()).unwrap();
        assert!(out.contains("Реестр баз пуст"));
    }

    #[test]
    fn добавленная_база_видна_в_списке() {
        let s = набор("добавление");
        let out = s
            .bases(&BasesInput {
                action: "add".into(),
                name: "ut11".into(),
                url: "http://localhost:8081/ut11/hs/gt-data".into(),
                user: "agent".into(),
                password: "тайна".into(),
                title: "Торговля".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(out.contains("добавлена в реестр"), "{out}");
        assert!(out.contains("ut11") && out.contains("Торговля"), "{out}");
    }

    #[test]
    fn пароль_не_показывается_ни_маской_ни_длиной() {
        let s = набор("пароль");
        let out = s
            .bases(&BasesInput {
                action: "add".into(),
                name: "ut11".into(),
                url: "http://x".into(),
                user: "agent".into(),
                password: "очень-секретный-пароль".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(
            !out.contains("очень-секретный-пароль"),
            "пароль утёк в вывод: {out}"
        );
        assert!(
            out.contains("пароль защищён") || out.contains("ОТКРЫТЫМ ТЕКСТОМ"),
            "{out}"
        );
    }

    #[test]
    fn правка_без_изменений_не_выглядит_успехом() {
        let s = набор("без-изменений");
        s.bases(&BasesInput {
            action: "add".into(),
            name: "ut11".into(),
            url: "http://x".into(),
            ..Default::default()
        })
        .unwrap();

        let out = s
            .bases(&BasesInput {
                action: "update".into(),
                name: "ut11".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(
            out.contains("изменений нет"),
            "иначе правка с опечаткой выглядит применённой: {out}"
        );
    }

    #[test]
    fn незнакомое_действие_даёт_отказ_с_перечнем() {
        let s = набор("чепуха");
        let err = s
            .bases(&BasesInput {
                action: "чепуха".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(
            err.to_string().contains("list, add, update, remove"),
            "{err}"
        );
    }

    #[test]
    fn старый_set_default_объясняет_что_механизм_убран() {
        let s = набор("set-default");
        let err = s
            .bases(&BasesInput {
                action: "set_default".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(
            err.to_string().contains("базы по умолчанию больше нет"),
            "старый вызов не должен выглядеть опечаткой: {err}"
        );
    }

    #[test]
    fn список_напоминает_что_умолчания_нет() {
        let s = набор("напоминание");
        s.bases(&BasesInput {
            action: "add".into(),
            name: "ut11".into(),
            url: "http://x".into(),
            ..Default::default()
        })
        .unwrap();
        let out = s.bases(&BasesInput::default()).unwrap();
        assert!(out.contains("Базы по умолчанию нет"), "{out}");
    }
}
