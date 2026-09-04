//! Индекс справки платформы: оглавление синтакс-помощника + ранжирование страниц.
//!
//! Зачем. Прежний поиск шёл тремя шагами (точное имя объекта, LIKE, полнотекст) и брал
//! `hits[0]` — первую строку, что вернул SQLite без ORDER BY. По журналу живых вызовов это
//! давало 52% мимо: половина вопросов получала либо отказ при существующей странице, либо
//! чужую страницу молча. Причины были механические, и лечатся они тоже механически.
//!
//! 1. КЛЮЧИ СТРАНИЦ АНГЛИЙСКИЕ, СПРАШИВАЮТ ПО-РУССКИ. Страница оператора ПОДОБНО называется
//!    LIKE, ЕСТЬNULL — ISNULL. Словарь для перевода не нужно составлять руками: он лежит
//!    в самой справке. Половина базы — это оглавление синтакс-помощника в формате
//!    «bracket file», где у каждой темы записаны русское имя, английское имя и путь
//!    страницы. Раньше эти записи только шумели в полнотексте.
//!
//! 2. СПРАВКА НАРЕЗАНА ПО ТЕМАМ, А НЕ ПО ОПЕРАТОРАМ. Страницы «УПОРЯДОЧИТЬ ПО» в природе
//!    нет — есть «Упорядочивание результатов запроса». Поэтому ищем не только точное имя,
//!    но и тему, чьё ПЕРВОЕ слово совпало с термином, и заголовок, покрывающий все слова
//!    вопроса.
//!
//! 3. ДВА РОДА ВОПРОСОВ, ОДИН ИНДЕКС. Операторы языка живут в shquery (130 страниц),
//!    виртуальные таблицы регистров — в shcntx (25 604). Сужать индекс по категории нельзя
//!    (тогда ОстаткиИОбороты получает отказ), поэтому вопрос классифицируется, а категория
//!    входит в вес: оператор тянет к shquery, таблица — к разделу /tables/.
//!
//! 4. `hits[0]` ЗАМЕНЁН ВЕСОМ. У каждого кандидата считается вес по способу находки, и ниже
//!    порога ответ не отдаётся вовсе — вместо уверенной чужой страницы список кандидатов.
//!
//! Оглавление разбирается один раз за жизнь процесса (~1 c) и держится в памяти. Формат
//! базы при этом не меняется: пакет работает с уже собранной справкой, ничего пересобирать
//! на чужой машине не нужно.

use std::collections::HashMap;

use rusqlite::Connection;

/// Страница справки платформы.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpPage {
    pub title: String,
    pub object: String,
    pub path: String,
}

/// Страница и имя темы, под которым она попала в индекс первого слова: без имени нельзя
/// понять, насколько тема близка к спрошенному термину.
#[derive(Debug, Clone)]
struct TocEntry {
    page: HelpPage,
    toc_name: String,
}

/// Страницы и разобранное оглавление.
pub struct HelpIndex {
    /// Имя страницы (нижний регистр) → страница.
    pages: HashMap<String, HelpPage>,
    /// Путь → имя страницы.
    by_path: HashMap<String, String>,
    /// Нормализованное имя темы → страницы.
    toc: HashMap<String, Vec<HelpPage>>,
    /// Первое слово темы → страницы (с именем темы).
    toc_head: HashMap<String, Vec<TocEntry>>,
    /// Ключевое слово → его вариант на другом языке.
    alias: HashMap<String, String>,
    /// Ключевое слово → тема, в тексте которой оно описано.
    in_text: HashMap<String, HelpPage>,
}

/// Записи оглавления: язык запросов и объекты платформы.
const TOC_SOURCES: &[&str] = &["shquery_ru.hbk#42", "shcntx_ru.hbk#52159/0"];

/// SQL-условие «страница относится к запросам».
///
/// Инструмент `syntax` отвечает про ЯЗЫК ЗАПРОСОВ и ТАБЛИЦЫ ПЛАТФОРМЫ, а справка вендора
/// содержит и весь встроенный язык: на 130 страниц shquery приходится 25 604 страницы
/// shcntx (объектная модель). Полнотекстовый поиск по такому составу отвечает про что
/// угодно: замер 31.08.2026 показал, как вопрос «GROUP BY функция выражение» вернул
/// страницу про ВыражениеXPath, после чего модель ушла сочинять запрос дальше.
///
/// Поэтому в поиске видны только shquery (язык запросов целиком) и раздел `/tables/` книги
/// shcntx — виртуальные таблицы регистров с их полями и параметрами. Прочая объектная
/// модель не удалена из базы (она общая, её читает ещё и скилл `kb-1c`) — она просто не
/// участвует в поиске этого инструмента. Вопрос «что умеет тип» идёт мимо этого условия:
/// у него свой путь, `members=true`.
pub const QUERY_SCOPE: &str = "config='platform' AND title NOT LIKE '{%' \
     AND (category='shquery' OR path LIKE '%/tables/%')";

impl HelpIndex {
    /// Строит индекс по базе справки.
    pub fn build(db: &Connection) -> Self {
        let mut ix = HelpIndex {
            pages: HashMap::new(),
            by_path: HashMap::new(),
            toc: HashMap::new(),
            toc_head: HashMap::new(),
            alias: HashMap::new(),
            in_text: HashMap::new(),
        };

        let sql = format!("SELECT title, object, path FROM pages WHERE {QUERY_SCOPE}");
        if let Ok(mut stmt) = db.prepare(&sql) {
            let rows = stmt.query_map([], |r| {
                Ok(HelpPage {
                    title: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    object: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    path: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                })
            });
            if let Ok(rows) = rows {
                for p in rows.flatten() {
                    let key = p.object.to_lowercase();
                    ix.by_path.insert(p.path.clone(), p.object.clone());
                    ix.pages.entry(key).or_insert(p);
                }
            }
        }

        for src in TOC_SOURCES {
            let text: String =
                match db.query_row("SELECT text FROM pages WHERE path = ?", [src], |r| r.get(0)) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
            let hbk = match src.find('#') {
                Some(i) => &src[..i],
                None => src,
            };
            for node in toc_nodes(&text) {
                if node.path.is_empty() {
                    continue;
                }
                let Some(page) = ix.resolve(hbk, &node.path) else {
                    continue;
                };
                for name in [&node.ru, &node.en] {
                    if name.is_empty() {
                        continue;
                    }
                    let k = norm_key(name);
                    if k.is_empty() {
                        continue;
                    }
                    ix.toc.entry(k.clone()).or_default().push(page.clone());
                    let head = first_word(name);
                    let hk = norm_key(&head);
                    if !hk.is_empty() && hk != k {
                        ix.toc_head.entry(hk).or_default().push(TocEntry {
                            page: page.clone(),
                            toc_name: name.clone(),
                        });
                    }
                }
            }
        }

        ix.load_keyword_aliases(db);
        ix.load_keywords_in_text(db);
        ix
    }

    /// Двуязычие языка запросов, взятое у вендора, а не составленное нами.
    ///
    /// В оглавлении shquery двуязычных имён нет ВОВСЕ (0 пар из 197) — в отличие от shcntx,
    /// где их 24 572. Поэтому английская форма темы языка запросов не находилась ничем:
    /// ORDER BY, TOTALS, SELECT уходили в отказ или в чужую страницу объектной модели, хотя
    /// русская форма отвечала верно. Таблица соответствий лежит в самой справке — страница
    /// «Двуязычное представление ключевых слов»; её и разбираем, вместо того чтобы
    /// выписывать пары руками и ошибаться в них.
    ///
    /// Словарь двусторонний: спрашивают и по-английски, и по-русски.
    fn load_keyword_aliases(&mut self, db: &Connection) {
        let text: String = match db.query_row(
            "SELECT text FROM pages WHERE config='platform' AND object='present'",
            [],
            |r| r.get(0),
        ) {
            Ok(t) => t,
            Err(_) => return,
        };
        let lines: Vec<&str> = text
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();

        // Таблица идёт парами строк «русское / английское». Шапка отсекается сама: пара
        // засчитывается, только когда первая строка целиком кириллическая заглавная,
        // а вторая — латинская заглавная. Заголовок «Английское / написание» этому
        // не отвечает.
        let mut i = 0;
        while i + 1 < lines.len() {
            let (ru, en) = (lines[i], lines[i + 1]);
            if !is_upper_ru(ru) || !is_upper_en(en) {
                i += 1;
                continue;
            }
            self.add_alias(ru, en);
            // Составные конструкции записаны через многоточие: «ИТОГИ … ПО» / «TOTALS … BY».
            // Спрашивают их обычно головным словом («ИТОГИ», «TOTALS»), поэтому связь
            // заводится и по голове — иначе английская форма самой частой конструкции
            // не находится.
            let (hru, hen) = (head_word(ru), head_word(en));
            if hru != ru || hen != en {
                self.add_alias(&hru, &hen);
            }
            i += 2; // пара разобрана целиком
        }
    }

    /// Слова, у которых СВОЕЙ страницы в справке нет.
    ///
    /// Замер двуязычия показал: из 90 ключевых слов языка запросов 10 не находятся ничем —
    /// ТОГДА, КОГДА, ИЛИ, ИНАЧЕ, СПЕЦСИМВОЛ, НАБОРАМ, ОБЩИЕ, УНИКАЛЬНО и подобные. Это не
    /// дефект поиска: отдельной страницы у них не существует в природе, они описаны ВНУТРИ
    /// тем — «ТОГДА» живёт на странице «Операция выбора в языке запросов», «СПЕЦСИМВОЛ» —
    /// на странице оператора ПОДОБНО. Верный ответ на такой вопрос — страница темы,
    /// а не отказ.
    ///
    /// Связь выводится из текста, а не из оглавления, поэтому вес у неё ниже вендорского
    /// соответствия: это догадка по частоте, пусть и надёжная. Берётся страница языка
    /// запросов с наибольшим числом упоминаний слова; страница самой таблицы двуязычия
    /// исключается — она упоминает КАЖДОЕ ключевое слово ровно один раз и иначе выигрывала
    /// бы у настоящих тем.
    fn load_keywords_in_text(&mut self, db: &Connection) {
        if self.alias.is_empty() {
            return;
        }
        let Ok(mut stmt) = db.prepare(
            "SELECT title, object, path, text FROM pages
              WHERE config='platform' AND category='shquery'
                AND object <> 'present' AND title NOT LIKE '{%'",
        ) else {
            return;
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                HelpPage {
                    title: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    object: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    path: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                },
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ))
        });
        let Ok(rows) = rows else { return };

        // Слово берётся в этот индекс, ТОЛЬКО если своей темы у него нет. Иначе выводы по
        // частоте начинают спорить с вендорским оглавлением и проигрывают ему по смыслу:
        // «ДАТА» — тема в оглавлении, а по числу упоминаний она чаще всего попадает на
        // РАЗНОСТЬДАТ. Проверено замером: без этого условия двуязычие падает с 99% до 94%.
        let orphan = |key: &str| -> bool {
            !self.toc.contains_key(key)
                && !self.toc_head.contains_key(key)
                && !self.pages.contains_key(&key.to_lowercase())
        };
        let keys: Vec<String> = self
            .alias
            .keys()
            .filter(|k| k.chars().count() >= 3 && orphan(k))
            .cloned()
            .collect();

        let mut found: HashMap<String, (HelpPage, usize)> = HashMap::new();
        for (p, text) in rows.flatten() {
            let upper = text.to_uppercase();
            for key in &keys {
                // Ключ уже нормализован — восстанавливать исходное написание не нужно:
                // в тексте справки ключевые слова набраны заглавными.
                let n = count_word(&upper, key);
                if n == 0 {
                    continue;
                }
                match found.get(key) {
                    Some((cur, cnt)) if n < *cnt => {}
                    Some((cur, cnt)) if n == *cnt && p.object >= cur.object => {}
                    _ => {
                        found.insert(key.clone(), (p.clone(), n));
                    }
                }
            }
        }
        for (key, (page, _)) in found {
            self.in_text.insert(key, page);
        }
    }

    /// Заводит двустороннюю связь между вариантами написания. Первое соответствие
    /// побеждает: таблица вендора не содержит противоречий, а страховка от них дешевле
    /// разбора.
    fn add_alias(&mut self, a: &str, b: &str) {
        let (ka, kb) = (norm_key(a), norm_key(b));
        if ka.is_empty() || kb.is_empty() || ka == kb {
            return;
        }
        self.alias.entry(kb).or_insert_with(|| a.to_string());
        self.alias.entry(ka).or_insert_with(|| b.to_string());
    }

    /// Дописывает к вариантам вопроса их иноязычные соответствия: они идут последними,
    /// потому что перевод дальше от того, что спросили, чем сама формулировка.
    fn translate(&self, variants: Vec<String>) -> Vec<String> {
        let mut seen: std::collections::HashSet<String> =
            variants.iter().map(|v| norm_key(v)).collect();
        let mut out = variants.clone();
        for v in &variants {
            if let Some(alt) = self.alias.get(&norm_key(v)) {
                let k = norm_key(alt);
                if seen.insert(k) {
                    out.push(alt.clone());
                }
            }
        }
        out
    }

    /// Переводит путь из оглавления в страницу базы.
    fn resolve(&self, hbk: &str, path: &str) -> Option<HelpPage> {
        let key = path.strip_prefix('/').unwrap_or(path);
        for cand in [format!("{hbk}#{key}"), format!("{hbk}#{key}.html")] {
            if let Some(obj) = self.by_path.get(&cand) {
                return self.pages.get(&obj.to_lowercase()).cloned();
            }
        }
        let leaf = key.strip_suffix(".html").unwrap_or(key);
        let leaf = match leaf.rfind('/') {
            Some(i) => &leaf[i + 1..],
            None => leaf,
        };
        self.pages.get(&leaf.to_lowercase()).cloned()
    }

    /// Конструкции из текста запроса, у которых есть своя страница справки: то, что стоит
    /// спросить по одной. Ключевые слова и функции пишутся заглавными — по ним и отбор;
    /// имена таблиц и полей заглавными не бывают и сюда не попадают.
    pub fn constructions_in(&self, text: &str) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for tok in word_tokens(text) {
            if tok.chars().count() < 3 || !(is_upper_ru(&tok) || is_upper_en(&tok)) {
                continue;
            }
            let key = norm_key(&tok);
            if seen.contains(&key) {
                continue;
            }
            let known = self.toc.contains_key(&key)
                || self.toc_head.contains_key(&key)
                || self.pages.contains_key(&tok.to_lowercase())
                || self.in_text.contains_key(&key);
            if known {
                seen.insert(key);
                out.push(tok);
            }
        }
        out
    }
}

/// Сколько раз слово встречается как ОТДЕЛЬНОЕ слово. Без границ «ВСЕ» нашлось бы внутри
/// «ВСЕГО», а «ИЛИ» — внутри любого «...ИЛИ».
fn count_word(haystack: &str, word: &str) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(i) = haystack[from..].find(word) {
        let i = i + from;
        let end = i + word.len();
        let left_ok = i == 0 || !haystack[..i].chars().next_back().is_some_and(is_word_char);
        let right_ok =
            end >= haystack.len() || !haystack[end..].chars().next().is_some_and(is_word_char);
        if left_ok && right_ok {
            n += 1;
        }
        from = end;
    }
    n
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Часть конструкции до многоточия: «ИТОГИ … ПО» → «ИТОГИ».
fn head_word(s: &str) -> String {
    match s.find('…') {
        Some(i) => s[..i].trim().to_string(),
        None => s.to_string(),
    }
}

/// Строка состоит только из заглавной кириллицы (и разделителей).
fn is_upper_ru(s: &str) -> bool {
    is_upper_only(s, 'А', 'Я')
}

/// То же для латиницы.
fn is_upper_en(s: &str) -> bool {
    is_upper_only(s, 'A', 'Z')
}

fn is_upper_only(s: &str, lo: char, hi: char) -> bool {
    let mut letters = 0;
    for r in s.chars() {
        if (r >= lo && r <= hi) || (r == 'Ё' && lo == 'А') {
            letters += 1;
        } else if matches!(r, ' ' | '.' | '[' | ']' | '…') {
            // Многоточие — часть записи составных конструкций: «ИТОГИ … ПО» / «TOTALS … BY».
            // Без него пара не опознаётся, и ИТОГИ теряют английскую форму.
        } else {
            return false;
        }
    }
    letters > 0
}

// ── разбор bracket-формата ───────────────────────────────────────────────────────────────

/// Тема оглавления: имена и путь страницы.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TocNode {
    pub ru: String,
    pub en: String,
    pub path: String,
}

/// Либо строка, либо список.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BracketValue {
    Str(String),
    List(Vec<BracketValue>),
}

impl BracketValue {
    fn as_str(&self) -> &str {
        match self {
            BracketValue::Str(s) => s,
            BracketValue::List(_) => "",
        }
    }
    fn is_list(&self) -> bool {
        matches!(self, BracketValue::List(_))
    }
    fn list(&self) -> &[BracketValue] {
        match self {
            BracketValue::List(v) => v,
            BracketValue::Str(_) => &[],
        }
    }
}

/// Разбирает `{a,b,{c}}` в дерево. Строки в кавычках, удвоенная кавычка внутри —
/// экранирование. Формат вендорский, документирован в `1c-syntax/bsl-help-toc-parser`.
fn parse_bracket(s: &str) -> BracketValue {
    let r: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    parse_value(&r, &mut i)
}

fn parse_value(r: &[char], i: &mut usize) -> BracketValue {
    skip_ws(r, i);
    if *i >= r.len() {
        return BracketValue::Str(String::new());
    }
    match r[*i] {
        '{' => {
            *i += 1;
            let mut out = Vec::new();
            while *i < r.len() {
                skip_ws(r, i);
                if *i >= r.len() {
                    break;
                }
                if r[*i] == '}' {
                    *i += 1;
                    break;
                }
                if r[*i] == ',' {
                    *i += 1;
                    continue;
                }
                out.push(parse_value(r, i));
            }
            BracketValue::List(out)
        }
        '"' => {
            *i += 1;
            let mut b = String::new();
            while *i < r.len() {
                if r[*i] == '"' {
                    if *i + 1 < r.len() && r[*i + 1] == '"' {
                        b.push('"');
                        *i += 2;
                        continue;
                    }
                    *i += 1;
                    break;
                }
                b.push(r[*i]);
                *i += 1;
            }
            BracketValue::Str(b)
        }
        _ => {
            let start = *i;
            while *i < r.len() && !matches!(r[*i], ',' | '{' | '}' | '\r' | '\n') {
                *i += 1;
            }
            let s: String = r[start..*i].iter().collect();
            BracketValue::Str(s.trim().to_string())
        }
    }
}

fn skip_ws(r: &[char], i: &mut usize) {
    while *i < r.len() && matches!(r[*i], ' ' | '\t' | '\r' | '\n') {
        *i += 1;
    }
}

/// Достаёт темы из записи оглавления.
///
/// Узел: `[id, parent, N, дети…, [1,1,<имена>,путь]]`, где `<имена>` — либо
/// `[1,1,["#","Тема"]]`, либо `[1,2,["ru","ПОДОБНО"],["en","LIKE"]]`.
pub fn toc_nodes(text: &str) -> Vec<TocNode> {
    let root = parse_bracket(text);
    if !root.is_list() || root.list().len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for rec in &root.list()[1..] {
        if !rec.is_list() || rec.list().len() < 4 {
            continue;
        }
        let tail = rec.list().last().unwrap();
        if !tail.is_list() || tail.list().len() < 4 {
            continue;
        }
        let (titles, path) = (&tail.list()[2], &tail.list()[3]);
        if !titles.is_list() || path.is_list() {
            continue;
        }
        let mut node = TocNode {
            path: path.as_str().to_string(),
            ..Default::default()
        };
        for item in titles.list().iter().skip(2) {
            if !item.is_list() || item.list().len() < 2 {
                continue;
            }
            let (lang, val) = (item.list()[0].as_str(), item.list()[1].as_str());
            match lang {
                "#" | "ru" => node.ru = val.to_string(),
                "en" => node.en = val.to_string(),
                _ => {}
            }
        }
        if !node.ru.is_empty() || !node.en.is_empty() {
            out.push(node);
        }
    }
    out
}

// ── нормализация вопроса ─────────────────────────────────────────────────────────────────

/// Связки, которые модель добавляет к вопросу и которые ничего не выбирают.
///
/// Слова вроде «бухгалтерия» сюда НЕ входят намеренно: они и отличают регистр бухгалтерии
/// от регистра накопления.
const NOISE_WORDS: &[&str] = &[
    "виртуальная",
    "виртуальные",
    "виртуальной",
    "таблица",
    "таблицы",
    "таблиц",
    "платформы",
    "запрос",
    "запроса",
    "язык",
    "языка",
    "функция",
    "функции",
    "оператор",
    "операторы",
    "ключевое",
    "слово",
    "справка",
];

fn is_noise(w: &str) -> bool {
    let low = w.to_lowercase();
    NOISE_WORDS.contains(&low.as_str())
}

/// Ключ сравнения: только буквы и цифры, верхний регистр. Пунктуация выброшена не для
/// красоты: тема называется «ИТОГИ ... ПО», и многоточие — оформление, а не имя.
pub fn norm_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// Грубая основа: «бухгалтерия» и «бухгалтерии» должны считаться одним словом.
fn stem_word(w: &str) -> String {
    w.to_uppercase().chars().take(6).collect()
}

fn first_word(s: &str) -> String {
    s.trim()
        .split([' ', '.'])
        .find(|p| !p.is_empty())
        .unwrap_or("")
        .to_string()
}

/// `ЛитералДата` → `[Литерал, Дата]`. Склейка слов — обычная форма вопроса модели.
fn split_camel(word: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_upper = false;
    for r in word.chars() {
        let is_upper = r.is_uppercase();
        if is_upper && !prev_upper && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(r);
        prev_upper = is_upper;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.retain(|p| p.chars().count() > 2);
    out
}

/// Убирает скобочные и угловые вставки: «Ссылка (Ref)» → «Ссылка».
fn strip_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut depth_paren = 0i32;
    let mut depth_angle = 0i32;
    for c in s.chars() {
        match c {
            '(' => {
                depth_paren += 1;
                out.push(' ');
            }
            ')' => {
                depth_paren = (depth_paren - 1).max(0);
                out.push(' ');
            }
            '<' => {
                depth_angle += 1;
                out.push(' ');
            }
            '>' => {
                depth_angle = (depth_angle - 1).max(0);
                out.push(' ');
            }
            _ if depth_paren > 0 || depth_angle > 0 => {}
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

/// Что искать: тема, её сегменты и очищенные формы, в порядке убывания точности.
fn query_variants(query: &str) -> Vec<String> {
    let q = query.trim().to_string();
    let mut out = vec![q.clone()];
    let cut = strip_brackets(&q);
    if !cut.is_empty() && cut != q {
        out.push(cut.clone());
    }
    if cut.contains('.') {
        let segs: Vec<&str> = cut.split('.').filter(|s| !s.trim().is_empty()).collect();
        if !segs.is_empty() {
            out.push(segs[segs.len() - 1].to_string());
            out.push(segs.join(" "));
        }
    }
    let words: Vec<&str> = cut.split(|c: char| c.is_whitespace() || c == ',').collect();
    let keep: Vec<&str> = words
        .iter()
        .filter(|w| !w.is_empty() && !is_noise(w))
        .copied()
        .collect();
    if !keep.is_empty() && keep.len() != words.len() {
        out.push(keep.join(" "));
    }
    if keep.len() > 1 {
        out.extend(keep.iter().map(|s| s.to_string()));
    }
    let mut seen = std::collections::HashSet::new();
    let mut res = Vec::new();
    for v in out {
        let k = norm_key(&v);
        if !k.is_empty() && seen.insert(k) {
            res.push(v);
        }
    }
    res
}

/// Значимые слова вопроса для поиска по заголовку через И.
fn query_keywords(query: &str) -> Vec<String> {
    // Скобки убираются вместе с содержимым, а угловые скобки и запятые — только как знаки.
    let cut: String = strip_parens(query)
        .chars()
        .map(|c| match c {
            '<' | '>' | '(' | ')' | ',' => ' ',
            other => other,
        })
        .collect();
    let mut words = Vec::new();
    for w in cut.split_whitespace() {
        if is_noise(w) {
            continue;
        }
        for seg in w.split('.') {
            if seg.chars().count() <= 2 {
                continue;
            }
            let parts = split_camel(seg);
            if parts.len() > 1 {
                // Склейку целиком не ищем: «ЛитералДата» не встречается нигде,
                // а «Литерал» + «Дата» находят страницу «Литерал типа ДАТА».
                words.extend(parts);
            } else {
                words.push(seg.to_string());
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut res = Vec::new();
    for w in words {
        if seen.insert(w.to_uppercase()) {
            res.push(w);
        }
    }
    res
}

/// Убирает круглые скобки вместе с содержимым.
fn strip_parens(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                out.push(' ');
            }
            ')' => {
                depth = (depth - 1).max(0);
                out.push(' ');
            }
            _ if depth > 0 => {}
            _ => out.push(c),
        }
    }
    out
}

/// Куда тянуть ответ: оператор языка запросов или таблица платформы.
fn question_kind(query: &str) -> &'static str {
    let q = query.trim();
    let low = q.to_lowercase();
    for pref in [
        "регистр",
        "register",
        "справочник",
        "catalog",
        "документ",
        "document",
    ] {
        if let Some(i) = low.find(pref) {
            if low[i..].contains('.') {
                return "table";
            }
        }
    }
    for w in ["таблиц", "регистр", "register", "срез", "остатк", "оборот"]
    {
        if low.contains(w) {
            return "table";
        }
    }
    let mut has_letter = false;
    let mut all_upper = true;
    for r in q.chars() {
        if r.is_alphabetic() {
            has_letter = true;
            if r.is_lowercase() {
                all_upper = false;
            }
        }
    }
    if has_letter && all_upper {
        return "query";
    }
    "any"
}

// ── ранжирование ─────────────────────────────────────────────────────────────────────────

/// Ниже этого веса ответ не отдаётся. Лучше список кандидатов, чем уверенная чужая
/// страница: отказ виден, подмена — нет.
const SEARCH_THRESHOLD: f64 = 45.0;

/// Слова языка запросов, по которым в поле `query` узнаётся ТЕКСТ ЗАПРОСА, а не имя
/// конструкции. Текст запроса нечёткий поиск разбирает на слова и отвечает страницей по
/// самому общему из них («выражение» → ВыражениеXPath) — убедительно и мимо.
const QUERY_TEXT_WORDS: &[&str] = &[
    "ВЫБРАТЬ",
    "SELECT",
    "ИЗ",
    "FROM",
    "ГДЕ",
    "WHERE",
    "СГРУППИРОВАТЬ",
    "GROUP",
    "УПОРЯДОЧИТЬ",
    "ORDER",
    "СОЕДИНЕНИЕ",
    "JOIN",
    "ПОМЕСТИТЬ",
    "INTO",
    "ОБЪЕДИНИТЬ",
    "UNION",
    "ИТОГИ",
    "TOTALS",
];

/// Слова текста: последовательности букв, цифр и подчёркиваний.
fn word_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            // Первым символом цифра быть не должна — как в исходном регулярном выражении.
            if cur.is_empty() && c.is_numeric() {
                continue;
            }
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Два и больше ключевых слова языка запросов, либо одно плюс параметр `&Имя`.
pub fn looks_like_query_text(q: &str) -> bool {
    let mut n = 0;
    for tok in word_tokens(q) {
        let up = tok.to_uppercase();
        if QUERY_TEXT_WORDS.contains(&up.as_str()) {
            n += 1;
        }
    }
    n >= 2 || (n >= 1 && q.contains('&'))
}

/// Кандидат и его вес.
struct ScoredPage {
    page: HelpPage,
    weight: f64,
}

/// Текст и версия страницы ПО ПУТИ.
///
/// Раньше тело бралось `WHERE object = ? LIMIT 1`: под одним именем объекта в базе лежит
/// и настоящая страница, и служебная в скобках, и LIMIT 1 без ORDER BY отдавал любую из
/// них. Прогон 27.08.2026: на вопрос про «Первые» модель получила `{1, {2, {"",1,0,"",""}`
/// вместо текста.
pub fn page_body(db: &Connection, p: &HelpPage) -> (String, String) {
    let mut body = String::new();
    let mut version = String::new();
    if let Ok((b, v)) = db.query_row(
        "SELECT text, config_version FROM pages WHERE path = ? LIMIT 1",
        [&p.path],
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        },
    ) {
        body = b;
        version = v;
    }
    if body.trim().is_empty() || body.trim().starts_with('{') {
        // Путь не нашёлся или сам оказался скобками — берём по объекту, но только страницу
        // с человеческим текстом.
        if let Ok((b, v)) = db.query_row(
            "SELECT text, config_version FROM pages
              WHERE object = ? AND config='platform' AND title NOT LIKE '{%' AND text NOT LIKE '{%'
              ORDER BY length(title) LIMIT 1",
            [&p.object],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            },
        ) {
            body = b;
            version = v;
        }
    }
    (body, version)
}

/// Ищет страницу справки платформы.
///
/// Возвращает лучшую (или `None`, если ни один кандидат не дотянул до порога) и список
/// кандидатов для подсказки.
pub fn search_help(
    db: &Connection,
    ix: &HelpIndex,
    query: &str,
) -> (Option<HelpPage>, Vec<HelpPage>) {
    let kind = question_kind(query);
    let mut scored: HashMap<String, ScoredPage> = HashMap::new();

    fn add(scored: &mut HashMap<String, ScoredPage>, p: &HelpPage, w: f64) {
        if p.object.is_empty() {
            return;
        }
        match scored.get(&p.object) {
            Some(cur) if cur.weight >= w => {}
            _ => {
                scored.insert(
                    p.object.clone(),
                    ScoredPage {
                        page: p.clone(),
                        weight: w,
                    },
                );
            }
        }
    }

    let variants = ix.translate(query_variants(query));
    for (depth, v) in variants.iter().enumerate() {
        let fade = 1.0 - 0.12 * depth as f64; // чем дальше от формулировки, тем слабее
        let key = norm_key(v);
        if let Some(pages) = ix.toc.get(&key) {
            for p in pages {
                add(&mut scored, p, 100.0 * fade); // 1. имя темы в оглавлении
            }
        }
        if let Some(entries) = ix.toc_head.get(&key) {
            for hp in entries {
                // 1а. термин — первое слово темы.
                //
                // Доля важна: «ИТОГИ» — первое слово и у темы «ИТОГИ … ПО» (query_totals),
                // и у «ИТОГИ … ПО ОБЩИЕ» (overall_totals). Ближе к голому термину та тема,
                // что короче, иначе выбор между ними — случайность.
                // Затухание мягкое (0.6…1.0), а не пропорциональное: соответствие из
                // оглавления — вендорское, и оно точнее случайного совпадения заголовка.
                // Пропорция здесь только разводит темы с одинаковым первым словом.
                let share = key.chars().count() as f64
                    / norm_key(&hp.toc_name).chars().count().max(1) as f64;
                add(&mut scored, &hp.page, 92.0 * (0.6 + 0.4 * share) * fade);
            }
        }
        // 1б. имя темы начинается с термина.
        if key.chars().count() >= 4 {
            for (name, pages) in &ix.toc {
                if name != &key && name.starts_with(&key) {
                    // Вес по доле совпадения: «ИТОГИ» в «ИТОГИ … ПО» — почти вся тема,
                    // «Упорядочивание» в «Упорядочивание по иерархии» — половина.
                    let share = key.chars().count() as f64 / name.chars().count() as f64;
                    for p in pages {
                        add(&mut scored, p, 88.0 * share * fade);
                    }
                }
            }
        }
        if let Some(p) = ix.pages.get(&v.to_lowercase()) {
            add(&mut scored, &p.clone(), 90.0 * fade); // 2. имя страницы
        }
        // 2а. слово описано ВНУТРИ темы, своей страницы у него нет. Вес ниже вендорского
        // соответствия из оглавления: связь выведена из частоты упоминаний, а не задана.
        if let Some(p) = ix.in_text.get(&key) {
            add(&mut scored, &p.clone(), 80.0 * fade);
        }
        // 3. заголовок. ORDER BY обязателен: без него LIMIT режет наугад — та же болезнь,
        // что hits[0]. Короткий заголовок точнее по смыслу.
        let sql = format!(
            "SELECT title, object, path FROM pages
              WHERE {QUERY_SCOPE} AND title LIKE ?
              ORDER BY length(title) LIMIT 60"
        );
        if let Ok(mut stmt) = db.prepare(&sql) {
            let pattern = format!("%{v}%");
            if let Ok(rows) = stmt.query_map([&pattern], |r| {
                Ok(HelpPage {
                    title: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    object: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    path: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                })
            }) {
                for p in rows.flatten() {
                    let w = if p.title.to_uppercase().starts_with(&v.to_uppercase()) {
                        70.0
                    } else {
                        55.0
                    };
                    add(&mut scored, &p, w * fade);
                }
            }
        }
    }

    // Полнотекстовый проход. Через FTS, а не LIKE: UPPER() в SQLite кириллицу не трогает,
    // поэтому UPPER(title) LIKE '%ЛИТЕРАЛ%' не находит «Литерал типа ДАТА». Токенизатор FTS
    // регистр учитывает верно, а совпадение именно в ЗАГОЛОВКЕ проверяем в коде.
    let fts_pass = |scored: &mut HashMap<String, ScoredPage>, words: &[String], weight: f64| {
        if words.is_empty() {
            return;
        }
        let quoted: Vec<String> = words
            .iter()
            .map(|w| format!("\"{}\"", w.replace('"', "")))
            .collect();
        let Ok(mut stmt) = db.prepare(
            "SELECT p.title, p.object, p.path FROM pages_fts f
              JOIN pages p ON p.id = f.rowid
              WHERE pages_fts MATCH ?
                AND p.config='platform' AND p.title NOT LIKE '{%'
                AND (p.category='shquery' OR p.path LIKE '%/tables/%')
              ORDER BY length(p.title) LIMIT 200",
        ) else {
            return;
        };
        let Ok(rows) = stmt.query_map([quoted.join(" AND ")], |r| {
            Ok(HelpPage {
                title: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                object: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                path: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            })
        }) else {
            return;
        };
        for p in rows.flatten() {
            let up = p.title.to_uppercase();
            if words.iter().all(|w| up.contains(&w.to_uppercase())) {
                add(scored, &p, weight);
            }
        }
    };

    let words = query_keywords(query);
    fts_pass(&mut scored, &words, 66.0);

    // Перевод ищется ОТДЕЛЬНЫМ проходом, а не подмешивается к словам вопроса. Слова внутри
    // запроса FTS соединяются через AND, поэтому «ВНЕШНЕЕ» + «OUTER» в одном наборе требует
    // страницу, где есть оба слова сразу, — то есть сужает поиск там, где надо расширить.
    // Проверено на себе: подмешивание увело «ВНЕШНЕЕ» с темы соединений на свойство
    // хранилища двоичных данных. Вес ниже: перевод дальше от того, что спросили.
    for v in variants.iter().skip(1) {
        let tw = query_keywords(v);
        if !tw.is_empty() {
            fts_pass(&mut scored, &tw, 60.0);
        }
    }

    let mut out: Vec<ScoredPage> = Vec::with_capacity(scored.len());
    for sp in scored.into_values() {
        let mut w = sp.weight;
        let p = sp.page;
        if kind == "query" {
            // Оператор языка живёт в shquery. Совпадение имени в shcntx — почти всегда
            // чужая тема с тем же словом: УПОРЯДОЧИТЬ там метод СКД, ПОРЯДОК — свойство
            // динамического списка. Поэтому не бонус своим, а штраф чужим.
            if p.path.starts_with("shquery") {
                w += 25.0;
            } else {
                w -= 45.0;
            }
        }
        if kind == "table" {
            // Вопрос про таблицу для запроса: менеджер и выборка с тем же именем — это
            // про код на встроенном языке, а не про текст запроса.
            if p.path.contains("/tables/") {
                w += 40.0;
            }
            if p.path.contains("/properties/") || p.path.contains("/methods/") {
                w -= 30.0;
            }
        }
        if p.title.starts_with("ОбъектМетаданных:") {
            w -= 10.0; // описание метаданного, а не таблица для запроса
        }
        if p.title.starts_with("БиблиотекаКартинок.") {
            w -= 60.0; // картинка с тем же именем не отвечает ни на один вопрос о запросе
        }
        if words.len() > 1 {
            // Чем больше слов вопроса нашлось в заголовке, тем вернее страница. Сравнение
            // по основе: «бухгалтерия» в вопросе и «бухгалтерии» в заголовке — одно слово,
            // и именно оно отделяет регистр бухгалтерии от регистра накопления.
            let up = p.title.to_uppercase();
            let covered = words.iter().filter(|w| up.contains(&stem_word(w))).count();
            w += 8.0 * covered as f64;
            if covered == words.len() {
                w += 20.0; // заголовок покрывает вопрос целиком — сигнал сильнее прочих
            }
        }
        out.push(ScoredPage { page: p, weight: w });
    }

    // Детерминированный тай-брейк при равном весе: короткий заголовок точнее по смыслу,
    // а имя страницы добавлено, чтобы порядок не зависел от обхода карты. Без этого один
    // и тот же вопрос отвечает разными страницами в разных запусках — ровно та болезнь,
    // ради которой убирали hits[0].
    out.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let (ra, rb) = (a.page.title.chars().count(), b.page.title.chars().count());
                ra.cmp(&rb).then_with(|| a.page.object.cmp(&b.page.object))
            })
    });

    let rest: Vec<HelpPage> = out.iter().take(6).map(|sp| sp.page.clone()).collect();
    if out.is_empty() || out[0].weight < SEARCH_THRESHOLD {
        return (None, rest);
    }
    let best = out[0].page.clone();
    // Слабое попадание: вопрос из нескольких слов, а заголовок лучшей страницы покрывает
    // не больше половины из них, и попадание не вендорское (вес ниже соответствия из
    // оглавления). Это страница по одному общему слову — «функция выражение» →
    // ВыражениеXPath. Отказ с перечнем соседей честнее: подмену не видно, отказ виден.
    if words.len() >= 2 && out[0].weight < 90.0 {
        let up = best.title.to_uppercase();
        let covered = words.iter().filter(|w| up.contains(&stem_word(w))).count();
        // Половина — тоже слабо: у вопроса из двух слов совпало одно, общее.
        if covered * 2 <= words.len() {
            return (None, rest);
        }
    }
    (Some(best), rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn разбор_узла_оглавления() {
        // Узел вендорского формата: [id, parent, N, дети…, [1,1,<имена>,путь]].
        // Двуязычная форма даёт русское и английское имя одной темы — на ней и держится
        // перевод ПОДОБНО → LIKE.
        let text = r##"{2,
{1,0,1,2,
{1,1,
{1,1,
{"#","Работа с запросами"}
},""}
},
{2,1,0,
{1,1,
{1,2,
{"ru","ПОДОБНО"},
{"en","LIKE"}
},"/LIKE.html"}
}
}"##;
        let nodes = toc_nodes(text);
        // Узлов два: раздел без пути тоже попадает в разбор — путь у него пустой,
        // и отсеивается он позже, при построении индекса. Сверено с Go 04.09.2026.
        assert_eq!(nodes.len(), 2, "{nodes:?}");
        assert_eq!(nodes[0].ru, "Работа с запросами");
        assert_eq!(nodes[0].path, "", "раздел без страницы");
        assert_eq!(nodes[1].ru, "ПОДОБНО");
        assert_eq!(nodes[1].en, "LIKE");
        assert_eq!(nodes[1].path, "/LIKE.html");
    }

    #[test]
    fn одноязычный_узел_кладётся_в_русское_имя() {
        let text = r##"{2,
{1,0,0,
{1,1,
{1,1,
{"#","Синтаксис текста запросов"}
},"/root.html"}
}
}"##;
        let nodes = toc_nodes(text);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].ru, "Синтаксис текста запросов");
        assert_eq!(nodes[0].en, "");
    }

    #[test]
    fn ключ_сравнения_выбрасывает_пунктуацию() {
        // Тема называется «ИТОГИ ... ПО», и многоточие — оформление, а не имя.
        assert_eq!(norm_key("ИТОГИ … ПО"), "ИТОГИПО");
        assert_eq!(norm_key("Order By"), "ORDERBY");
        assert_eq!(norm_key("  "), "");
        assert_eq!(norm_key("ЕСТЬNULL"), "ЕСТЬNULL");
    }

    #[test]
    fn склейка_слов_разбирается() {
        // «ЛитералДата» не встречается нигде, а «Литерал» + «Дата» находят страницу.
        assert_eq!(split_camel("ЛитералДата"), vec!["Литерал", "Дата"]);
        // Подряд идущие заглавные не разделяются: «И» примыкает к «Оборотам». Так же
        // ведёт себя Go — сверено 04.09.2026, и для поиска это годится: обе части длиннее
        // двух букв и находят нужную страницу.
        assert_eq!(split_camel("ОстаткиИОбороты"), vec!["Остатки", "ИОбороты"]);
        // Куски короче трёх букв выбрасываются: «И» ничего не выбирает.
        assert!(split_camel("Код").len() <= 1);
    }

    #[test]
    fn род_вопроса_узнаётся() {
        for (q, ждём) in [
            ("ПОДОБНО", "query"),
            ("ORDER BY", "query"),
            ("ОстаткиИОбороты", "table"),
            ("РегистрНакопления.Товары", "table"),
            ("виртуальная таблица регистра", "table"),
            ("СрезПоследних", "table"),
            ("Литерал типа ДАТА", "any"),
        ] {
            assert_eq!(question_kind(q), ждём, "question_kind({q:?})");
        }
    }

    #[test]
    fn текст_запроса_отличается_от_имени_конструкции() {
        // Два ключевых слова или одно плюс параметр — это текст, а не тема справки.
        assert!(looks_like_query_text("ВЫБРАТЬ Ссылка ИЗ Справочник.Валюты"));
        assert!(looks_like_query_text("SELECT * FROM Catalog.Товары"));
        assert!(
            looks_like_query_text("ВЫБРАТЬ Код ГДЕ Дата = &Дата"),
            "одно слово плюс параметр"
        );
        // Имя конструкции текстом запроса не считается, даже если оно ключевое слово.
        assert!(!looks_like_query_text("ИТОГИ"));
        assert!(!looks_like_query_text("ОстаткиИОбороты"));
        assert!(!looks_like_query_text("ВЫБРАТЬ"));
    }

    #[test]
    fn слово_считается_целиком_а_не_подстрокой() {
        // Без границ «ВСЕ» нашлось бы внутри «ВСЕГО», а «ИЛИ» — внутри любого «...ИЛИ».
        assert_eq!(count_word("ВСЕ ВСЕГО ВСЕ", "ВСЕ"), 2);
        assert_eq!(count_word("ВСЕГО", "ВСЕ"), 0);
        assert_eq!(count_word("А ИЛИ Б", "ИЛИ"), 1);
        assert_eq!(count_word("ПОСТАВИЛИ", "ИЛИ"), 0);
    }

    #[test]
    fn заглавные_строки_различаются_по_алфавиту() {
        assert!(is_upper_ru("ИТОГИ … ПО"), "многоточие — часть записи");
        assert!(is_upper_en("TOTALS … BY"));
        assert!(!is_upper_ru("TOTALS"), "латиница не кириллица");
        assert!(!is_upper_en("ИТОГИ"));
        assert!(!is_upper_ru("Итоги"), "строчные не проходят");
        assert!(!is_upper_ru(""), "пустая строка — не имя");
    }

    #[test]
    fn голова_конструкции_отрезается_по_многоточию() {
        assert_eq!(head_word("ИТОГИ … ПО"), "ИТОГИ");
        assert_eq!(head_word("TOTALS … BY"), "TOTALS");
        assert_eq!(head_word("ПОДОБНО"), "ПОДОБНО");
    }

    #[test]
    fn основа_слова_обрезается_до_шести_букв() {
        // «бухгалтерия» и «бухгалтерии» должны считаться одним словом.
        assert_eq!(stem_word("бухгалтерия"), stem_word("бухгалтерии"));
        assert_eq!(stem_word("Код"), "КОД");
    }
}
