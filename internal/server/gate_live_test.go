package server_test

import (
	"strings"
	"testing"
)

// Гейт построителя — живьём и на сервере БЕЗ AllowRawQuery: query выполняет только текст,
// собранный query_build; query_check после одного отказа требует построителя. Сценарий
// повторяет ход локальной модели 27.08.2026 (мусор в тексте → ещё текст → текст руками
// в query), и каждый шаг обязан кончиться так, как задумано механизмом, а не правилом.

func TestLiveГейтПостроителя(t *testing.T) {
	cs, ctx := liveSessionWith(t, false)
	handWritten := "ВЫБРАТЬ КОЛИЧЕСТВО(Р.Ссылка) КАК Кол ИЗ Документ.РеализацияТоваровУслуг КАК Р ГДЕ Р.Проведен"

	// 1. Мусор в тексте: отказ платформы, к нему пришит пример вызова построителя.
	out, isErr := call(t, cs, ctx, "query_check", map[string]any{
		"base": "bu3", "query": "ВЫБРАТ Р.Ссылка ИЗ Документ.РеализацияТоваровУслуг КАК Р",
	})
	if !isErr || !strings.Contains(out, "query_build") || !strings.Contains(out, "Документ.РеализацияТоваровУслуг") {
		t.Fatalf("отказ проверки обязан звать построитель с источником: %s", out)
	}

	// 2. Следующий текст, даже верный, не разбирается: после одного отказа — только построитель.
	out, isErr = call(t, cs, ctx, "query_check", map[string]any{"base": "bu3", "query": handWritten})
	if !isErr || !strings.Contains(out, "закрыта после отказа") {
		t.Fatalf("после одного отказа проверка обязана быть закрыта: %s", out)
	}

	// 3. Текст руками в query — не выполняется, каким бы верным он ни был.
	out, isErr = call(t, cs, ctx, "query", map[string]any{"base": "bu3", "query": handWritten})
	if !isErr || !strings.Contains(out, "не собран построителем") {
		t.Fatalf("написанный руками текст обязан быть отвергнут: %s", out)
	}

	// 4. Построитель: выражение в поле и в группировке, порядок с направлением.
	out, isErr = call(t, cs, ctx, "query_build", map[string]any{
		"base": "bu3", "источник": "Документ.РеализацияТоваровУслуг", "псевдоним": "Р",
		"поля": []any{
			map[string]any{"выражение": "НАЧАЛОПЕРИОДА(Р.Дата, МЕСЯЦ)", "как": "Месяц"},
			map[string]any{"поле": "Ссылка", "функция": "КОЛИЧЕСТВО", "как": "Кол"},
		},
		"отбор":       []string{"Дата МЕЖДУ &Н И &К", "Проведен"},
		"группировка": []string{"НАЧАЛОПЕРИОДА(Р.Дата, МЕСЯЦ)"},
		"порядок":     []string{"Кол УБЫВ", "Месяц"},
	})
	if isErr {
		t.Fatalf("построитель отказал: %s", out)
	}
	if !strings.Contains(out, "Кол УБЫВ") {
		t.Errorf("направление порядка потеряно:\n%s", out)
	}
	built := strings.TrimSpace(strings.SplitN(strings.SplitN(out, "платформой:", 2)[1], "Параметры к заполнению", 2)[0])

	// 5. Собранный текст выполняется как есть.
	out, isErr = call(t, cs, ctx, "query", map[string]any{
		"base": "bu3", "query": built,
		"parameters": map[string]any{"Н": "2026-01-01", "К": "2026-12-31"},
	})
	if isErr {
		t.Fatalf("собранный текст обязан выполняться: %s", out)
	}

	// 6. После вызова построителя проверка текста снова открыта — но выполнение не даёт.
	out, isErr = call(t, cs, ctx, "query_check", map[string]any{"base": "bu3", "query": handWritten})
	if isErr || !strings.Contains(out, "Запрос разобран") {
		t.Fatalf("после построителя проверка обязана быть открыта: %s", out)
	}
	if !strings.Contains(out, "выполняется только собранный query_build") {
		t.Errorf("проверка обязана сказать, что к выполнению не открывает: %s", out)
	}

	// 7. Собранный текст с правкой руками — уже не собранный.
	out, isErr = call(t, cs, ctx, "query", map[string]any{
		"base": "bu3", "query": built + " ИТОГИ",
		"parameters": map[string]any{"Н": "2026-01-01", "К": "2026-12-31"},
	})
	if !isErr || !strings.Contains(out, "не собран построителем") {
		t.Fatalf("правленый текст обязан быть отвергнут: %s", out)
	}
}

// Отказ по базе важнее гейта: на незнакомой базе запрос не выполняется по этой причине,
// а не по «не собран построителем» — иначе слабая модель пойдёт чинить не то. (Пустой
// base режет схема SDK ещё до обработчика — это проверяет base_required_test.)
func TestLiveГейтНеПодменяетОтказПоБазе(t *testing.T) {
	cs, ctx := liveSessionWith(t, false)
	out, isErr := call(t, cs, ctx, "query", map[string]any{"base": "нет-такой", "query": "ВЫБРАТЬ 1 КАК Один"})
	if !isErr || strings.Contains(out, "не собран построителем") || !strings.Contains(out, "нет-такой") {
		t.Fatalf("без базы отказ обязан быть про базу: %s", out)
	}
}
