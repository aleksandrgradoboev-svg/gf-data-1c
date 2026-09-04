//! Инструмент `syntax` — справка платформы 1С: язык запросов и синтакс-помощник.
//!
//! Зачем отдельный инструмент, когда есть обогащение отказов. Обогащение отвечает на ошибку,
//! которая уже случилась, и отвечает коротко: «такой конструкции нет, используйте вот эту».
//! Этого хватает для частого случая и не хватает для остального: какие поля у виртуальной
//! таблицы регистра бухгалтерии, что такое РазвернутыйОстаток, чем ИТОГИ отличаются от
//! группировки. Спросить раньше, чем ошибиться, дешевле — если есть чем спросить.
//!
//! Источник — справка ВЕНДОРА для установленного релиза: `shquery_ru.hbk` (язык запросов) и
//! `shcntx_ru.hbk` (объекты и таблицы платформы), распакованные в общую базу справки
//! (`tools/kb/hbk-extract.py --to-kb`). Не пересказ и не наше знание о языке: язык
//! дополняется между релизами, и «как обычно бывает» стоит здесь ровно столько же, сколько
//! имя объекта, взятое по памяти.
//!
//! Драйвер SQLite взят с `bundled`-сборкой, без системной библиотеки: пакет обязан
//! собираться одной командой на чужой машине.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use crate::refusal::{Kind, Refusal};

use super::members::{members_answer, MemberIndex};
use super::syntaxindex::{
    looks_like_query_text, page_body, search_help, HelpIndex, HelpPage, QUERY_SCOPE,
};
use super::Set;

/// Вход инструмента.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SyntaxInput {
    /// Оператор, функция, таблица или тема: ПОДОБНО, ИТОГИ, ОстаткиИОбороты.
    pub query: String,
    /// Отдать страницу целиком, а не выдержку.
    pub full: bool,
    /// Для типа платформы отдать перечень его методов, свойств, событий и конструкторов
    /// вместо обзорной страницы.
    ///
    /// Отдельным полем, а не догадкой по виду вопроса: «ТаблицаЗначений» — законный вопрос
    /// и про назначение объекта, и про список методов, и решать за спрашивающего, чего он
    /// хотел, значит ошибаться молча.
    pub members: bool,
}

/// Имена файлов базы справки, в порядке предпочтения.
///
/// Отдельный файл платформы идёт первым: справка платформы и справка типовых конфигураций —
/// разные книги для разных читателей (`syntax` читает первую, скилл `kb-1c` — вторую),
/// и одно имя на обе означает, что рано или поздно одна перезапишет другую молча. Общее имя
/// оставлено вторым ради совместимости: там, где справка собрана одним файлом, ничего
/// не сломается.
const KB_NAMES: &[&str] = &["1c-platform-help.db", "1c-help.db"];

/// Порядок поиска базы справки, отделённый от файловой системы, чтобы его можно было
/// проверить тестом: порядок здесь и есть правило, а правило, которое некому нарушить
/// заметно, живёт ровно до первой правки.
pub fn kb_candidates(env: &str, pkg_root: &str, work_dir: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !env.is_empty() {
        out.push(PathBuf::from(env));
    }
    for dir in [pkg_root, work_dir] {
        if dir.is_empty() {
            continue;
        }
        for name in KB_NAMES {
            out.push(Path::new(dir).join("kb").join(name));
        }
    }
    out
}

/// Где лежит база справки.
///
/// Переменная окружения важнее: она позволяет держать справку вне пакета и обновлять её,
/// не пересобирая сервер.
fn kb_path() -> Option<PathBuf> {
    let mut pkg = String::new();
    if let Ok(exe) = std::env::current_exe() {
        // bin → gf-data-1c → корень пакета
        if let Some(p) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            pkg = p.to_string_lossy().to_string();
        }
    }
    let wd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let env = std::env::var("GTDATA_KB").unwrap_or_default();
    kb_candidates(env.trim(), &pkg, &wd)
        .into_iter()
        .find(|c| c.is_file())
}

/// Открывает базу справки на чтение.
fn open_kb() -> Result<Connection, Refusal> {
    let Some(path) = kb_path() else {
        return Err(Refusal::new(
            Kind::BadRequest,
            "справка платформы недоступна",
            "базы справки нет ни по GTDATA_KB, ни рядом с пакетом (kb/1c-platform-help.db, kb/1c-help.db)",
        )
        .hint("собрать: python tools/kb/hbk-extract.py --hbk <платформа>/bin/shquery_ru.hbk --to-kb kb/1c-help.db")
        .hint("то же для shcntx_ru.hbk — там таблицы платформы и их поля")
        .hint("работа по данным этим не блокируется: числа берутся из базы, а не из справки"));
    };
    // Только чтение: инструмент справочный, портить общую базу знаний ему нечем.
    Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| Refusal::new(Kind::Internal, "база справки не открылась", e.to_string()))
}

/// Открывает базу справки для внешнего замера качества.
///
/// Нужна примеру `truth_syntax`, который считает долю попаданий по эталонному набору тем.
/// Замер обязан ходить в ту же базу и тем же путём, что и сам инструмент, — иначе он
/// измеряет не то, чем пользуются.
pub fn open_kb_for_eval() -> Result<Connection, Refusal> {
    open_kb()
}

/// Индексы строятся один раз за жизнь процесса: разбор оглавления занимает около секунды,
/// а справка нужна не в каждом сеансе.
///
/// Как и `sync.Once` в Go, это означает: смена пути к базе внутри процесса не подхватится.
/// Поведение сохранено намеренно — расхождение здесь было бы расхождением в поведении,
/// а не улучшением.
struct Indexes {
    help: HelpIndex,
    members: MemberIndex,
}

fn indexes(db: &Connection) -> &'static Indexes {
    static CELL: OnceLock<Indexes> = OnceLock::new();
    CELL.get_or_init(|| Indexes {
        help: HelpIndex::build(db),
        members: MemberIndex::build(db),
    })
}

/// Выдержка из страницы вокруг спрошенного слова.
///
/// Окно задано в БАЙТАХ, а не в символах, и это не описка. Так считает Go-версия, а
/// кириллица занимает два байта — значит окно в 1600 «единиц» вмещает около 800 русских
/// букв. Считать здесь в символах означало бы отдавать вдвое больше текста, чем отдаёт
/// работающий сервер: не улучшение, а расхождение в поведении на каждой странице справки.
///
/// Резать при этом приходится аккуратнее, чем в Go: там срез по середине буквы даёт битый
/// UTF-8 молча, здесь — панику. Поэтому граница сдвигается до ближайшей начальной точки
/// символа.
fn excerpt(body: &str, needle: &str, full: bool) -> String {
    if full {
        return body.to_string();
    }
    const WINDOW: usize = 1600;
    if body.len() <= WINDOW {
        return body.to_string();
    }
    // Ближайшая слева граница символа — срез по середине буквы паникует.
    fn floor_boundary(s: &str, mut i: usize) -> usize {
        if i >= s.len() {
            return s.len();
        }
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    // Ищем ПО ВЕРХНЕМУ РЕГИСТРУ обеих сторон, а не сперва точное вхождение: иначе
    // «УНИКАЛЬНО» находится в первом попавшемся месте страницы, а не там, где о нём
    // говорится, и выдержка центрируется мимо. Регистр меняет длину в байтах у отдельных
    // букв, поэтому позиция из верхнерегистровой копии годится, только когда длина
    // сохранилась; на кириллице и латинице справки она сохраняется всегда.
    let up = body.to_uppercase();
    let pos = if up.len() == body.len() {
        up.find(&needle.to_uppercase())
    } else {
        body.find(needle)
    };
    let Some(i) = pos else {
        let end = floor_boundary(body, WINDOW);
        return format!(
            "{}\n…\n(страница длиннее; полностью — full: true)",
            &body[..end]
        );
    };
    let start = floor_boundary(body, i.saturating_sub(300));
    let end = floor_boundary(body, (start + WINDOW).min(body.len()));
    format!("…\n{}\n…\n(полностью — full: true)", &body[start..end])
}

/// Укорачивает заголовок для перечня, не разрезая букву пополам.
fn trim_title(s: &str) -> String {
    let r: Vec<char> = s.chars().collect();
    if r.len() <= 70 {
        return s.to_string();
    }
    let head: String = r[..70].iter().collect();
    format!("{head}…")
}

impl Set {
    /// Отдаёт страницу справки платформы.
    pub fn syntax(&self, input: &SyntaxInput) -> Result<String, Refusal> {
        let needle = input.query.trim();
        if needle.is_empty() {
            return Err(Refusal::new(
                Kind::BadRequest,
                "тема не названа",
                "поле query обязательно: оператор, функция, таблица или тема",
            ));
        }
        let db = open_kb()?;
        let ix = indexes(&db);

        if input.members {
            return members_reply(&ix.members, needle);
        }

        // Текст запроса в поле query — не тема справки. Нечёткий поиск ответил бы страницей
        // по самому общему слову; вместо этого называем конструкции, которые в тексте
        // узнаны, и просим спросить их по одной. Проверка текста и его сборка — другие
        // инструменты.
        if looks_like_query_text(needle) {
            let found = ix.help.constructions_in(needle);
            let mut r = Refusal::new(
                Kind::BadRequest,
                "в поле query — текст запроса, а не имя конструкции",
                "справка отвечает на имя оператора, функции или таблицы; текст запроса она не разбирает",
            );
            if !found.is_empty() {
                r = r.hint(format!(
                    "в тексте узнаны конструкции со своей страницей: {}",
                    found.join(", ")
                ));
            }
            return Err(r
                .hint("спрашивайте по одной конструкции: ПОДОБНО, НАЧАЛОПЕРИОДА, СГРУППИРОВАТЬ, ОстаткиИОбороты")
                .hint("правилен ли текст — инструмент query_check; собрать текст без ошибок — query_build"));
        }

        // Спросили про ТИП платформы, а не про язык запросов. Область поиска этого
        // инструмента сужена до языка запросов и таблиц платформы (см. QUERY_SCOPE),
        // поэтому страницы типа здесь нет — но и молчать нельзя: без адреса вопрос уйдёт
        // в перебор названий, а ближайшая по буквам страница из оставшихся ответит
        // правдоподобно и не по делу («ТаблицаЗначений» → «Субконто регистра бухгалтерии»).
        if ix.members.members_of(needle).is_some() {
            return Err(Refusal::new(
                Kind::BadRequest,
                format!("{needle:?} — тип встроенного языка, а не конструкция языка запросов"),
                "перечень его методов и свойств — тот же вызов с members: true",
            )
            .hint("этот инструмент отвечает про язык запросов и таблицы платформы")
            .hint("данные базы читают query и count, объекты конфигурации — metadata и object"));
        }

        let (best, rest) = search_help(&db, &ix.help, needle);
        let Some(best) = best else {
            return Err(not_found(&db, needle, &rest));
        };

        let (body, version) = page_body(&db, &best);

        let mut b = format!(
            "Справка платформы {version} — {}\n{}\n\n",
            best.title, best.path
        );
        b.push_str(&excerpt(&body, needle, input.full));
        if rest.len() > 1 {
            let others: Vec<String> = rest
                .iter()
                .filter(|p| p.object != best.object)
                .take(5)
                .map(|p| trim_title(&p.title))
                .collect();
            if !others.is_empty() {
                b.push_str(&format!("\n\nСмежные страницы: {}", others.join(" · ")));
            }
        }
        Ok(b)
    }
}

/// Отказ при ненайденной теме.
///
/// Отказ при существующей странице — то, что и порождает перебор названий: модель
/// спрашивает «УПОРЯДОЧИТЬ», получает «нет такой», спрашивает «УПОРЯДОЧИТЬ ПО», «ПОРЯДОК»
/// и так далее. Поэтому вместе с отказом отдаём то, что рядом нашлось.
fn not_found(db: &Connection, needle: &str, rest: &[HelpPage]) -> Refusal {
    let near: Vec<String> = rest.iter().map(|p| trim_title(&p.title)).collect();
    // Считаем по той же области, что и ищем: сказать «страниц 52290», обыскав 854,
    // значит соврать о полноте поиска — и подтолкнуть к перебору названий.
    let total: i64 = db
        .query_row(
            &format!("SELECT COUNT(*) FROM pages WHERE {QUERY_SCOPE}"),
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut details: Vec<String> = Vec::new();
    if !near.is_empty() {
        details.push(format!(
            "точного совпадения нет; термин встречается на страницах: {}",
            near.join(" · ")
        ));
    }
    details.push(format!(
        "страниц по языку запросов и таблицам платформы: {total}"
    ));
    details
        .push("назовите как пишется в коде: ОстаткиИОбороты, ПОДОБНО, РазвернутыйОстаток".into());
    details.push("английское имя тоже работает: BalanceAndTurnovers, LIKE".into());
    details.push(
        "справка не собрана — python tools/kb/hbk-extract.py --hbk <платформа>/bin/shcntx_ru.hbk --to-kb kb/1c-help.db".into(),
    );

    let mut r = Refusal::new(
        Kind::BadRequest,
        format!("в справке платформы нет страницы по {needle:?}"),
        details[0].clone(),
    );
    for d in &details[1..] {
        r = r.hint(d.clone());
    }
    r
}

/// Ответ на `members: true`.
///
/// Отказ здесь обязан подсказывать: перечень членов спрашивают, уже зная имя типа, и «нет
/// такого» без вариантов отправляет спрашивающего перебирать написания — ровно то, от чего
/// уводит весь этот инструмент.
fn members_reply(ix: &MemberIndex, type_name: &str) -> Result<String, Refusal> {
    let Some(e) = ix.members_of(type_name) else {
        let mut r = Refusal::new(
            Kind::BadRequest,
            format!("в справке платформы нет типа {type_name:?} с разобранными членами"),
            format!("типов с разобранными членами в базе: {}", ix.type_count()),
        );
        let near = ix.near(type_name);
        if !near.is_empty() {
            r = r.hint(format!("похожие типы: {}", near.join(" · ")));
        }
        return Err(r
            .hint("имя типа пишется как в коде: ТаблицаЗначений, ТабличныйДокумент, Запрос")
            .hint("английское имя тоже работает: ValueTable, SpreadsheetDocument")
            .hint("обзорная страница про назначение объекта — тот же вызов без members"));
    };
    Ok(members_answer(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn порядок_имён_базы_справки() {
        // Отдельный файл платформы идёт первым: справка платформы и справка типовых —
        // разные книги, и одно имя на обе означает молчаливую перезапись.
        let c = kb_candidates("", "/pkg", "/wd");
        let got: Vec<String> = c
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(
            got,
            vec![
                "/pkg/kb/1c-platform-help.db",
                "/pkg/kb/1c-help.db",
                "/wd/kb/1c-platform-help.db",
                "/wd/kb/1c-help.db",
            ]
        );
    }

    #[test]
    fn переменная_окружения_важнее_пакета() {
        // GTDATA_KB позволяет держать справку вне пакета и обновлять её, не пересобирая
        // сервер, — значит она обязана проверяться первой.
        let c = kb_candidates("/env/help.db", "/pkg", "/wd");
        assert_eq!(c[0].to_string_lossy(), "/env/help.db");
        assert_eq!(c.len(), 5);
    }

    #[test]
    fn пустые_каталоги_пропускаются() {
        let c = kb_candidates("", "", "/wd");
        assert_eq!(c.len(), 2, "пустой корень пакета не даёт кандидатов");
        assert!(c[0]
            .to_string_lossy()
            .replace('\\', "/")
            .starts_with("/wd/kb/"));
    }

    #[test]
    fn выдержка_не_режет_букву_пополам() {
        // Байтовая нарезка на кириллице разрезает букву: Go так и делает и выдаёт битый
        // UTF-8 (9 мест из 51 вопроса, замер 04.09.2026). Здесь такой срез — паника,
        // поэтому граница сдвигается до начала символа.
        // 3000 букв «а» — это 6000 байт; окно в 1600 байт приходится на середину буквы.
        let тело = "а".repeat(3000);
        let out = excerpt(&тело, "нетуслова", false);
        assert!(out.contains("страница длиннее"));
        // Главное: строка осталась целой. Байтовый срез Go на этом месте даёт битый UTF-8.
        let выдержка = out.split('\n').next().unwrap_or("");
        assert!(
            выдержка.chars().all(|c| c == 'а'),
            "выдержка обязана состоять из целых букв, получено {выдержка:?}"
        );
        assert_eq!(
            выдержка.len(),
            1600,
            "окно считается в байтах, как в Go: 1600 байт — это 800 русских букв"
        );
    }

    #[test]
    fn короткая_страница_отдаётся_целиком() {
        let тело = "Короткий текст справки";
        assert_eq!(excerpt(тело, "текст", false), тело);
    }

    #[test]
    fn full_отдаёт_страницу_целиком() {
        let тело = "а".repeat(5000);
        assert_eq!(excerpt(&тело, "а", true), тело);
    }

    #[test]
    fn заголовок_укорачивается_по_символам() {
        let длинный = "Я".repeat(100);
        let out = trim_title(&длинный);
        assert_eq!(out.chars().count(), 71, "70 символов и многоточие");
        assert!(out.ends_with('…'));
        // Короткий не трогается.
        assert_eq!(trim_title("Короткий"), "Короткий");
    }
}
