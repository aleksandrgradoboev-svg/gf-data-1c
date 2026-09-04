//! Перечень информационных баз, с которыми работает сервер.
//!
//! Мультибаза — исходное требование, а не опция. Отсюда два правила, заложенные в тип:
//! имя базы разрешается явно и только по реестру, а незнакомое имя даёт отказ с перечнем
//! известных — молчаливый уход не в ту базу выглядел бы как достоверный ответ.
//!
//! Базы по умолчанию нет и не предусмотрено. Она была, и 26.08.2026 её вырезали по случаю
//! из живой работы: вызов `object` без base ушёл в базу по умолчанию, документ другой
//! конфигурации там не нашёлся, и модель принялась перебирать имена — «может, он называется
//! иначе». Отказ, не назвавший базу, читается как факт о конфигурации, а не как промах
//! вызова. Поэтому умолчания нет как МЕХАНИЗМА: правило, которое можно обойти, не назвав
//! параметр, исполняется ровно до первой спешки.
//!
//! Учётные данные хранятся отдельно от адреса: в URL они не попадают никогда, иначе рано
//! или поздно уедут в журнал вместе с адресом.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::refusal::{Kind, Refusal};
use crate::secret;

/// Одна информационная база.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user: String,
    /// Хранится защищённым (префикс `dpapi:`). Открытое значение допускается — реестр
    /// правят руками, — но при первом же сохранении оно шифруется.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
    /// Способ аутентификации: `basic` (умолчание) или `ntlm` для доменных учёток вида
    /// `ДОМЕН\пользователь`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auth: String,
}

impl Base {
    /// Пароль в открытом виде — только в момент обращения к базе.
    pub fn reveal_password(&self) -> Result<String, Refusal> {
        secret::reveal(&self.password)
            .map_err(|e| Refusal::new(Kind::Internal, "пароль базы не прочитан", e))
    }
}

/// Реестр баз. Базы по умолчанию у него нет: см. шапку модуля.
///
/// Ключ `default` в старых файлах реестра остаётся нераспознанным и просто игнорируется
/// при чтении — отдельной миграции это не требует.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub bases: Vec<Base>,

    #[serde(skip)]
    path: PathBuf,
}

/// Путь реестра по умолчанию. Регистрация сервера сводится к пути бинарника: ни флагов,
/// ни обёрток, ни переменных окружения не требуется.
///
/// Каталог `gt-data-1c` остался прежним при переименовании продукта в `gf-data-1c`
/// (04.09.2026): в нём лежат пароли, защищённые DPAPI, и переезд осиротил бы реестр уже
/// работающих установок. Имя каталога — не витрина.
pub fn default_path() -> PathBuf {
    let dir = config_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.join("gt-data-1c").join("bases.json")
}

/// Каталог настроек пользователя: `%AppData%` в Windows, `$XDG_CONFIG_HOME` или
/// `~/.config` в прочих системах — то же, что даёт `os.UserConfigDir` в Go.
fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }
}

impl Registry {
    /// Читает реестр. Отсутствующий файл — не ошибка: это пустой реестр, в который
    /// сейчас добавят первую базу.
    pub fn load(path: Option<&Path>) -> Result<Self, Refusal> {
        let path = match path {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => default_path(),
        };

        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    bases: Vec::new(),
                    path,
                });
            }
            Err(e) => {
                return Err(
                    Refusal::new(Kind::Internal, "реестр баз не прочитан", e.to_string())
                        .hint(format!("проверьте доступ к файлу {}", path.display())),
                );
            }
        };

        let mut r: Registry = serde_json::from_slice(&data).map_err(|e| {
            Refusal::new(Kind::Internal, "реестр баз испорчен", e.to_string())
                .hint(format!("файл: {}", path.display()))
        })?;
        r.path = path;

        // Пароль, вписанный руками или оставшийся от прежней версии, защищается при
        // первом же чтении. Иначе он лежит открытым до ближайшего изменения реестра,
        // а изменений может не быть месяцами.
        if secret::available() && r.has_plain_passwords() {
            // Не повод отказывать в работе: пароль остаётся открытым, но канал жив.
            let _ = r.save();
        }
        Ok(r)
    }

    fn has_plain_passwords(&self) -> bool {
        self.bases
            .iter()
            .any(|b| !b.password.is_empty() && !secret::is_protected(&b.password))
    }

    /// Записывает реестр, создавая каталог при необходимости.
    ///
    /// Перед записью пароли шифруются: открытый пароль, вписанный руками, переживает
    /// сохранение ровно один раз — дальше в файле лежит защищённое значение.
    pub fn save(&mut self) -> Result<(), Refusal> {
        for b in &mut self.bases {
            b.password = secret::protect(&b.password).map_err(|e| {
                Refusal::new(Kind::Internal, "пароль базы не защищён", e)
                    .hint(format!("база: {}", b.name))
            })?;
        }

        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                Refusal::new(Kind::Internal, "каталог реестра не создан", e.to_string())
            })?;
        }
        let data = serde_json::to_vec_pretty(self)
            .map_err(|e| Refusal::new(Kind::Internal, "реестр не сериализован", e.to_string()))?;

        // В файле лежат пароли к базам, поэтому права 0600. В Windows файловые права
        // работают иначе, и разграничение там даёт DPAPI — привязка к учётной записи.
        write_private(&self.path, &data)
            .map_err(|e| Refusal::new(Kind::Internal, "реестр не записан", e.to_string()))
    }

    /// Путь файла реестра (нужен для диагностики).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Имена баз в устойчивом порядке.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.bases.iter().map(|b| b.name.clone()).collect();
        names.sort();
        names
    }

    /// Возвращает базу по имени. Пустое имя — всегда отказ: базу выбирает вызывающий,
    /// сервер за него не выбирает никогда.
    ///
    /// Поблажки «если база одна — бери её» здесь тоже нет, и это не строгость ради
    /// строгости: с нею умолчание возвращается само в тот день, когда баз в реестре
    /// останется одна, — а правило, действующее не всегда, не действует.
    pub fn resolve(&self, name: &str) -> Result<Base, Refusal> {
        let name = name.trim();
        if name.is_empty() {
            let why = if self.bases.is_empty() {
                "реестр баз пуст".to_string()
            } else {
                format!("в реестре баз: {}", self.bases.len())
            };
            return Err(Refusal::new(Kind::BadRequest, "база не названа", why)
                .hint("назовите базу параметром base — базы по умолчанию у сервера нет")
                .hint("перечень баз — инструмент bases с action=list"));
        }
        self.bases
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(name) || equal_fold(&b.name, name))
            .cloned()
            .ok_or_else(|| Refusal::unknown_base(name, &self.names()))
    }

    /// Добавляет базу или заменяет одноимённую.
    pub fn add(&mut self, base: Base) -> Result<(), Refusal> {
        if base.name.trim().is_empty() || base.url.trim().is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "база не добавлена",
                "нужны имя и адрес HTTP-сервиса базы",
            ));
        }
        if let Some(slot) = self
            .bases
            .iter_mut()
            .find(|b| equal_fold(&b.name, &base.name))
        {
            *slot = base;
        } else {
            self.bases.push(base);
        }
        self.save()
    }

    /// Применяет патч к базе. Возвращает изменённую базу и перечень изменённых полей —
    /// вызывающему есть что показать человеку, а «ничего не изменилось» не выглядит
    /// успехом.
    pub fn update(&mut self, p: &Patch) -> Result<(Base, Vec<String>), Refusal> {
        let name = p.name.trim();
        if name.is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "база не названа",
                "нужно имя базы, которую правим",
            )
            .hint("перечень баз — инструмент bases с action=list"));
        }

        let idx = self
            .bases
            .iter()
            .position(|b| equal_fold(&b.name, name))
            .ok_or_else(|| Refusal::unknown_base(name, &self.names()))?;

        let mut base = self.bases[idx].clone();
        let mut changed: Vec<String> = Vec::new();

        let url = p.url.trim();
        if !url.is_empty() && url != base.url {
            base.url = url.to_string();
            changed.push("адрес".into());
        }
        let user = p.user.trim();
        if !user.is_empty() && user != base.user {
            base.user = user.to_string();
            changed.push("пользователь".into());
        }
        let auth = p.auth.trim();
        if !auth.is_empty() && !equal_fold(auth, &base.auth) {
            base.auth = auth.to_string();
            changed.push("аутентификация".into());
        }
        // Пароль не сравнивается со старым: хранится он защищённым, а сравнивать
        // открытое с шифрованным — значит расшифровывать ради сравнения.
        if !p.password.is_empty() {
            base.password = p.password.clone();
            changed.push("пароль".into());
        }
        if p.clear_title {
            if !base.title.is_empty() {
                base.title.clear();
                changed.push("название снято".into());
            }
        } else {
            let title = p.title.trim();
            if !title.is_empty() && title != base.title {
                base.title = title.to_string();
                changed.push("название".into());
            }
        }

        if changed.is_empty() {
            return Ok((base, changed));
        }

        self.bases[idx] = base.clone();
        self.save()?;
        Ok((base, changed))
    }

    /// Убирает базу из реестра.
    pub fn remove(&mut self, name: &str) -> Result<(), Refusal> {
        match self.bases.iter().position(|b| equal_fold(&b.name, name)) {
            Some(i) => {
                self.bases.remove(i);
                self.save()
            }
            None => Err(Refusal::unknown_base(name, &self.names())),
        }
    }
}

/// Правит существующую базу, НЕ трогая незаполненные поля.
///
/// Отдельно от `add` потому, что тот заменяет запись целиком: правка адреса через него
/// стирает пароль, если его не передали заново. А пароль наверх не отдаётся (и не должен),
/// поэтому UI физически не может прислать его обратно — он его не видел. Отсюда правило:
/// пустое поле здесь значит «оставить как было», а не «очистить».
///
/// Исключение — `title`: имя, стёртое намеренно, обязано стираться. Для него пустая строка
/// это значение, поэтому очистка заголовка передаётся явным `clear_title`.
#[derive(Debug, Clone, Default)]
pub struct Patch {
    pub name: String,
    pub title: String,
    pub url: String,
    pub user: String,
    pub password: String,
    pub auth: String,
    /// Отличает «заголовок не меняли» от «заголовок стёрли».
    pub clear_title: bool,
}

/// Сравнение имён без учёта регистра, включая кириллицу: `EqualFold` в Go работает по
/// Unicode, и «УТ11» обязано находить базу «ут11».
fn equal_fold(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
}

/// Запись файла с правами только для владельца.
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(data)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn временный(имя: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gfdata-reg-{}-{имя}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("bases.json")
    }

    fn реестр(path: &Path) -> Registry {
        Registry {
            bases: Vec::new(),
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn отсутствующий_файл_это_пустой_реестр_а_не_ошибка() {
        let p = временный("нет-файла");
        let _ = std::fs::remove_file(&p);
        let r = Registry::load(Some(&p)).expect("пустой реестр — законное состояние");
        assert!(r.bases.is_empty());
    }

    #[test]
    fn база_без_имени_даёт_отказ_а_не_умолчание() {
        let r = реестр(&временный("умолчание"));
        let err = r.resolve("").unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
        assert!(
            err.to_string().contains("базы по умолчанию у сервера нет"),
            "{err}"
        );
    }

    #[test]
    fn единственная_база_не_становится_умолчанием() {
        let mut r = реестр(&временный("одна"));
        r.bases.push(Base {
            name: "ut11".into(),
            url: "http://localhost/ut11/hs/gt-data".into(),
            ..Default::default()
        });
        assert!(
            r.resolve("").is_err(),
            "правило, действующее не всегда, не действует"
        );
    }

    #[test]
    fn незнакомая_база_отвечает_перечнем_известных() {
        let mut r = реестр(&временный("незнакомая"));
        r.bases.push(Base {
            name: "ut11".into(),
            url: "http://x".into(),
            ..Default::default()
        });
        let err = r.resolve("нетбазы").unwrap_err();
        assert_eq!(err.kind, Kind::UnknownBase);
        assert!(err.to_string().contains("ut11"), "{err}");
    }

    #[test]
    fn имя_базы_находится_без_учёта_регистра_включая_кириллицу() {
        let mut r = реестр(&временный("регистр"));
        r.bases.push(Base {
            name: "ут11".into(),
            url: "http://x".into(),
            ..Default::default()
        });
        assert!(r.resolve("УТ11").is_ok(), "EqualFold в Go знает Unicode");
    }

    #[test]
    fn пустой_патч_ничего_не_меняет() {
        let p = временный("пустой-патч");
        let mut r = реестр(&p);
        r.bases.push(Base {
            name: "ut11".into(),
            url: "http://x".into(),
            title: "Торговля".into(),
            ..Default::default()
        });
        let (_, changed) = r
            .update(&Patch {
                name: "ut11".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(changed.is_empty(), "изменения: {changed:?}");
    }

    #[test]
    fn патч_не_трогает_соседние_поля() {
        let p = временный("соседние");
        let mut r = реестр(&p);
        r.bases.push(Base {
            name: "ut11".into(),
            url: "http://старый".into(),
            user: "вася".into(),
            password: "секрет".into(),
            ..Default::default()
        });
        let (base, changed) = r
            .update(&Patch {
                name: "ut11".into(),
                url: "http://новый".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(changed, vec!["адрес".to_string()]);
        assert_eq!(
            base.user, "вася",
            "пользователь не передавался — не трогаем"
        );
        assert!(
            !base.password.is_empty(),
            "пароль наверх не отдаётся, поэтому пустое поле значит «оставить»"
        );
    }

    #[test]
    fn название_снимается_только_явно() {
        let p = временный("название");
        let mut r = реестр(&p);
        r.bases.push(Base {
            name: "ut11".into(),
            url: "http://x".into(),
            title: "Торговля".into(),
            ..Default::default()
        });

        let (base, changed) = r
            .update(&Patch {
                name: "ut11".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(base.title, "Торговля", "пустая строка — не команда стереть");
        assert!(changed.is_empty());

        let (base, changed) = r
            .update(&Patch {
                name: "ut11".into(),
                clear_title: true,
                ..Default::default()
            })
            .unwrap();
        assert!(base.title.is_empty());
        assert_eq!(changed, vec!["название снято".to_string()]);
    }

    #[test]
    fn база_без_адреса_не_добавляется() {
        let mut r = реестр(&временный("без-адреса"));
        let err = r
            .add(Base {
                name: "ut11".into(),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.kind, Kind::BadRequest);
    }

    #[test]
    fn удаление_незнакомой_базы_даёт_отказ() {
        let mut r = реестр(&временный("удаление"));
        assert_eq!(r.remove("нетбазы").unwrap_err().kind, Kind::UnknownBase);
    }

    #[test]
    fn сохранение_защищает_открытый_пароль() {
        let p = временный("защита");
        let _ = std::fs::remove_file(&p);
        let mut r = реестр(&p);
        r.bases.push(Base {
            name: "ut11".into(),
            url: "http://x".into(),
            password: "открытый".into(),
            ..Default::default()
        });
        r.save().unwrap();

        let текст = std::fs::read_to_string(&p).unwrap();
        if secret::available() {
            assert!(
                !текст.contains("открытый"),
                "пароль обязан быть защищён при первом же сохранении: {текст}"
            );
        }
        let обратно = Registry::load(Some(&p)).unwrap();
        assert_eq!(обратно.bases[0].reveal_password().unwrap(), "открытый");
    }
}
