package tools

// Инструмент syntax — справка платформы 1С: язык запросов и синтакс-помощник.
//
// Зачем отдельный инструмент, когда есть обогащение отказов. Обогащение отвечает на ошибку,
// которая уже случилась, и отвечает коротко: «такой конструкции нет, используйте вот эту».
// Этого хватает для частого случая и не хватает для остального: какие поля у виртуальной
// таблицы регистра бухгалтерии, что такое РазвернутыйОстаток, чем ИТОГИ отличаются от
// группировки. Спросить раньше, чем ошибиться, дешевле — если есть чем спросить.
//
// Источник — справка ВЕНДОРА для установленного релиза: shquery_ru.hbk (язык запросов) и
// shcntx_ru.hbk (объекты и таблицы платформы), распакованные в общую базу справки
// (tools/kb/hbk-extract.py --to-kb). Не пересказ и не наше знание о языке: язык дополняется
// между релизами, и «как обычно бывает» стоит здесь ровно столько же, сколько имя объекта,
// взятое по памяти.
//
// База одна на всё — та же, где лежит справка типовых конфигураций. Это сознательно: два
// хранилища одного знания разойдутся, а обновлять справку без пересборки сервера нужно уметь.
// Драйвер SQLite взят чистый на Go (modernc), без cgo: пакет обязан собираться одной командой
// на чужой машине.

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"
	_ "modernc.org/sqlite"

	"github.com/greentech/gt-data-1c/internal/refusal"
)

// SyntaxInput — вход инструмента.
type SyntaxInput struct {
	Query string `json:"query" jsonschema:"Оператор, функция, таблица или тема: ПОДОБНО, ИТОГИ, ОстаткиИОбороты, РазвернутыйОстаток"`
	Full  bool   `json:"full,omitempty" jsonschema:"Отдать страницу целиком, а не выдержку"`
	// Members просит перечень членов ТИПА вместо страницы про него. Отдельным полем, а не
	// догадкой по виду вопроса: «ТаблицаЗначений» — законный вопрос и про назначение объекта,
	// и про список методов, и решать за спрашивающего, чего он хотел, значит ошибаться молча.
	Members bool `json:"members,omitempty" jsonschema:"Для типа платформы отдать перечень его методов, свойств, событий и конструкторов вместо обзорной страницы"`
}

// SyntaxTool — объявление инструмента.
func SyntaxTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "syntax",
		Description: "Справка платформы 1С: ЯЗЫК ЗАПРОСОВ (операторы, функции, соединения, итоги, " +
			"шаблоны ПОДОБНО) и ТАБЛИЦЫ ПЛАТФОРМЫ (виртуальные таблицы регистров, их поля и параметры). " +
			"Отвечает страницей из справки вендора для установленной версии платформы, а не пересказом. " +
			"Спрашивай ДО того, как сочинять конструкцию: выдуманный оператор даёт либо отказ, либо " +
			"неверный отбор. Не про данные базы (это query) и не про объекты конфигурации (это metadata и object). " +
			"Вопрос «что умеет тип» — members=true: перечень методов и свойств одним ответом, " +
			"вместо страницы про назначение объекта.",
	}
}

// kbNames — имена файлов базы справки, в порядке предпочтения. Отдельный файл платформы идёт
// первым: справка платформы и справка типовых конфигураций — разные книги для разных читателей
// (syntax читает первую, скилл kb-1c — вторую), и одно имя на обе означает, что рано или поздно
// одна перезапишет другую молча. Общее имя оставлено вторым ради совместимости: там, где справка
// собрана одним файлом, ничего не сломается.
var kbNames = []string{"1c-platform-help.db", "1c-help.db"}

// kbCandidates — порядок поиска базы справки, отделённый от файловой системы, чтобы его можно
// было проверить тестом: порядок здесь и есть правило, а правило, которое некому нарушить
// заметно, живёт ровно до первой правки.
func kbCandidates(env, pkgRoot, workDir string) []string {
	var out []string
	if env != "" {
		out = append(out, env)
	}
	for _, dir := range []string{pkgRoot, workDir} {
		if dir == "" {
			continue
		}
		for _, name := range kbNames {
			out = append(out, filepath.Join(dir, "kb", name))
		}
	}
	return out
}

// kbPath — где лежит база справки. Переменная окружения важнее: она позволяет держать справку
// вне пакета и обновлять её, не пересобирая сервер.
func kbPath() (string, bool) {
	var pkg, wd string
	if exe, err := os.Executable(); err == nil {
		pkg = filepath.Dir(filepath.Dir(filepath.Dir(exe))) // bin → gt-data-1c → корень пакета
	}
	if cur, err := os.Getwd(); err == nil {
		wd = cur
	}
	candidates := kbCandidates(strings.TrimSpace(os.Getenv("GTDATA_KB")), pkg, wd)
	for _, c := range candidates {
		if st, err := os.Stat(c); err == nil && !st.IsDir() {
			return c, true
		}
	}
	return "", false
}

func openKB() (*sql.DB, error) {
	path, ok := kbPath()
	if !ok {
		return nil, refusal.New(refusal.BadRequest, "справка платформы недоступна",
			"базы справки нет ни по GTDATA_KB, ни рядом с пакетом (kb/1c-platform-help.db, kb/1c-help.db)",
			"собрать: python tools/kb/hbk-extract.py --hbk <платформа>/bin/shquery_ru.hbk --to-kb kb/1c-help.db",
			"то же для shcntx_ru.hbk — там таблицы платформы и их поля",
			"работа по данным этим не блокируется: числа берутся из базы, а не из справки")
	}
	// Только чтение: инструмент справочный, портить общую базу знаний ему нечем.
	db, err := sql.Open("sqlite", "file:"+filepath.ToSlash(path)+"?mode=ro")
	if err != nil {
		return nil, refusal.New(refusal.Internal, "база справки не открылась", err.Error())
	}
	return db, nil
}

func excerpt(body string, needle string, full bool) string {
	if full {
		return body
	}
	const window = 1600
	if len(body) <= window {
		return body
	}
	up := strings.ToUpper(body)
	i := strings.Index(up, strings.ToUpper(needle))
	if i < 0 {
		return body[:window] + "\n…\n(страница длиннее; полностью — full: true)"
	}
	start := i - 300
	if start < 0 {
		start = 0
	}
	end := start + window
	if end > len(body) {
		end = len(body)
	}
	return "…\n" + body[start:end] + "\n…\n(полностью — full: true)"
}

// Syntax отдаёт страницу справки платформы.
func (s *Set) Syntax(_ context.Context, _ *mcp.CallToolRequest, in SyntaxInput) (*mcp.CallToolResult, any, error) {
	needle := strings.TrimSpace(in.Query)
	if needle == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "тема не названа",
			"поле query обязательно: оператор, функция, таблица или тема")
	}
	db, err := openKB()
	if err != nil {
		return nil, nil, err
	}
	defer db.Close()

	if in.Members {
		return membersAnswer(db, needle)
	}

	// Текст запроса в поле query — не тема справки. Нечёткий поиск ответил бы страницей по
	// самому общему слову; вместо этого называем конструкции, которые в тексте узнаны, и
	// просим спросить их по одной. Проверка текста и его сборка — другие инструменты.
	if looksLikeQueryText(needle) {
		found := getHelpIndex(db).constructionsIn(needle)
		hints := []string{
			"спрашивайте по одной конструкции: ПОДОБНО, НАЧАЛОПЕРИОДА, СГРУППИРОВАТЬ, ОстаткиИОбороты",
			"правилен ли текст — инструмент query_check; собрать текст без ошибок — query_build",
		}
		if len(found) > 0 {
			hints = append([]string{"в тексте узнаны конструкции со своей страницей: " + strings.Join(found, ", ")}, hints...)
		}
		return nil, nil, refusal.New(refusal.BadRequest,
			"в поле query — текст запроса, а не имя конструкции",
			"справка отвечает на имя оператора, функции или таблицы; текст запроса она не разбирает",
			hints...)
	}

	// Спросили про ТИП платформы, а не про язык запросов. Область поиска этого инструмента
	// сужена до языка запросов и таблиц платформы (см. queryScope), поэтому страницы типа
	// здесь нет — но и молчать нельзя: без адреса вопрос уйдёт в перебор названий, а
	// ближайшая по буквам страница из оставшихся ответит правдоподобно и не по делу
	// («ТаблицаЗначений» → «Субконто регистра бухгалтерии»).
	if _, isType := getMemberIndex(db).membersOf(needle); isType {
		return nil, nil, refusal.New(refusal.BadRequest,
			fmt.Sprintf("%q — тип встроенного языка, а не конструкция языка запросов", needle),
			"перечень его методов и свойств — тот же вызов с members: true",
			"этот инструмент отвечает про язык запросов и таблицы платформы",
			"данные базы читают query и count, объекты конфигурации — metadata и object")
	}

	best, rest := searchHelp(db, needle)
	if best == nil {
		// Отказ при существующей странице — то, что и порождает перебор названий: модель
		// спрашивает «УПОРЯДОЧИТЬ», получает «нет такой», спрашивает «УПОРЯДОЧИТЬ ПО»,
		// «ПОРЯДОК» и так далее. Поэтому вместе с отказом отдаём то, что рядом нашлось.
		var near []string
		for _, p := range rest {
			near = append(near, trimTitle(p.Title))
		}
		details := []string{}
		if len(near) > 0 {
			details = append(details,
				"точного совпадения нет; термин встречается на страницах: "+strings.Join(near, " · "))
		}
		var total int
		// Считаем по той же области, что и ищем: сказать «страниц 52290», обыскав 1611,
		// значит соврать о полноте поиска — и подтолкнуть к перебору названий.
		_ = db.QueryRow(`SELECT COUNT(*) FROM pages WHERE ` + queryScope).Scan(&total)
		details = append(details,
			fmt.Sprintf("страниц по языку запросов и таблицам платформы: %d", total),
			"назовите как пишется в коде: ОстаткиИОбороты, ПОДОБНО, РазвернутыйОстаток",
			"английское имя тоже работает: BalanceAndTurnovers, LIKE",
			"справка не собрана — python tools/kb/hbk-extract.py --hbk <платформа>/bin/shcntx_ru.hbk --to-kb kb/1c-help.db")
		return nil, nil, refusal.New(refusal.BadRequest,
			fmt.Sprintf("в справке платформы нет страницы по %q", needle),
			details[0], details[1:]...)
	}

	body, version := pageBody(db, *best)

	var b strings.Builder
	fmt.Fprintf(&b, "Справка платформы %s — %s\n%s\n\n", version, best.Title, best.Path)
	b.WriteString(excerpt(body, needle, in.Full))
	if len(rest) > 1 {
		var others []string
		for _, p := range rest {
			if p.Object == best.Object {
				continue
			}
			others = append(others, trimTitle(p.Title))
			if len(others) >= 5 {
				break
			}
		}
		if len(others) > 0 {
			b.WriteString("\n\nСмежные страницы: " + strings.Join(others, " · "))
		}
	}
	return text(b.String()), nil, nil
}

// trimTitle укорачивает заголовок для перечня, не разрезая букву пополам.
func trimTitle(s string) string {
	r := []rune(s)
	if len(r) <= 70 {
		return s
	}
	return string(r[:70]) + "…"
}
