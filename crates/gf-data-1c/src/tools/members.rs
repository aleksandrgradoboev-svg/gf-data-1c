//! Члены типов платформы: что умеет ТаблицаЗначений, Запрос, ТабличныйДокумент.
//!
//! Вопрос «что умеет тип» — законный и частый, но обзорная страница про назначение объекта
//! на него не отвечает: перечень методов и свойств у вендора разложен по отдельным
//! страницам, по одной на член. Здесь они собираются обратно в перечень — один ответ
//! вместо тридцати вызовов.

use std::collections::HashMap;

use regex::Regex;
use rusqlite::Connection;

/// Раздел, в котором член лежит у вендора.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemberKind {
    Method,
    Property,
    Event,
    Ctor,
}

impl MemberKind {
    /// Имя раздела в ответе — русское, как его читает модель.
    fn title(self) -> &'static str {
        match self {
            MemberKind::Method => "методы",
            MemberKind::Property => "свойства",
            MemberKind::Event => "события",
            MemberKind::Ctor => "конструкторы",
        }
    }

    /// Каталог пути → раздел. Имена каталогов заданы вендором, а не нами.
    fn from_dir(dir: &str) -> Option<Self> {
        match dir {
            "methods" => Some(MemberKind::Method),
            "properties" => Some(MemberKind::Property),
            "events" => Some(MemberKind::Event),
            "ctors" => Some(MemberKind::Ctor),
            _ => None,
        }
    }

    /// Порядок разделов в ответе: сперва то, чем пользуются чаще.
    fn order(self) -> u8 {
        match self {
            MemberKind::Method => 0,
            MemberKind::Property => 1,
            MemberKind::Event => 2,
            MemberKind::Ctor => 3,
        }
    }
}

/// Один член типа.
#[derive(Debug, Clone, Default)]
pub struct TypeMember {
    /// Русское имя: Добавить.
    pub name: String,
    /// Английское: Add.
    pub name_en: String,
    pub kind: Option<MemberKind>,
    /// Заголовок страницы целиком.
    pub title: String,
    pub path: String,
    /// Возвращаемый тип, если справка его называет.
    pub returns: String,
}

/// Члены одного типа, сгруппированные по разделам.
#[derive(Debug, Clone, Default)]
pub struct TypeMembers {
    pub type_ru: String,
    pub type_en: String,
    pub members: Vec<TypeMember>,
}

/// Все типы, у которых есть разобранные члены.
pub struct MemberIndex {
    /// Нормализованное имя типа (РУ и EN) → индекс записи в `entries`.
    by_type: HashMap<String, usize>,
    entries: Vec<TypeMembers>,
}

/// Заголовок страницы члена: «ТаблицаЗначений.Добавить (ValueTable.Add)».
fn title_rx() -> &'static Regex {
    static RX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RX.get_or_init(|| Regex::new(r"^(.+?)\.(.+?)\s+\((.+?)\.(.+?)\)\s*$").unwrap())
}

/// Раздел и английское имя типа из пути вендора.
fn path_rx() -> &'static Regex {
    static RX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RX.get_or_init(|| Regex::new(r"/([^/]+)/(methods|properties|events|ctors)/").unwrap())
}

/// Строка «Возвращаемое значение:» и тип за ней. Справка называет его не всегда,
/// и отсутствие типа — не ошибка разбора: у процедур его нет.
fn returns_rx() -> &'static Regex {
    static RX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RX.get_or_init(|| Regex::new(r"(?i)Возвращаемое значение:\s*\n?\s*([^\n]{1,80})").unwrap())
}

/// Последний сегмент пути без расширения и без хвостового номера страницы:
/// `.../methods/Insert582.html` → `Insert`.
///
/// Номер приписывает распаковщик справки, он не часть имени метода. Расширение любое:
/// в справке встречаются и `.html`, и `.st` — привязка к одному `.html` молча теряла
/// половину членов типа (34 из 65 у ТаблицаЗначений).
fn file_rx() -> &'static Regex {
    static RX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RX.get_or_init(|| Regex::new(r"/([A-Za-z][A-Za-z0-9]*?)\d*\.[A-Za-z0-9]+$").unwrap())
}

/// Достаёт латинское имя члена из пути страницы.
///
/// Нужно там, где заголовок оказался служебной строкой формата hbk: имя в пути есть всегда,
/// а заголовок бывает битым.
fn member_name_from_path(path: &str) -> String {
    file_rx()
        .captures(path)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// Первое непустое из двух.
///
/// Ключ дедупликации строится по ЛАТИНСКОМУ имени: оно есть у обеих форм записи члена,
/// русское — только у разобранных из заголовка.
fn first_non_empty<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.is_empty() {
        b
    } else {
        a
    }
}

/// Какой из двух одинаковых членов оставить.
///
/// Русское имя и тип возврата приходят только из нормального заголовка; восстановленный
/// из пути беднее и уступает.
fn better_member(a: &TypeMember, b: &TypeMember) -> bool {
    let ar = !a.name.is_empty() && a.name != a.name_en;
    let br = !b.name.is_empty() && b.name != b.name_en;
    if ar != br {
        return ar;
    }
    a.returns.len() > b.returns.len()
}

/// Тип возврата без служебной обёртки справки.
///
/// Вендор пишет его строкой «Тип: СтрокаТаблицыЗначений.» — слово «Тип:» и точка в перечне
/// только мешают, а внутри перечисление через запятую сохраняется: «Число, Неопределено»
/// это два допустимых типа, а не мусор.
fn clean_returns(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("Тип:").unwrap_or(s);
    let s = s.strip_prefix("тип:").unwrap_or(s);
    s.trim().trim_end_matches(['.', ' ']).trim().to_string()
}

impl MemberIndex {
    /// Строит индекс членов по базе справки.
    pub fn build(db: &Connection) -> Self {
        let mut entries: Vec<TypeMembers> = Vec::new();
        let mut by_en: HashMap<String, usize> = HashMap::new();

        let Ok(mut stmt) = db.prepare(
            "SELECT title, path, text FROM pages
              WHERE config='platform'
                AND (path LIKE '%/methods/%' OR path LIKE '%/properties/%'
                     OR path LIKE '%/events/%' OR path LIKE '%/ctors/%')",
        ) else {
            return MemberIndex {
                by_type: HashMap::new(),
                entries,
            };
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        });
        let Ok(rows) = rows else {
            return MemberIndex {
                by_type: HashMap::new(),
                entries,
            };
        };

        for (mut title, path, text) in rows.flatten() {
            let Some(pm) = path_rx().captures(&path) else {
                continue;
            };
            let type_en = pm[1].to_string();
            let Some(kind) = MemberKind::from_dir(&pm[2]) else {
                continue;
            };

            // Служебная строка формата hbk вместо заголовка — имя берётся из пути.
            if title.trim().starts_with('{') {
                let name = member_name_from_path(&path);
                if name.is_empty() {
                    continue;
                }
                title = name;
            }

            let mut m = TypeMember {
                kind: Some(kind),
                title: title.clone(),
                path: path.clone(),
                ..Default::default()
            };
            let mut type_ru = String::new();
            // Заголовок вида «Тип.Член (Type.Member)» разбирается у 394 страниц из 400.
            // Остальные записаны без префикса типа («Прочитать (Read)») — там имя типа
            // берётся из пути, а русское остаётся пустым и подставляется от собратьев
            // по типу ниже.
            if let Some(tm) = title_rx().captures(title.trim()) {
                type_ru = tm[1].to_string();
                m.name = tm[2].to_string();
                m.name_en = tm[4].to_string();
            } else {
                m.name = title.trim().to_string();
                if let Some(i) = m.name.find(" (") {
                    m.name_en = m.name[i + 2..].trim_matches(['(', ')', ' ']).to_string();
                    m.name = m.name[..i].to_string();
                }
            }
            if let Some(r) = returns_rx().captures(&text) {
                m.returns = clean_returns(&r[1]);
            }

            let key = type_en.to_lowercase();
            let idx = *by_en.entry(key).or_insert_with(|| {
                entries.push(TypeMembers {
                    type_en: type_en.clone(),
                    type_ru: type_ru.clone(),
                    members: Vec::new(),
                });
                entries.len() - 1
            });
            if entries[idx].type_ru.is_empty() && !type_ru.is_empty() {
                entries[idx].type_ru = type_ru;
            }
            entries[idx].members.push(m);
        }

        // Русское имя типа известно только из заголовков его членов, поэтому связь РУ → члены
        // заводится вторым проходом, когда имя уже собрано.
        let mut by_type: HashMap<String, usize> = HashMap::new();
        for (idx, e) in entries.iter_mut().enumerate() {
            // Член, восстановленный из пути, — запасной вариант для страниц со служебным
            // заголовком. Если тот же член разобран из нормального заголовка, он и остаётся:
            // у него есть русское имя и тип возврата. Иначе перечень двоится («Add» рядом
            // с «Вставить (Insert)») и выглядит вдвое богаче, чем есть.
            let mut by_key: HashMap<String, TypeMember> = HashMap::new();
            for m in std::mem::take(&mut e.members) {
                let kind_name = m.kind.map(|k| k.title()).unwrap_or("");
                let key = format!("{}\u{0}{}", kind_name, first_non_empty(&m.name_en, &m.name))
                    .to_lowercase();
                match by_key.get(&key) {
                    Some(cur) if !better_member(&m, cur) => {}
                    _ => {
                        by_key.insert(key, m);
                    }
                }
            }
            e.members = by_key.into_values().collect();
            e.members.sort_by(|a, b| {
                let (ka, kb) = (
                    a.kind.map(|k| k.order()).unwrap_or(9),
                    b.kind.map(|k| k.order()).unwrap_or(9),
                );
                ka.cmp(&kb).then_with(|| a.name.cmp(&b.name))
            });

            by_type.insert(e.type_en.to_lowercase(), idx);
            if !e.type_ru.is_empty() {
                by_type.insert(e.type_ru.to_lowercase(), idx);
            }
        }

        MemberIndex { by_type, entries }
    }

    /// Члены типа по русскому или английскому имени.
    pub fn members_of(&self, type_name: &str) -> Option<&TypeMembers> {
        let idx = *self.by_type.get(&type_name.trim().to_lowercase())?;
        let e = &self.entries[idx];
        if e.members.is_empty() {
            None
        } else {
            Some(e)
        }
    }

    /// Типы, чьё имя похоже на спрошенное.
    ///
    /// Дешёвая проверка на вхождение подстроки: перебирать тысячу имён незачем, а опечатку
    /// в одну букву она не ловит — и не обещает.
    pub fn near(&self, type_name: &str) -> Vec<String> {
        let needle = type_name.trim().to_lowercase();
        if needle.chars().count() < 4 {
            return Vec::new();
        }
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<String> = Vec::new();
        // Обход ключей — ОТСОРТИРОВАННЫЙ, и это единственное намеренное расхождение с Go.
        //
        // Там перечень собирается из обхода map и обрезается восемью именами, поэтому
        // выбор зависит от порядка обхода: замер 04.09.2026 дал 21 РАЗНЫЙ ответ на «Табл»
        // за 25 прогонов. Итоговая сортировка этого не лечит — она упорядочивает уже
        // выбранную восьмёрку, а выбиралась она случайно.
        //
        // Для подсказки «похожие типы» это прямой вред: спрашивающий видит разные варианты
        // на один и тот же вопрос и не может понять, чем они отличаются. Воспроизводить
        // случайность незачем — сортируем ключи до обрезки.
        let mut keys: Vec<&String> = self.by_type.keys().collect();
        keys.sort();
        for key in keys {
            if !key.contains(&needle) && !needle.contains(key.as_str()) {
                continue;
            }
            let e = &self.entries[self.by_type[key]];
            let name = if e.type_ru.is_empty() {
                &e.type_en
            } else {
                &e.type_ru
            };
            if !seen.insert(name.clone()) {
                continue;
            }
            out.push(name.clone());
            if out.len() >= 8 {
                break;
            }
        }
        out.sort();
        out
    }

    /// Сколько РАЗНЫХ типов в индексе.
    ///
    /// Ключей вдвое больше: русское и английское имя ведут на одну запись, и счёт по
    /// ключам завысил бы число вдвое.
    pub fn type_count(&self) -> usize {
        let uniq: std::collections::HashSet<usize> = self.by_type.values().copied().collect();
        uniq.len()
    }
}

/// Ответ на вопрос «что умеет этот тип».
pub fn members_answer(e: &TypeMembers) -> String {
    let mut name = e.type_ru.clone();
    if name.is_empty() {
        name = e.type_en.clone();
    } else if !e.type_en.is_empty() {
        name = format!("{name} ({})", e.type_en);
    }

    let mut b = format!("{name} — членов: {}\n", e.members.len());
    let mut cur: Option<MemberKind> = None;
    for m in &e.members {
        if m.kind != cur {
            cur = m.kind;
            b.push_str(&format!(
                "\n== {} ==\n",
                m.kind.map(|k| k.title()).unwrap_or("")
            ));
        }
        let mut line = format!("  {}", m.name);
        if !m.name_en.is_empty() && m.name_en != m.name {
            line.push_str(&format!(" ({})", m.name_en));
        }
        if !m.returns.is_empty() {
            line.push_str(&format!(" → {}", m.returns));
        }
        b.push_str(&line);
        b.push('\n');
    }
    b.push_str(&format!(
        "\nПодробности по любому члену — тот же инструмент без members: запрос вида «{}».\n",
        first_member_example(e)
    ));
    b
}

/// Как спросить подробности: пример строится из реального члена этого типа, а не из
/// выдуманного, иначе подсказка ведёт в отказ.
fn first_member_example(e: &TypeMembers) -> String {
    let Some(first) = e.members.first() else {
        return String::new();
    };
    let name = if e.type_ru.is_empty() {
        &e.type_en
    } else {
        &e.type_ru
    };
    format!("{name}.{}", first.name)
}
