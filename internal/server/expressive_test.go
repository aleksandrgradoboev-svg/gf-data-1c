package server_test

import (
	"strings"
	"testing"
)

// Требование к продукту: агент должен уметь выразить ЛЮБОЙ запрос к данным.
// Здесь проверяются конструкции языка запросов, на которых обёртка обычно и ломается.

func TestВыразительностьПакетСВременнойТаблицей(t *testing.T) {
	cs, ctx := liveSession(t)

	пакет := `ВЫБРАТЬ ПЕРВЫЕ 5
		Ссылка КАК Ссылка,
		Наименование КАК Наименование
	ПОМЕСТИТЬ ВТНоменклатура
	ИЗ Справочник.Номенклатура
	;
	ВЫБРАТЬ
		Наименование КАК Наименование
	ИЗ ВТНоменклатура
	УПОРЯДОЧИТЬ ПО Наименование`

	out, isErr := call(t, cs, ctx, "query", map[string]any{"base": "ut11", "query": пакет})
	if isErr {
		t.Fatalf("пакетный запрос с временной таблицей не выполнен:\n%s", out)
	}
	if !strings.Contains(out, "Наименование") {
		t.Errorf("результат пакета не содержит ожидаемой колонки:\n%s", out)
	}
}

func TestВыразительностьУничтожениеВременнойТаблицы(t *testing.T) {
	cs, ctx := liveSession(t)

	пакет := `ВЫБРАТЬ ПЕРВЫЕ 3 Ссылка КАК Ссылка ПОМЕСТИТЬ ВТ ИЗ Справочник.Номенклатура
	;
	ВЫБРАТЬ Ссылка КАК Ссылка ИЗ ВТ
	;
	УНИЧТОЖИТЬ ВТ`

	out, isErr := call(t, cs, ctx, "query", map[string]any{"base": "ut11", "query": пакет})
	if isErr {
		t.Errorf("пакет с УНИЧТОЖИТЬ отклонён — а это законная часть запроса, не запись:\n%s", out)
	}
}

func TestВыразительностьИтогиПо(t *testing.T) {
	cs, ctx := liveSession(t)

	запрос := `ВЫБРАТЬ
		Ссылка КАК Ссылка,
		Наименование КАК Наименование
	ИЗ Справочник.Номенклатура
	ИТОГИ КОЛИЧЕСТВО(Ссылка) ПО ОБЩИЕ`

	out, isErr := call(t, cs, ctx, "query", map[string]any{"base": "ut11", "query": запрос})
	if isErr {
		t.Errorf("запрос с ИТОГИ не выполнен:\n%s", out)
	}
}

func TestВыразительностьОтборПоСсылке(t *testing.T) {
	cs, ctx := liveSession(t)

	// Берём существующую ссылку и её идентификатор.
	first, isErr := call(t, cs, ctx, "query", map[string]any{
		"base": "ut11", "limit": 1,
		"query": "ВЫБРАТЬ ПЕРВЫЕ 1 Ссылка КАК Ссылка, Наименование КАК Наименование ИЗ Справочник.Номенклатура",
	})
	if isErr {
		t.Fatalf("образец не получен: %s", first)
	}
	id := valueAfter(first, "CatalogRef.Номенклатура, ")
	if id == "" {
		t.Fatalf("идентификатор ссылки не разобран из ответа:\n%s", first)
	}

	// Отбор по ссылке — параметром. Строка тут не годится: платформа молча вернёт ноль,
	// и это неотличимо от «данных нет».
	out, isErr := call(t, cs, ctx, "query", map[string]any{
		"base":  "ut11",
		"query": "ВЫБРАТЬ Наименование КАК Наименование ИЗ Справочник.Номенклатура ГДЕ Ссылка = &Позиция",
		"parameters": map[string]any{
			"Позиция": map[string]any{"тип": "CatalogRef.Номенклатура", "идентификатор": id},
		},
	})
	if isErr {
		t.Fatalf("отбор по ссылке не выполнен:\n%s", out)
	}
	if strings.Contains(out, "строк 0") {
		t.Errorf("отбор по ссылке вернул ноль строк — параметр не стал ссылкой:\n%s", out)
	}
}

// valueAfter достаёт значение, идущее сразу после метки и до закрывающей скобки.
func valueAfter(text, label string) string {
	idx := strings.Index(text, label)
	if idx < 0 {
		return ""
	}
	rest := text[idx+len(label):]
	if end := strings.IndexAny(rest, ")\n"); end >= 0 {
		return strings.TrimSpace(rest[:end])
	}
	return ""
}
