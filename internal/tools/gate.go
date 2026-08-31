package tools

import (
	"regexp"
	"strings"
	"sync"
)

// queryGate — механизм вместо правила: query выполняет ТОЛЬКО текст, который в этой же
// сессии собрал query_build (решение 27.08.2026: «завернуть всё на построитель»). Текст,
// написанный руками, не выполняется даже после удачного query_check — тот остаётся
// диагностикой синтаксиса, не пропуском. Соединения и пакеты построитель не собирает,
// и такой вопрос честно возвращается как «не собирается» — это принятая цена.
// Вторая половина: после отказа query_check следующий текст не разбирается, пока не
// позван построитель.
//
// Зачем механизм, а не строка в промпте: правило «три отказа — остановись» стояло в агенте,
// и прогон 27.08.2026 (qwen3.6, Kilo) дал 21 отказ query_check подряд при одном вызове
// построителя — с ключом на китайском. Промпт читают, а не исполняют; сервер исполняет.
//
// Состояние живёт в процессе: в режиме stdio это ровно одна сессия клиента. В сетевом
// режиме (-http) оно общее на всех подключённых — это известное ограничение, а не дефект.
type queryGate struct {
	mu          sync.Mutex
	approved    map[string]bool
	checkLocked bool
	lastSource  string
}

// Порог — ОДИН отказ (решение 27.08.2026): после отказа query_check следующая проверка
// текста не выполняется, пока не вызван query_build. Любой исход вызова построителя
// открывает проверку снова: соединения и пакеты построитель не собирает, и без этой
// оговорки такой запрос встал бы намертво.

func (g *queryGate) init() {
	if g.approved == nil {
		g.approved = map[string]bool{}
	}
}

// approve запоминает текст как проверенный или собранный платформой.
func (g *queryGate) approve(text string) {
	g.mu.Lock()
	defer g.mu.Unlock()
	g.init()
	g.approved[normalizeQuery(text)] = true
}

// isApproved — можно ли выполнять этот текст.
func (g *queryGate) isApproved(text string) bool {
	g.mu.Lock()
	defer g.mu.Unlock()
	g.init()
	return g.approved[normalizeQuery(text)]
}

// onCheckRefused закрывает проверку текста до вызова построителя.
func (g *queryGate) onCheckRefused(text string) {
	g.mu.Lock()
	defer g.mu.Unlock()
	g.checkLocked = true
	g.lastSource = sourceOf(text)
}

// onCheckPassed — текст разобран: замок снят. К выполнению это текст НЕ открывает.
func (g *queryGate) onCheckPassed(text string) {
	g.mu.Lock()
	defer g.mu.Unlock()
	g.checkLocked = false
}

// onBuildCalled — построитель позван; исход неважен, проверка текста снова открыта.
func (g *queryGate) onBuildCalled() {
	g.mu.Lock()
	defer g.mu.Unlock()
	g.checkLocked = false
}

// checkAllowed — открыта ли проверка текста; если нет — вторая строка объясняет, куда идти.
func (g *queryGate) checkAllowed() (bool, string) {
	g.mu.Lock()
	defer g.mu.Unlock()
	if !g.checkLocked {
		return true, ""
	}
	return false, builderHint(g.lastSource)
}

// builderHint — готовый вызов построителя под источник отказанного текста. Пример
// конкретный, а не «воспользуйтесь query_build»: модель копирует форму, а не читает совет.
func builderHint(source string) string {
	if source == "" {
		source = "Документ.ИмяДокумента"
	}
	return "после отказа проверки следующий текст не разбирается — соберите запрос построителем " +
		"query_build (текст пишет платформа, выдуманное поле отвергается поимённо), например: " +
		`{"base": "…", "источник": "` + source + `", "поля": ["Ссылка", ` +
		`{"поле": "СуммаДокумента", "функция": "СУММА", "как": "Сумма"}], ` +
		`"отбор": ["Дата МЕЖДУ &Н И &К"], "группировка": ["Ссылка"]}. ` +
		"Собранный текст выполняйте query как есть. Соединения и пакеты построитель не собирает — " +
		"после его вызова проверка текста открывается снова"
}

var (
	reQuerySpaces = regexp.MustCompile(`\s+`)
	reSource      = regexp.MustCompile(`(?i)(?:^|\s)ИЗ\s+([A-Za-zА-Яа-яЁё0-9_.]+)`)
)

// normalizeQuery — текст сравнивается без учёта пробелов, переносов и регистра: платформа
// отдаёт собранный текст с табуляцией, модель переписывает его в одну строку.
func normalizeQuery(q string) string {
	return strings.ToUpper(strings.TrimSpace(reQuerySpaces.ReplaceAllString(q, " ")))
}

// sourceOf — первая таблица после ИЗ; для примера вызова построителя, не для разбора.
func sourceOf(q string) string {
	if m := reSource.FindStringSubmatch(q); m != nil {
		return strings.TrimSuffix(m[1], ".")
	}
	return ""
}
