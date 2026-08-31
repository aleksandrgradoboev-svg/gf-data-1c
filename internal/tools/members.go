package tools

// Срез членов типа: все методы, свойства, события и конструкторы одним ответом.
//
// Зачем. Справка нарезана по одной странице на КАЖДЫЙ метод: у ТаблицаЗначений их 19, у
// ТабличныйДокумент — 45, у глобального контекста — 527. Вопрос «что умеет этот тип» через
// поиск по странице типа не решается: обзорная страница описывает назначение объекта и не
// перечисляет членов. Собрать перечень можно было только перебором страниц, по вызову на метод.
//
// Замер 26.08.2026 против mcp-bsl-context 0.3.2 на восьми типах встроенного языка: чужой
// getMembers отдал 102 метода, наш syntax — 0, хотя отвечал на все восемь. Отвечал обзорной
// страницей: 1241 символ про то, что такое таблица значений, и ни одного метода. Преимущество
// чужого сервера оказалось не в данных, а в форме подачи — те же страницы того же .hbk.
//
// Данные для среза уже лежат в нашей базе, разложенные вендором по путям:
//
//	.../objects/catalog234/catalog236/ValueTable/methods/Add110.html
//	.../objects/catalog234/catalog236/ValueTable/properties/Columns1.html
//
// 963 типа, 7199 методов, 13 662 свойства, 693 события, 444 конструктора. Внешних служб для
// этого не нужно — ни JDK, ни индекса, ни отдельного сервера.

import (
	"database/sql"
	"fmt"
	"regexp"
	"sort"
	"strings"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/refusal"
)

// memberKind — раздел, в котором член лежит у вендора.
type memberKind string

const (
	kindMethod   memberKind = "методы"
	kindProperty memberKind = "свойства"
	kindEvent    memberKind = "события"
	kindCtor     memberKind = "конструкторы"
)

// kindByDir — каталог пути → раздел. Имена каталогов заданы вендором, а не нами.
var kindByDir = map[string]memberKind{
	"methods":    kindMethod,
	"properties": kindProperty,
	"events":     kindEvent,
	"ctors":      kindCtor,
}

// typeMember — один член типа.
type typeMember struct {
	Name    string // русское имя: Добавить
	NameEN  string // английское: Add
	Kind    memberKind
	Title   string // заголовок страницы целиком
	Path    string
	Returns string // возвращаемый тип, если справка его называет
}

// typeMembers — члены одного типа, сгруппированные по разделам.
type typeMembers struct {
	TypeRU  string
	TypeEN  string
	Members []typeMember
}

// memberIndex — все типы, у которых есть разобранные члены.
type memberIndex struct {
	byType map[string]*typeMembers // нормализованное имя типа (РУ и EN) → члены
}

var (
	memberOnce sync.Once
	memberData *memberIndex
)

// titleRx — заголовок страницы члена: «ТаблицаЗначений.Добавить (ValueTable.Add)».
var titleRx = regexp.MustCompile(`^(.+?)\.(.+?)\s+\((.+?)\.(.+?)\)\s*$`)

// pathRx — раздел и английское имя типа из пути вендора.
var pathRx = regexp.MustCompile(`/([^/]+)/(methods|properties|events|ctors)/`)

// returnsRx — строка «Возвращаемое значение:» и тип за ней. Справка называет его не всегда,
// и отсутствие типа — не ошибка разбора: у процедур его нет.
var returnsRx = regexp.MustCompile(`(?i)Возвращаемое значение:\s*\n?\s*([^\n]{1,80})`)

// fileRx — последний сегмент пути без расширения и без хвостового номера страницы:
// «.../methods/Insert582.html» → «Insert». Номер приписывает распаковщик справки, он не
// часть имени метода. Расширение любое: в справке встречаются и .html, и .st — привязка
// к одному .html молча теряла половину членов типа (34 из 65 у ТаблицаЗначений).
var fileRx = regexp.MustCompile(`/([A-Za-z][A-Za-z0-9]*?)\d*\.[A-Za-z0-9]+$`)

// memberNameFromPath достаёт латинское имя члена из пути страницы. Нужно там, где заголовок
// оказался служебной строкой формата hbk: имя в пути есть всегда, а заголовок бывает битым.
func memberNameFromPath(path string) string {
	m := fileRx.FindStringSubmatch(path)
	if m == nil {
		return ""
	}
	return m[1]
}

func getMemberIndex(db *sql.DB) *memberIndex {
	memberOnce.Do(func() { memberData = buildMemberIndex(db) })
	return memberData
}

func buildMemberIndex(db *sql.DB) *memberIndex {
	ix := &memberIndex{byType: map[string]*typeMembers{}}
	rows, err := db.Query(`SELECT title, path, text FROM pages
	                        WHERE config='platform'
	                          AND (path LIKE '%/methods/%' OR path LIKE '%/properties/%'
	                               OR path LIKE '%/events/%' OR path LIKE '%/ctors/%')`)
	if err != nil {
		return ix
	}
	defer rows.Close()

	for rows.Next() {
		var title, path, text string
		if rows.Scan(&title, &path, &text) != nil {
			continue
		}
		pm := pathRx.FindStringSubmatch(path)
		if pm == nil {
			continue
		}
		typeEN := pm[1]
		kind, ok := kindByDir[pm[2]]
		if !ok {
			continue
		}

		// Служебная строка формата hbk вместо заголовка — имя берётся из пути.
		if strings.HasPrefix(strings.TrimSpace(title), "{") {
			if name := memberNameFromPath(path); name != "" {
				title = name
			} else {
				continue
			}
		}

		m := typeMember{Kind: kind, Title: title, Path: path}
		var typeRU string
		// Заголовок вида «Тип.Член (Type.Member)» разбирается у 394 страниц из 400. Остальные
		// записаны без префикса типа («Прочитать (Read)») — там имя типа берётся из пути,
		// а русское остаётся пустым и подставляется от собратьев по типу ниже.
		if tm := titleRx.FindStringSubmatch(strings.TrimSpace(title)); tm != nil {
			typeRU, m.Name, m.NameEN = tm[1], tm[2], tm[4]
		} else {
			m.Name = strings.TrimSpace(title)
			if i := strings.Index(m.Name, " ("); i > 0 {
				m.NameEN = strings.Trim(m.Name[i+2:], "() ")
				m.Name = m.Name[:i]
			}
		}
		if r := returnsRx.FindStringSubmatch(text); r != nil {
			m.Returns = cleanReturns(r[1])
		}

		entry := ix.byType[strings.ToLower(typeEN)]
		if entry == nil {
			entry = &typeMembers{TypeEN: typeEN, TypeRU: typeRU}
			ix.byType[strings.ToLower(typeEN)] = entry
		}
		if entry.TypeRU == "" && typeRU != "" {
			entry.TypeRU = typeRU
		}
		entry.Members = append(entry.Members, m)
	}

	// Русское имя типа известно только из заголовков его членов, поэтому связь РУ → члены
	// заводится вторым проходом, когда имя уже собрано.
	for _, e := range ix.byType {
		// Член, восстановленный из пути, — запасной вариант для страниц со служебным
		// заголовком. Если тот же член разобран из нормального заголовка, он и остаётся:
		// у него есть русское имя и тип возврата. Иначе перечень двоится («Add» рядом
		// с «Вставить (Insert)») и выглядит вдвое богаче, чем есть.
		byKey := map[string]typeMember{}
		for _, m := range e.Members {
			key := strings.ToLower(string(m.Kind) + "\x00" + firstNonEmpty(m.NameEN, m.Name))
			cur, seen := byKey[key]
			if !seen || betterMember(m, cur) {
				byKey[key] = m
			}
		}
		e.Members = e.Members[:0]
		for _, m := range byKey {
			e.Members = append(e.Members, m)
		}

		sort.Slice(e.Members, func(i, j int) bool {
			if e.Members[i].Kind != e.Members[j].Kind {
				return kindOrder(e.Members[i].Kind) < kindOrder(e.Members[j].Kind)
			}
			return e.Members[i].Name < e.Members[j].Name
		})
		if e.TypeRU != "" {
			ix.byType[strings.ToLower(e.TypeRU)] = e
		}
	}
	return ix
}


// firstNonEmpty — первое непустое из двух. Ключ дедупликации строится по ЛАТИНСКОМУ имени:
// оно есть у обеих форм записи члена, русское — только у разобранных из заголовка.
func firstNonEmpty(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

// betterMember — какой из двух одинаковых членов оставить. Русское имя и тип возврата
// приходят только из нормального заголовка; восстановленный из пути беднее и уступает.
func betterMember(a, b typeMember) bool {
	ar := a.Name != "" && a.Name != a.NameEN
	br := b.Name != "" && b.Name != b.NameEN
	if ar != br {
		return ar
	}
	return len(a.Returns) > len(b.Returns)
}

// cleanReturns — тип возврата без служебной обёртки справки. Вендор пишет его строкой
// «Тип: СтрокаТаблицыЗначений.» — слово «Тип:» и точка в перечне только мешают, а внутри
// перечисление через запятую сохраняется: «Число, Неопределено» это два допустимых типа,
// а не мусор.
func cleanReturns(s string) string {
	s = strings.TrimSpace(s)
	s = strings.TrimPrefix(s, "Тип:")
	s = strings.TrimPrefix(s, "тип:")
	s = strings.TrimSpace(s)
	s = strings.TrimRight(s, ". ")
	return strings.TrimSpace(s)
}

// kindOrder — порядок разделов в ответе: сперва то, чем пользуются чаще.
func kindOrder(k memberKind) int {
	switch k {
	case kindMethod:
		return 0
	case kindProperty:
		return 1
	case kindEvent:
		return 2
	default:
		return 3
	}
}

// membersOf — члены типа по русскому или английскому имени.
func (ix *memberIndex) membersOf(typeName string) (*typeMembers, bool) {
	e, ok := ix.byType[strings.ToLower(strings.TrimSpace(typeName))]
	return e, ok && e != nil && len(e.Members) > 0
}

// membersAnswer — ответ на вопрос «что умеет этот тип».
//
// Отказ здесь обязан подсказывать: перечень членов спрашивают, уже зная имя типа, и «нет
// такого» без вариантов отправляет спрашивающего перебирать написания — ровно то, от чего
// уводит весь этот инструмент.
func membersAnswer(db *sql.DB, typeName string) (*mcp.CallToolResult, any, error) {
	ix := getMemberIndex(db)
	e, ok := ix.membersOf(typeName)
	if !ok {
		var details []string
		if near := ix.near(typeName); len(near) > 0 {
			details = append(details, "похожие типы: "+strings.Join(near, " · "))
		}
		details = append(details,
			"имя типа пишется как в коде: ТаблицаЗначений, ТабличныйДокумент, Запрос",
			"английское имя тоже работает: ValueTable, SpreadsheetDocument",
			"обзорная страница про назначение объекта — тот же вызов без members")
		return nil, nil, refusal.New(refusal.BadRequest,
			fmt.Sprintf("в справке платформы нет типа %q с разобранными членами", typeName),
			fmt.Sprintf("типов с разобранными членами в базе: %d", ix.typeCount()),
			details...)
	}

	name := e.TypeRU
	if name == "" {
		name = e.TypeEN
	} else if e.TypeEN != "" {
		name += " (" + e.TypeEN + ")"
	}

	var b strings.Builder
	fmt.Fprintf(&b, "%s — членов: %d\n", name, len(e.Members))

	var cur memberKind
	for _, m := range e.Members {
		if m.Kind != cur {
			cur = m.Kind
			fmt.Fprintf(&b, "\n== %s ==\n", cur)
		}
		line := "  " + m.Name
		if m.NameEN != "" && m.NameEN != m.Name {
			line += " (" + m.NameEN + ")"
		}
		if m.Returns != "" {
			line += " → " + m.Returns
		}
		b.WriteString(line + "\n")
	}
	b.WriteString("\nПодробности по любому члену — тот же инструмент без members: " +
		"запрос вида «" + firstMemberExample(e) + "».\n")
	return text(b.String()), nil, nil
}

// firstMemberExample — как спросить подробности: пример строится из реального члена этого типа,
// а не из выдуманного, иначе подсказка ведёт в отказ.
func firstMemberExample(e *typeMembers) string {
	if len(e.Members) == 0 {
		return ""
	}
	name := e.TypeRU
	if name == "" {
		name = e.TypeEN
	}
	return name + "." + e.Members[0].Name
}

// near — типы, чьё имя похоже на спрошенное. Дешёвая проверка на вхождение подстроки:
// перебирать 963 имени незачем, а опечатку в одну букву она не ловит — и не обещает.
func (ix *memberIndex) near(typeName string) []string {
	needle := strings.ToLower(strings.TrimSpace(typeName))
	if len([]rune(needle)) < 4 {
		return nil
	}
	seen := map[string]bool{}
	var out []string
	for key, e := range ix.byType {
		if !strings.Contains(key, needle) && !strings.Contains(needle, key) {
			continue
		}
		name := e.TypeRU
		if name == "" {
			name = e.TypeEN
		}
		if seen[name] {
			continue
		}
		seen[name] = true
		out = append(out, name)
		if len(out) >= 8 {
			break
		}
	}
	sort.Strings(out)
	return out
}

// typeCount — сколько РАЗНЫХ типов в индексе. Ключей вдвое больше: русское и английское имя
// ведут на одну запись, и счёт по ключам завысил бы число вдвое.
func (ix *memberIndex) typeCount() int {
	seen := map[*typeMembers]bool{}
	for _, e := range ix.byType {
		seen[e] = true
	}
	return len(seen)
}
