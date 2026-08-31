package server_test

import (
	"strings"
	"testing"
)

// Три дыры построителя, найденные прогоном локальной модели 27.08.2026 (сессия 16:09):
// счёт строк без поля, группировка по псевдониму колонки, регистр бухгалтерии в register.
// Каждая закрывала модели законный путь — и она либо считала руками, либо шла не туда.

func TestLiveПостроительСчётСтрокИПсевдонимГруппировки(t *testing.T) {
	cs, ctx := liveSessionWith(t, false)

	out, isErr := call(t, cs, ctx, "query_build", map[string]any{
		"base": "bu3", "источник": "Документ.РеализацияТоваровУслуг", "псевдоним": "Док",
		"поля": []any{
			map[string]any{"выражение": "НАЧАЛОПЕРИОДА(Док.Дата, МЕСЯЦ)", "как": "Период"},
			map[string]any{"функция": "КОЛИЧЕСТВО", "как": "Кол"},
		},
		"отбор":       []string{"Дата МЕЖДУ &Н И &К", "Проведен"},
		"группировка": []string{"Период"},
		"порядок":     []string{"Кол УБЫВ", "Период"},
	})
	if isErr {
		t.Fatalf("построитель отказал: %s", out)
	}
	for _, want := range []string{"КОЛИЧЕСТВО(*) КАК Кол", "СГРУППИРОВАТЬ ПО\n\tНАЧАЛОПЕРИОДА(Док.Дата, МЕСЯЦ)", "Кол УБЫВ"} {
		if !strings.Contains(out, want) {
			t.Errorf("в собранном тексте нет %q:\n%s", want, out)
		}
	}
	built := strings.TrimSpace(strings.SplitN(strings.SplitN(out, "платформой:", 2)[1], "Параметры к заполнению", 2)[0])
	out, isErr = call(t, cs, ctx, "query", map[string]any{
		"base": "bu3", "query": built,
		"parameters": map[string]any{"Н": "2026-01-01", "К": "2026-12-31"},
	})
	if isErr || !strings.Contains(out, "Кол = ") {
		t.Fatalf("собранный счёт строк обязан выполняться и отдавать Кол: %s", out)
	}

	// Поле «*» — то же самое другими словами.
	out, isErr = call(t, cs, ctx, "query_build", map[string]any{
		"base": "bu3", "источник": "Справочник.Контрагенты",
		"поля": []any{map[string]any{"поле": "*", "функция": "КОЛИЧЕСТВО", "как": "Всего"}},
	})
	if isErr || !strings.Contains(out, "КОЛИЧЕСТВО(*) КАК Всего") {
		t.Errorf("поле «*» с КОЛИЧЕСТВО обязано давать КОЛИЧЕСТВО(*): %s", out)
	}
}

func TestLiveRegisterНазываетAccountsДляРегистраБухгалтерии(t *testing.T) {
	cs, ctx := liveSessionWith(t, false)
	for _, name := range []string{"Хозрасчетный", "РегистрБухгалтерии.Хозрасчетный.Остатки"} {
		out, isErr := call(t, cs, ctx, "register", map[string]any{
			"base": "bu3", "register": name, "kind": "Остатки", "period": "2026-12-31",
		})
		if !isErr || !strings.Contains(out, "регистр бухгалтерии") || !strings.Contains(out, "accounts") {
			t.Errorf("%s: отказ обязан назвать регистр бухгалтерии и инструмент accounts: %s", name, out)
		}
	}
	// Префикс у регистра накопления — принимается, а не отвергается.
	out, isErr := call(t, cs, ctx, "register", map[string]any{
		"base": "bu3", "register": "РегистрНакопления.РеализацияУслуг", "kind": "Обороты",
		"resources": []string{"СуммаОборот"}, "start": "2026-01-01", "end": "2026-12-31",
	})
	if isErr {
		t.Errorf("имя с префиксом РегистрНакопления. обязано приниматься: %s", out)
	}
	// Регистра нет вовсе — прежний честный отказ, без адреса к accounts.
	out, isErr = call(t, cs, ctx, "register", map[string]any{
		"base": "bu3", "register": "ВыручкаПоКаналамСбыта", "kind": "Обороты",
		"resources": []string{"Сумма"}, "start": "2026-01-01", "end": "2026-12-31",
	})
	if !isErr || !strings.Contains(out, "не найден") || strings.Contains(out, "accounts") {
		t.Errorf("несуществующий регистр: ждали «не найден» без адреса к accounts: %s", out)
	}
}

// Четвёртая дыра, найденная разбором прогона 27.08.2026 (сессия 18:15, К1): периодные
// функции двухаргументные, а построитель склеивал их как одноаргументные — платформа
// молча достраивала пропущенный период до ДЕНЬ. Ответ приходил с ok=true, колонка
// называлась «Месяц», группировка шла по дню: 20 строк вместо 5. Тихая подмена страшнее
// отказа — отказ виден, а этот ответ правдоподобен и неверен.
func TestLiveПериодОбязателенУПериоднойФункции(t *testing.T) {
	cs, ctx := liveSessionWith(t, false)

	// Без периода — отказ, называющий и виновника, и способ починки.
	out, isErr := call(t, cs, ctx, "query_build", map[string]any{
		"base": "bu3", "источник": "Документ.РеализацияТоваровУслуг",
		"поля": []any{map[string]any{"поле": "Дата", "функция": "НАЧАЛОПЕРИОДА", "как": "Месяц"}},
	})
	if !isErr {
		t.Fatalf("НАЧАЛОПЕРИОДА без периода обязана отвергаться, получено:\n%s", out)
	}
	for _, want := range []string{"период", "МЕСЯЦ", "ДЕНЬ"} {
		if !strings.Contains(out, want) {
			t.Errorf("в отказе нет %q — модель не поймёт, что чинить:\n%s", want, out)
		}
	}

	// С периодом — собирается ровно то, что просили, и выполняется.
	out, isErr = call(t, cs, ctx, "query_build", map[string]any{
		"base": "bu3", "источник": "Документ.РеализацияТоваровУслуг",
		"поля": []any{
			map[string]any{"поле": "Дата", "функция": "НАЧАЛОПЕРИОДА", "период": "МЕСЯЦ", "как": "Месяц"},
			map[string]any{"функция": "КОЛИЧЕСТВО", "как": "Кол"},
		},
		"отбор":       []string{"Дата МЕЖДУ &Н И &К"},
		"группировка": []string{"Месяц"},
		"порядок":     []string{"Месяц"},
	})
	if isErr {
		t.Fatalf("построитель отказал с явным периодом: %s", out)
	}
	if strings.Contains(out, ", ДЕНЬ)") {
		t.Errorf("запрошен МЕСЯЦ, а собран ДЕНЬ — та самая тихая подмена:\n%s", out)
	}
	if !strings.Contains(out, "НАЧАЛОПЕРИОДА(РеализацияТоваровУслуг.Дата, МЕСЯЦ)") {
		t.Errorf("в собранном тексте нет периода МЕСЯЦ:\n%s", out)
	}

	// Числа: помесячная разбивка обязана дать 5 строк, а не 20 дневных.
	built := strings.TrimSpace(strings.SplitN(strings.SplitN(out, "платформой:", 2)[1], "Параметры к заполнению", 2)[0])
	out, isErr = call(t, cs, ctx, "query", map[string]any{
		"base": "bu3", "query": built,
		"parameters": map[string]any{"Н": "2026-01-01", "К": "2026-12-31"},
	})
	if isErr {
		t.Fatalf("собранный запрос не выполнился: %s", out)
	}
	if !strings.Contains(out, "строк 5") {
		t.Errorf("помесячная группировка обязана дать 5 строк за 2026:\n%s", out)
	}

	// Период там, где его не бывает, — тоже отказ: молчание научило бы модели неверному.
	out, isErr = call(t, cs, ctx, "query_build", map[string]any{
		"base": "bu3", "источник": "Документ.РеализацияТоваровУслуг",
		"поля": []any{map[string]any{"поле": "СуммаДокумента", "функция": "СУММА", "период": "МЕСЯЦ", "как": "С"}},
	})
	if !isErr || !strings.Contains(out, "периода не принимает") {
		t.Errorf("«период» при СУММА обязан отвергаться:\n%s", out)
	}
}
