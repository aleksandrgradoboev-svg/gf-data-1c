package tools

// Индекс справки платформы: оглавление синтакс-помощника + ранжирование страниц.
//
// Зачем. Прежний поиск шёл тремя шагами (точное имя объекта, LIKE, полнотекст) и брал
// hits[0] — первую строку, что вернул SQLite без ORDER BY. По журналу живых вызовов это
// давало 52% мимо: половина вопросов получала либо отказ при существующей странице, либо
// чужую страницу молча. Причины были механические, и лечатся они тоже механически.
//
// 1. КЛЮЧИ СТРАНИЦ АНГЛИЙСКИЕ, СПРАШИВАЮТ ПО-РУССКИ. Страница оператора ПОДОБНО называется
//    LIKE, ЕСТЬNULL — ISNULL. Словарь для перевода не нужно составлять руками: он лежит
//    в самой справке. Половина базы (26 648 записей) — это оглавление синтакс-помощника
//    в формате «bracket file», где у каждой темы записаны русское имя, английское имя и
//    путь страницы. Раньше эти записи только шумели в полнотексте.
//
// 2. СПРАВКА НАРЕЗАНА ПО ТЕМАМ, А НЕ ПО ОПЕРАТОРАМ. Страницы «УПОРЯДОЧИТЬ ПО» в природе
//    нет — есть «Упорядочивание результатов запроса». Поэтому ищем не только точное имя,
//    но и тему, чьё ПЕРВОЕ слово совпало с термином, и заголовок, покрывающий все слова
//    вопроса.
//
// 3. ДВА РОДА ВОПРОСОВ, ОДИН ИНДЕКС. Операторы языка живут в shquery (132 страницы),
//    виртуальные таблицы регистров — в shcntx (52 158). Сужать индекс по категории нельзя
//    (тогда ОстаткиИОбороты получает отказ), поэтому вопрос классифицируется, а категория
//    входит в вес: оператор тянет к shquery, таблица — к разделу /tables/.
//
// 4. hits[0] ЗАМЕНЁН ВЕСОМ. У каждого кандидата считается вес по способу находки, и ниже
//    порога ответ не отдаётся вовсе — вместо уверенной чужой страницы список кандидатов.
//
// Оглавление разбирается один раз за жизнь процесса (~1 c) и держится в памяти. Формат
// базы при этом не меняется: пакет работает с уже собранной справкой, ничего пересобирать
// на чужой машине не нужно.

import (
	"database/sql"
	"regexp"
	"sort"
	"strings"
	"sync"
	"unicode"
)

// helpPage — страница справки платформы.
type helpPage struct {
	Title  string
	Object string
	Path   string
}

// tocEntry — страница и имя темы, под которым она попала в индекс первого слова:
// без имени нельзя понять, насколько тема близка к спрошенному термину.
type tocEntry struct {
	Page    helpPage
	TocName string
}

// helpIndex — страницы и разобранное оглавление.
type helpIndex struct {
	pages   map[string]helpPage   // имя страницы (нижний регистр) → страница
	byPath  map[string]string     // путь → имя страницы
	toc     map[string][]helpPage // нормализованное имя темы → страницы
	tocHead map[string][]tocEntry // первое слово темы → страницы (с именем темы)
	alias   map[string]string     // ключевое слово → его вариант на другом языке
	inText  map[string]helpPage   // ключевое слово → тема, в тексте которой оно описано
}

var (
	indexOnce sync.Once
	indexData *helpIndex
)

// tocSources — записи оглавления: язык запросов и объекты платформы.
var tocSources = []string{"shquery_ru.hbk#42", "shcntx_ru.hbk#52159/0"}

// getHelpIndex строит индекс лениво: справка нужна не в каждом сеансе.
func getHelpIndex(db *sql.DB) *helpIndex {
	indexOnce.Do(func() { indexData = buildHelpIndex(db) })
	return indexData
}

// queryScope — SQL-условие «страница относится к запросам».
//
// Инструмент syntax отвечает про ЯЗЫК ЗАПРОСОВ и ТАБЛИЦЫ ПЛАТФОРМЫ, а справка вендора
// содержит и весь встроенный язык: на 132 страницы shquery приходится 52 158 страниц
// shcntx (объектная модель) — 1 к 395. Полнотекстовый поиск по такому составу отвечает
// про что угодно: замер 31.08.2026 показал, как вопрос «SGROUP BY функция выражение»
// вернул страницу про ВыражениеXPath, после чего модель ушла сочинять запрос дальше.
//
// Поэтому в поиске видны только shquery (язык запросов целиком) и раздел /tables/ книги
// shcntx — виртуальные таблицы регистров с их полями и параметрами. Прочая объектная
// модель не удалена из базы (она общая, её читает ещё и скилл kb-1c) — она просто не
// участвует в поиске этого инструмента. Вопрос «что умеет тип» идёт мимо этого условия:
// у него свой путь, members=true.
const queryScope = `config='platform' AND title NOT LIKE '{%' ` +
	`AND (category='shquery' OR path LIKE '%/tables/%')`

func buildHelpIndex(db *sql.DB) *helpIndex {
	ix := &helpIndex{
		pages:   map[string]helpPage{},
		byPath:  map[string]string{},
		toc:     map[string][]helpPage{},
		tocHead: map[string][]tocEntry{},
		alias:   map[string]string{},
		inText:  map[string]helpPage{},
	}
	rows, err := db.Query(`SELECT title, object, path FROM pages
	                        WHERE ` + queryScope)
	if err != nil {
		return ix
	}
	for rows.Next() {
		var p helpPage
		if err := rows.Scan(&p.Title, &p.Object, &p.Path); err != nil {
			continue
		}
		key := strings.ToLower(p.Object)
		if _, seen := ix.pages[key]; !seen {
			ix.pages[key] = p
		}
		ix.byPath[p.Path] = p.Object
	}
	rows.Close()

	for _, src := range tocSources {
		var text string
		if err := db.QueryRow(`SELECT text FROM pages WHERE path = ?`, src).Scan(&text); err != nil {
			continue
		}
		hbk := src
		if i := strings.Index(src, "#"); i >= 0 {
			hbk = src[:i]
		}
		for _, node := range tocNodes(text) {
			if node.Path == "" {
				continue
			}
			page, ok := ix.resolve(hbk, node.Path)
			if !ok {
				continue
			}
			for _, name := range []string{node.RU, node.EN} {
				if name == "" {
					continue
				}
				k := normKey(name)
				if k == "" {
					continue
				}
				ix.toc[k] = append(ix.toc[k], page)
				head := firstWord(name)
				if hk := normKey(head); hk != "" && hk != k {
					ix.tocHead[hk] = append(ix.tocHead[hk], tocEntry{Page: page, TocName: name})
				}
			}
		}
	}

	ix.loadKeywordAliases(db)
	ix.loadKeywordsInText(db)
	return ix
}

// loadKeywordAliases — двуязычие языка запросов, взятое у вендора, а не составленное нами.
//
// В оглавлении shquery двуязычных имён нет ВОВСЕ (0 пар из 197) — в отличие от shcntx, где их
// 24 572. Поэтому английская форма темы языка запросов не находилась ничем: ORDER BY, TOTALS,
// SELECT уходили в отказ или в чужую страницу объектной модели, хотя русская форма отвечала
// верно. Таблица соответствий лежит в самой справке — страница «Двуязычное представление
// ключевых слов»; её и разбираем, вместо того чтобы выписывать пары руками и ошибаться в них.
//
// Словарь двусторонний: спрашивают и по-английски, и по-русски.
func (ix *helpIndex) loadKeywordAliases(db *sql.DB) {
	var text string
	if err := db.QueryRow(`SELECT text FROM pages
	                        WHERE config='platform' AND object='present'`).Scan(&text); err != nil {
		return
	}
	var lines []string
	for _, l := range strings.Split(text, "\n") {
		if l = strings.TrimSpace(l); l != "" {
			lines = append(lines, l)
		}
	}
	// Таблица идёт парами строк «русское / английское». Шапка отсекается сама: пара
	// засчитывается, только когда первая строка целиком кириллическая заглавная, а вторая —
	// латинская заглавная. Заголовок «Английское / написание» этому не отвечает.
	for i := 0; i+1 < len(lines); i++ {
		ru, en := lines[i], lines[i+1]
		if !isUpperRU(ru) || !isUpperEN(en) {
			continue
		}
		ix.addAlias(ru, en)
		// Составные конструкции записаны через многоточие: «ИТОГИ … ПО» / «TOTALS … BY».
		// Спрашивают их обычно головным словом («ИТОГИ», «TOTALS»), поэтому связь заводится
		// и по голове — иначе английская форма самой частой конструкции не находится.
		if hru, hen := headWord(ru), headWord(en); hru != ru || hen != en {
			ix.addAlias(hru, hen)
		}
		i++ // пара разобрана целиком
	}
}

// loadKeywordsInText — слова, у которых СВОЕЙ страницы в справке нет.
//
// Замер двуязычия показал: из 90 ключевых слов языка запросов 10 не находятся ничем — ТОГДА,
// КОГДА, ИЛИ, ИНАЧЕ, СПЕЦСИМВОЛ, НАБОРАМ, ОБЩИЕ, УНИКАЛЬНО и подобные. Это не дефект поиска:
// отдельной страницы у них не существует в природе, они описаны ВНУТРИ тем — «ТОГДА» живёт
// на странице «Операция выбора в языке запросов», «СПЕЦСИМВОЛ» — на странице оператора
// ПОДОБНО. Верный ответ на такой вопрос — страница темы, а не отказ.
//
// Связь выводится из текста, а не из оглавления, поэтому вес у неё ниже вендорского
// соответствия: это догадка по частоте, пусть и надёжная. Берётся страница языка запросов
// с наибольшим числом упоминаний слова; страница самой таблицы двуязычия исключается — она
// упоминает КАЖДОЕ ключевое слово ровно один раз и иначе выигрывала бы у настоящих тем.
func (ix *helpIndex) loadKeywordsInText(db *sql.DB) {
	if len(ix.alias) == 0 {
		return
	}
	rows, err := db.Query(`SELECT title, object, path, text FROM pages
	                        WHERE config='platform' AND category='shquery'
	                          AND object <> 'present' AND title NOT LIKE '{%'`)
	if err != nil {
		return
	}
	defer rows.Close()

	type best struct {
		page  helpPage
		count int
	}
	// Слово берётся в этот индекс, ТОЛЬКО если своей темы у него нет. Иначе выводы по частоте
	// начинают спорить с вендорским оглавлением и проигрывают ему по смыслу: «ДАТА» — тема
	// в оглавлении, а по числу упоминаний она чаще всего попадает на РАЗНОСТЬДАТ. Проверено
	// замером: без этого условия двуязычие падает с 99% до 94%.
	orphan := func(key string) bool {
		if _, ok := ix.toc[key]; ok {
			return false
		}
		if _, ok := ix.tocHead[key]; ok {
			return false
		}
		_, ok := ix.pages[strings.ToLower(key)]
		return !ok
	}

	found := map[string]best{}
	for rows.Next() {
		var p helpPage
		var text string
		if rows.Scan(&p.Title, &p.Object, &p.Path, &text) != nil {
			continue
		}
		upper := strings.ToUpper(text)
		for key := range ix.alias {
			// Ключ уже нормализован — восстанавливать исходное написание не нужно:
			// в тексте справки ключевые слова набраны заглавными.
			if len([]rune(key)) < 3 {
				continue // «В», «И», «ПО» встречаются в любом тексте и ничего не означают
			}
			if !orphan(key) {
				continue
			}
			n := countWord(upper, key)
			if n == 0 {
				continue
			}
			if cur, ok := found[key]; !ok || n > cur.count ||
				(n == cur.count && p.Object < cur.page.Object) {
				found[key] = best{page: p, count: n}
			}
		}
	}
	for key, b := range found {
		ix.inText[key] = b.page
	}
}

// countWord — сколько раз слово встречается как ОТДЕЛЬНОЕ слово. Без границ «ВСЕ» нашлось бы
// внутри «ВСЕГО», а «ИЛИ» — внутри любого «...ИЛИ».
func countWord(haystack, word string) int {
	n, from := 0, 0
	for {
		i := strings.Index(haystack[from:], word)
		if i < 0 {
			return n
		}
		i += from
		leftOK := i == 0 || !isWordRune(rune(haystack[i-1]))
		end := i + len(word)
		rightOK := end >= len(haystack) || !isWordRune(rune(haystack[end]))
		if leftOK && rightOK {
			n++
		}
		from = i + len(word)
	}
}

func isWordRune(r rune) bool {
	return unicode.IsLetter(r) || unicode.IsDigit(r) || r == '_'
}

// addAlias заводит двустороннюю связь между вариантами написания. Первое соответствие
// побеждает: таблица вендора не содержит противоречий, а страховка от них дешевле разбора.
func (ix *helpIndex) addAlias(a, b string) {
	ka, kb := normKey(a), normKey(b)
	if ka == "" || kb == "" || ka == kb {
		return
	}
	if _, seen := ix.alias[kb]; !seen {
		ix.alias[kb] = a
	}
	if _, seen := ix.alias[ka]; !seen {
		ix.alias[ka] = b
	}
}

// headWord — часть конструкции до многоточия: «ИТОГИ … ПО» → «ИТОГИ».
func headWord(s string) string {
	if i := strings.IndexRune(s, '…'); i >= 0 {
		return strings.TrimSpace(s[:i])
	}
	return s
}

// isUpperRU — строка состоит только из заглавной кириллицы (и разделителей).
func isUpperRU(s string) bool { return isUpperOnly(s, 'А', 'Я') }

// isUpperEN — то же для латиницы.
func isUpperEN(s string) bool { return isUpperOnly(s, 'A', 'Z') }

func isUpperOnly(s string, lo, hi rune) bool {
	letters := 0
	for _, r := range s {
		switch {
		case r >= lo && r <= hi, r == 'Ё' && lo == 'А':
			letters++
		// Многоточие — часть записи составных конструкций: «ИТОГИ … ПО» / «TOTALS … BY».
		// Без него пара не опознаётся, и ИТОГИ теряют английскую форму.
		case r == ' ', r == '.', r == '[', r == ']', r == '…':
		default:
			return false
		}
	}
	return letters > 0
}

// translate дописывает к вариантам вопроса их иноязычные соответствия: они идут последними,
// потому что перевод дальше от того, что спросили, чем сама формулировка.
func (ix *helpIndex) translate(variants []string) []string {
	seen := map[string]bool{}
	for _, v := range variants {
		seen[normKey(v)] = true
	}
	out := variants
	for _, v := range variants {
		if alt, ok := ix.alias[normKey(v)]; ok && !seen[normKey(alt)] {
			seen[normKey(alt)] = true
			out = append(out, alt)
		}
	}
	return out
}

// resolve переводит путь из оглавления в страницу базы.
func (ix *helpIndex) resolve(hbk, path string) (helpPage, bool) {
	key := strings.TrimPrefix(path, "/")
	for _, cand := range []string{hbk + "#" + key, hbk + "#" + key + ".html"} {
		if obj, ok := ix.byPath[cand]; ok {
			return ix.pages[strings.ToLower(obj)], true
		}
	}
	leaf := strings.TrimSuffix(key, ".html")
	if i := strings.LastIndex(leaf, "/"); i >= 0 {
		leaf = leaf[i+1:]
	}
	p, ok := ix.pages[strings.ToLower(leaf)]
	return p, ok
}

// ── разбор bracket-формата ───────────────────────────────────────────────────────────────

// tocNode — тема оглавления: имена и путь страницы.
type tocNode struct {
	RU   string
	EN   string
	Path string
}

// bracketValue — либо строка, либо список.
type bracketValue struct {
	Str  string
	List []bracketValue
	IsL  bool
}

// parseBracket разбирает {a,b,{c}} в дерево. Строки в кавычках, удвоенная кавычка внутри —
// экранирование. Формат вендорский, документирован в 1c-syntax/bsl-help-toc-parser.
func parseBracket(s string) bracketValue {
	i := 0
	r := []rune(s)
	var value func() bracketValue
	skipWS := func() {
		for i < len(r) && (r[i] == ' ' || r[i] == '\t' || r[i] == '\r' || r[i] == '\n') {
			i++
		}
	}
	value = func() bracketValue {
		skipWS()
		if i >= len(r) {
			return bracketValue{}
		}
		switch r[i] {
		case '{':
			i++
			out := bracketValue{IsL: true}
			for i < len(r) {
				skipWS()
				if i >= len(r) {
					break
				}
				if r[i] == '}' {
					i++
					break
				}
				if r[i] == ',' {
					i++
					continue
				}
				out.List = append(out.List, value())
			}
			return out
		case '"':
			i++
			var b strings.Builder
			for i < len(r) {
				if r[i] == '"' {
					if i+1 < len(r) && r[i+1] == '"' {
						b.WriteRune('"')
						i += 2
						continue
					}
					i++
					break
				}
				b.WriteRune(r[i])
				i++
			}
			return bracketValue{Str: b.String()}
		}
		start := i
		for i < len(r) && r[i] != ',' && r[i] != '{' && r[i] != '}' && r[i] != '\r' && r[i] != '\n' {
			i++
		}
		return bracketValue{Str: strings.TrimSpace(string(r[start:i]))}
	}
	return value()
}

// tocNodes достаёт темы из записи оглавления. Узел: [id, parent, N, дети…, [1,1,<имена>,путь]],
// где <имена> — либо [1,1,["#","Тема"]], либо [1,2,["ru","ПОДОБНО"],["en","LIKE"]].
func tocNodes(text string) []tocNode {
	root := parseBracket(text)
	if !root.IsL || len(root.List) < 2 {
		return nil
	}
	var out []tocNode
	for _, rec := range root.List[1:] {
		if !rec.IsL || len(rec.List) < 4 {
			continue
		}
		tail := rec.List[len(rec.List)-1]
		if !tail.IsL || len(tail.List) < 4 {
			continue
		}
		titles, path := tail.List[2], tail.List[3]
		if !titles.IsL || path.IsL {
			continue
		}
		var node tocNode
		node.Path = path.Str
		for _, item := range titles.List[2:] {
			if !item.IsL || len(item.List) < 2 {
				continue
			}
			lang, val := item.List[0].Str, item.List[1].Str
			switch lang {
			case "#", "ru":
				node.RU = val
			case "en":
				node.EN = val
			}
		}
		if node.RU != "" || node.EN != "" {
			out = append(out, node)
		}
	}
	return out
}

// ── нормализация вопроса ─────────────────────────────────────────────────────────────────

// noiseWords — связки, которые модель добавляет к вопросу и которые ничего не выбирают.
// Слова вроде «бухгалтерия» сюда НЕ входят намеренно: они и отличают регистр бухгалтерии
// от регистра накопления.
var noiseWords = map[string]bool{
	"виртуальная": true, "виртуальные": true, "виртуальной": true,
	"таблица": true, "таблицы": true, "таблиц": true, "платформы": true,
	"запрос": true, "запроса": true, "язык": true, "языка": true,
	"функция": true, "функции": true, "оператор": true, "операторы": true,
	"ключевое": true, "слово": true, "справка": true,
}

var (
	reParens = regexp.MustCompile(`\([^)]*\)`)
	reAngle  = regexp.MustCompile(`<[^>]*>`)
	reSpaces = regexp.MustCompile(`[\s,]+`)
)

// normKey — ключ сравнения: только буквы и цифры, верхний регистр. Пунктуация выброшена
// не для красоты: тема называется «ИТОГИ ... ПО», и многоточие — оформление, а не имя.
func normKey(s string) string {
	var b strings.Builder
	for _, r := range s {
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			b.WriteRune(unicode.ToUpper(r))
		}
	}
	return b.String()
}

// stemWord — грубая основа: «бухгалтерия» и «бухгалтерии» должны считаться одним словом.
func stemWord(w string) string {
	up := []rune(strings.ToUpper(w))
	if len(up) > 6 {
		up = up[:6]
	}
	return string(up)
}

func firstWord(s string) string {
	f := strings.FieldsFunc(strings.TrimSpace(s), func(r rune) bool { return r == ' ' || r == '.' })
	if len(f) == 0 {
		return ""
	}
	return f[0]
}

// splitCamel: ЛитералДата → [Литерал, Дата]. Склейка слов — обычная форма вопроса модели.
func splitCamel(word string) []string {
	var out []string
	var cur []rune
	var prevUpper bool
	for _, r := range word {
		isUpper := unicode.IsUpper(r)
		if isUpper && !prevUpper && len(cur) > 0 {
			out = append(out, string(cur))
			cur = nil
		}
		cur = append(cur, r)
		prevUpper = isUpper
	}
	if len(cur) > 0 {
		out = append(out, string(cur))
	}
	var keep []string
	for _, p := range out {
		if len([]rune(p)) > 2 {
			keep = append(keep, p)
		}
	}
	return keep
}

// queryVariants — что искать: тема, её сегменты и очищенные формы, в порядке убывания точности.
func queryVariants(query string) []string {
	q := strings.TrimSpace(query)
	out := []string{q}
	cut := strings.TrimSpace(reAngle.ReplaceAllString(reParens.ReplaceAllString(q, " "), " "))
	if cut != "" && cut != q {
		out = append(out, cut)
	}
	if strings.Contains(cut, ".") {
		var segs []string
		for _, s := range strings.Split(cut, ".") {
			if strings.TrimSpace(s) != "" {
				segs = append(segs, s)
			}
		}
		if len(segs) > 0 {
			out = append(out, segs[len(segs)-1], strings.Join(segs, " "))
		}
	}
	words := reSpaces.Split(cut, -1)
	var keep []string
	for _, w := range words {
		if w != "" && !noiseWords[strings.ToLower(w)] {
			keep = append(keep, w)
		}
	}
	if len(keep) > 0 && len(keep) != len(words) {
		out = append(out, strings.Join(keep, " "))
	}
	if len(keep) > 1 {
		out = append(out, keep...)
	}
	seen := map[string]bool{}
	var res []string
	for _, v := range out {
		k := normKey(v)
		if k != "" && !seen[k] {
			seen[k] = true
			res = append(res, v)
		}
	}
	return res
}

// queryKeywords — значимые слова вопроса для поиска по заголовку через И.
func queryKeywords(query string) []string {
	cut := reParens.ReplaceAllString(query, " ")
	cut = strings.Map(func(r rune) rune {
		if r == '<' || r == '>' || r == '(' || r == ')' || r == ',' {
			return ' '
		}
		return r
	}, cut)
	var words []string
	for _, w := range strings.Fields(cut) {
		if noiseWords[strings.ToLower(w)] {
			continue
		}
		for _, seg := range strings.Split(w, ".") {
			if len([]rune(seg)) <= 2 {
				continue
			}
			if parts := splitCamel(seg); len(parts) > 1 {
				// Склейку целиком не ищем: «ЛитералДата» не встречается нигде,
				// а «Литерал» + «Дата» находят страницу «Литерал типа ДАТА».
				words = append(words, parts...)
			} else {
				words = append(words, seg)
			}
		}
	}
	seen := map[string]bool{}
	var res []string
	for _, w := range words {
		k := strings.ToUpper(w)
		if !seen[k] {
			seen[k] = true
			res = append(res, w)
		}
	}
	return res
}

// questionKind — куда тянуть ответ: оператор языка запросов или таблица платформы.
func questionKind(query string) string {
	q := strings.TrimSpace(query)
	low := strings.ToLower(q)
	for _, pref := range []string{"регистр", "register", "справочник", "catalog", "документ", "document"} {
		if i := strings.Index(low, pref); i >= 0 && strings.Contains(low[i:], ".") {
			return "table"
		}
	}
	for _, w := range []string{"таблиц", "регистр", "register", "срез", "остатк", "оборот"} {
		if strings.Contains(low, w) {
			return "table"
		}
	}
	hasLetter, allUpper := false, true
	for _, r := range q {
		if unicode.IsLetter(r) {
			hasLetter = true
			if unicode.IsLower(r) {
				allUpper = false
			}
		}
	}
	if hasLetter && allUpper {
		return "query"
	}
	return "any"
}

// ── ранжирование ─────────────────────────────────────────────────────────────────────────

// scoredPage — кандидат и его вес.
// pageBody — текст и версия страницы ПО ПУТИ. Раньше тело бралось «WHERE object = ? LIMIT 1»:
// под одним именем объекта в базе лежит и настоящая страница, и служебная в скобках
// (заголовков вида «{…» — 26 648), и LIMIT 1 без ORDER BY отдавал любую из них. Прогон
// 27.08.2026: на вопрос про «Первые» модель получила {1, {2, {"",1,0,"",""} … вместо текста.
func pageBody(db *sql.DB, p helpPage) (body, version string) {
	_ = db.QueryRow(`SELECT text, config_version FROM pages WHERE path = ? LIMIT 1`, p.Path).Scan(&body, &version)
	if strings.TrimSpace(body) == "" || strings.HasPrefix(strings.TrimSpace(body), "{") {
		// Путь не нашёлся или сам оказался скобками — берём по объекту, но только страницу
		// с человеческим текстом.
		_ = db.QueryRow(`SELECT text, config_version FROM pages
		                 WHERE object = ? AND config='platform' AND title NOT LIKE '{%' AND text NOT LIKE '{%'
		                 ORDER BY length(title) LIMIT 1`, p.Object).Scan(&body, &version)
	}
	return body, version
}

// reQueryText — ключевые слова, по которым в поле query узнаётся ТЕКСТ ЗАПРОСА, а не
// имя конструкции. Текст запроса нечёткий поиск разбирает на слова и отвечает страницей
// по самому общему из них («выражение» → ВыражениеXPath) — убедительно и мимо.
var reQueryText = regexp.MustCompile(`(?i)(^|[\s(])(ВЫБРАТЬ|SELECT|ИЗ|FROM|ГДЕ|WHERE|СГРУППИРОВАТЬ|GROUP|УПОРЯДОЧИТЬ|ORDER|СОЕДИНЕНИЕ|JOIN|ПОМЕСТИТЬ|INTO|ОБЪЕДИНИТЬ|UNION|ИТОГИ|TOTALS)([\s(]|$)`)

var reWordTok = regexp.MustCompile(`[A-Za-zА-Яа-яЁё_][A-Za-zА-Яа-яЁё0-9_]*`)

// looksLikeQueryText — два и больше ключевых слова языка запросов, либо одно плюс параметр &Имя.
func looksLikeQueryText(q string) bool {
	n := len(reQueryText.FindAllStringIndex(q, -1))
	return n >= 2 || (n >= 1 && strings.Contains(q, "&"))
}

// constructionsIn — конструкции из текста запроса, у которых есть своя страница справки:
// то, что стоит спросить по одной. Ключевые слова и функции пишутся заглавными — по ним
// и отбор; имена таблиц и полей заглавными не бывают и сюда не попадают.
func (ix *helpIndex) constructionsIn(text string) []string {
	seen := map[string]bool{}
	var out []string
	for _, tok := range reWordTok.FindAllString(text, -1) {
		if len([]rune(tok)) < 3 || !(isUpperRU(tok) || isUpperEN(tok)) {
			continue
		}
		key := normKey(tok)
		if seen[key] {
			continue
		}
		_, inToc := ix.toc[key]
		_, inHead := ix.tocHead[key]
		_, inPages := ix.pages[strings.ToLower(tok)]
		_, inBody := ix.inText[key]
		if inToc || inHead || inPages || inBody {
			seen[key] = true
			out = append(out, tok)
		}
	}
	return out
}

type scoredPage struct {
	Page   helpPage
	Weight float64
}

// searchThreshold — ниже этого веса ответ не отдаётся. Лучше список кандидатов, чем
// уверенная чужая страница: отказ виден, подмена — нет.
const searchThreshold = 45

// searchHelp ищет страницу справки платформы. Возвращает лучшую (или nil, если ни один
// кандидат не дотянул до порога) и список кандидатов для подсказки.
func searchHelp(db *sql.DB, query string) (*helpPage, []helpPage) {
	ix := getHelpIndex(db)
	kind := questionKind(query)
	scored := map[string]*scoredPage{}

	add := func(p helpPage, w float64) {
		if p.Object == "" {
			return
		}
		if cur, ok := scored[p.Object]; !ok || w > cur.Weight {
			scored[p.Object] = &scoredPage{Page: p, Weight: w}
		}
	}

	// betterPage — детерминированный тай-брейк при равном весе: короткий заголовок точнее
	// по смыслу, а имя страницы добавлено, чтобы порядок не зависел от обхода map. Без
	// этого один и тот же вопрос отвечает разными страницами в разных запусках — ровно
	// та болезнь, ради которой убирали hits[0].
	betterPage := func(a, b helpPage) bool {
		ra, rb := len([]rune(a.Title)), len([]rune(b.Title))
		if ra != rb {
			return ra < rb
		}
		return a.Object < b.Object
	}

	variants := ix.translate(queryVariants(query))
	for depth, v := range variants {
		fade := 1.0 - 0.12*float64(depth) // чем дальше от исходной формулировки, тем слабее
		key := normKey(v)
		for _, p := range ix.toc[key] { // 1. имя темы в оглавлении
			add(p, 100*fade)
		}
		for _, hp := range ix.tocHead[key] { // 1а. термин — первое слово темы
			// Доля важна: «ИТОГИ» — первое слово и у темы «ИТОГИ … ПО» (query_totals),
			// и у «ИТОГИ … ПО ОБЩИЕ» (overall_totals). Ближе к голому термину та тема,
			// что короче, иначе выбор между ними — случайность.
			// Затухание мягкое (0.6…1.0), а не пропорциональное: соответствие из
			// оглавления — вендорское, и оно точнее случайного совпадения заголовка.
			// Пропорция здесь только разводит темы с одинаковым первым словом.
			share := float64(len([]rune(key))) / float64(len([]rune(normKey(hp.TocName))))
			add(hp.Page, 92*(0.6+0.4*share)*fade)
		}
		for name, pages := range ix.toc { // 1б. имя темы начинается с термина
			if len(key) >= 4 && name != key && strings.HasPrefix(name, key) {
				// Вес по доле совпадения: «ИТОГИ» в «ИТОГИ … ПО» — почти вся тема,
				// «Упорядочивание» в «Упорядочивание по иерархии» — половина.
				share := float64(len([]rune(key))) / float64(len([]rune(name)))
				for _, p := range pages {
					add(p, 88*share*fade)
				}
			}
		}
		if p, ok := ix.pages[strings.ToLower(v)]; ok { // 2. имя страницы
			add(p, 90*fade)
		}
		// 2а. слово описано ВНУТРИ темы, своей страницы у него нет. Вес ниже вендорского
		// соответствия из оглавления: связь выведена из частоты упоминаний, а не задана.
		if p, ok := ix.inText[key]; ok {
			add(p, 80*fade)
		}
		// 3. заголовок. ORDER BY обязателен: без него LIMIT режет наугад — та же болезнь,
		// что hits[0]. Короткий заголовок точнее по смыслу.
		rows, err := db.Query(`SELECT title, object, path FROM pages
		                        WHERE `+queryScope+` AND title LIKE ?
		                        ORDER BY length(title) LIMIT 60`, "%"+v+"%")
		if err == nil {
			for rows.Next() {
				var p helpPage
				if rows.Scan(&p.Title, &p.Object, &p.Path) != nil {
					continue
				}
				w := 55.0
				if strings.HasPrefix(strings.ToUpper(p.Title), strings.ToUpper(v)) {
					w = 70.0
				}
				add(p, w*fade)
			}
			rows.Close()
		}
	}

	// Полнотекстовый проход. Через FTS, а не LIKE: UPPER() в SQLite кириллицу не трогает,
	// поэтому UPPER(title) LIKE '%ЛИТЕРАЛ%' не находит «Литерал типа ДАТА». Токенизатор FTS
	// регистр учитывает верно, а совпадение именно в ЗАГОЛОВКЕ проверяем в коде.
	ftsPass := func(words []string, weight float64) {
		if len(words) == 0 {
			return
		}
		quoted := make([]string, 0, len(words))
		for _, w := range words {
			quoted = append(quoted, `"`+strings.ReplaceAll(w, `"`, ``)+`"`)
		}
		rows, err := db.Query(`SELECT p.title, p.object, p.path FROM pages_fts f
		                        JOIN pages p ON p.id = f.rowid
		                        WHERE pages_fts MATCH ?
		                          AND p.config='platform' AND p.title NOT LIKE '{%'
		                          AND (p.category='shquery' OR p.path LIKE '%/tables/%')
		                        ORDER BY length(p.title) LIMIT 200`, strings.Join(quoted, " AND "))
		if err != nil {
			return
		}
		defer rows.Close()
		for rows.Next() {
			var p helpPage
			if rows.Scan(&p.Title, &p.Object, &p.Path) != nil {
				continue
			}
			up := strings.ToUpper(p.Title)
			all := true
			for _, w := range words {
				if !strings.Contains(up, strings.ToUpper(w)) {
					all = false
					break
				}
			}
			if all {
				add(p, weight)
			}
		}
	}

	words := queryKeywords(query)
	ftsPass(words, 66)

	// Перевод ищется ОТДЕЛЬНЫМ проходом, а не подмешивается к словам вопроса. Слова внутри
	// запроса FTS соединяются через AND, поэтому «ВНЕШНЕЕ» + «OUTER» в одном наборе требует
	// страницу, где есть оба слова сразу, — то есть сужает поиск там, где надо расширить.
	// Проверено на себе: подмешивание увело «ВНЕШНЕЕ» с темы соединений на свойство
	// хранилища двоичных данных. Вес ниже: перевод дальше от того, что спросили.
	for _, v := range variants[1:] {
		if tw := queryKeywords(v); len(tw) > 0 {
			ftsPass(tw, 60)
		}
	}

	out := make([]scoredPage, 0, len(scored))
	for _, sp := range scored {
		w := sp.Weight
		p := sp.Page
		if kind == "query" {
			// Оператор языка живёт в shquery. Совпадение имени в shcntx — почти всегда
			// чужая тема с тем же словом: УПОРЯДОЧИТЬ там метод СКД, ПОРЯДОК — свойство
			// динамического списка. Поэтому не бонус своим, а штраф чужим.
			if strings.HasPrefix(p.Path, "shquery") {
				w += 25
			} else {
				w -= 45
			}
		}
		if kind == "table" {
			// Вопрос про таблицу для запроса: менеджер и выборка с тем же именем — это
			// про код на встроенном языке, а не про текст запроса.
			if strings.Contains(p.Path, "/tables/") {
				w += 40
			}
			if strings.Contains(p.Path, "/properties/") || strings.Contains(p.Path, "/methods/") {
				w -= 30
			}
		}
		if strings.HasPrefix(p.Title, "ОбъектМетаданных:") {
			w -= 10 // описание метаданного, а не таблица для запроса
		}
		if strings.HasPrefix(p.Title, "БиблиотекаКартинок.") {
			w -= 60 // картинка с тем же именем не отвечает ни на один вопрос о запросе
		}
		if len(words) > 1 {
			// Чем больше слов вопроса нашлось в заголовке, тем вернее страница. Сравнение
			// по основе: «бухгалтерия» в вопросе и «бухгалтерии» в заголовке — одно слово,
			// и именно оно отделяет регистр бухгалтерии от регистра накопления.
			up := strings.ToUpper(p.Title)
			covered := 0
			for _, word := range words {
				if strings.Contains(up, stemWord(word)) {
					covered++
				}
			}
			w += 8 * float64(covered)
			if covered == len(words) {
				w += 20 // заголовок покрывает вопрос целиком — сигнал сильнее прочих
			}
		}
		out = append(out, scoredPage{Page: p, Weight: w})
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Weight != out[j].Weight {
			return out[i].Weight > out[j].Weight
		}
		return betterPage(out[i].Page, out[j].Page)
	})

	rest := make([]helpPage, 0, 6)
	for i, sp := range out {
		if i >= 6 {
			break
		}
		rest = append(rest, sp.Page)
	}
	if len(out) == 0 || out[0].Weight < searchThreshold {
		return nil, rest
	}
	best := out[0].Page
	// Слабое попадание: вопрос из нескольких слов, а заголовок лучшей страницы покрывает
	// не больше половины из них, и попадание не вендорское (вес ниже соответствия из
	// оглавления). Это страница по одному общему слову — «функция выражение» →
	// ВыражениеXPath. Отказ с перечнем соседей честнее: подмену не видно, отказ виден.
	if len(words) >= 2 && out[0].Weight < 90 {
		up := strings.ToUpper(best.Title)
		covered := 0
		for _, w := range words {
			if strings.Contains(up, stemWord(w)) {
				covered++
			}
		}
		// Половина — тоже слабо: у вопроса из двух слов совпало одно, общее.
		if covered*2 <= len(words) {
			return nil, rest
		}
	}
	return &best, rest
}
