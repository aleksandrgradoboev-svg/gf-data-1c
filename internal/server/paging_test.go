package server_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Пагинация и выгрузка в файл: проверяется не «отработало», а что страницы не
// пересекаются и что в файле оказывается столько строк, сколько обещано.

const пробныйЗапрос = "ВЫБРАТЬ Ссылка КАК Ссылка, Наименование КАК Наименование " +
	"ИЗ Справочник.Номенклатура УПОРЯДОЧИТЬ ПО Наименование"

func TestLiveСтраницыНеПересекаются(t *testing.T) {
	cs, ctx := liveSession(t)

	перваяСтраница, isErr := call(t, cs, ctx, "query", map[string]any{
		"base": "ut11", "query": пробныйЗапрос, "limit": 5,
	})
	if isErr {
		t.Fatalf("первая страница не получена: %s", перваяСтраница)
	}
	if !strings.Contains(перваяСтраница, "offset=5") {
		t.Fatalf("ответ не подсказывает следующее смещение:\n%s", перваяСтраница)
	}

	втораяСтраница, isErr := call(t, cs, ctx, "query", map[string]any{
		"base": "ut11", "query": пробныйЗапрос, "limit": 5, "offset": 5,
	})
	if isErr {
		t.Fatalf("вторая страница не получена: %s", втораяСтраница)
	}
	if !strings.Contains(втораяСтраница, "начиная с 5") {
		t.Errorf("вторая страница не сообщает своё смещение:\n%s", втораяСтраница)
	}

	// Строки первой страницы не должны повториться во второй: иначе разбивка врёт.
	for _, имя := range namesOfRows(перваяСтраница) {
		if strings.Contains(втораяСтраница, имя) {
			t.Errorf("строка %q попала на обе страницы — разбивка пересекается", имя)
		}
	}
}

func TestLiveВыгрузкаВФайл(t *testing.T) {
	cs, ctx := liveSession(t)

	target := filepath.Join(t.TempDir(), "nomenclature.csv")
	out, isErr := call(t, cs, ctx, "export", map[string]any{
		"base": "ut11", "query": пробныйЗапрос, "path": target,
	})
	if isErr {
		t.Fatalf("выгрузка не выполнена: %s", out)
	}

	data, err := os.ReadFile(target)
	if err != nil {
		t.Fatalf("файл выгрузки не прочитан: %v", err)
	}
	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) < 2 {
		t.Fatalf("в файле нет данных, только %d строк", len(lines))
	}
	// Первая строка — заголовок с именами колонок.
	if !strings.Contains(lines[0], "Наименование") {
		t.Errorf("в файле нет заголовка колонок: %q", lines[0])
	}

	// Число строк файла (без заголовка) обязано совпасть с обещанным в ответе.
	обещано := extractNumber(out, "Выгружено строк: ")
	фактически := len(lines) - 1
	if обещано != "" && обещано != itoa(фактически) {
		t.Errorf("обещано строк %s, в файле %d", обещано, фактически)
	}
}

func TestLiveВыгрузкаОстанавливаетсяПредохранителем(t *testing.T) {
	cs, ctx := liveSession(t)

	target := filepath.Join(t.TempDir(), "capped.csv")
	out, isErr := call(t, cs, ctx, "export", map[string]any{
		"base": "ut11", "query": пробныйЗапрос, "path": target, "max_rows": 3,
	})
	if isErr {
		t.Fatalf("выгрузка не выполнена: %s", out)
	}
	if !strings.Contains(out, "Выгружено строк: 3") {
		t.Errorf("предохранитель не сработал:\n%s", out)
	}
	// Молчаливое усечение — главная ложь выгрузки: о нём обязано быть сказано.
	if !strings.Contains(out, "Это не весь результат") {
		t.Errorf("об усечении не предупреждено:\n%s", out)
	}
}

// namesOfRows достаёт значения колонки Наименование из напечатанной таблицы.
func namesOfRows(table string) []string {
	var names []string
	for _, line := range strings.Split(table, "\n") {
		idx := strings.Index(line, "Наименование = ")
		if idx < 0 {
			continue
		}
		name := strings.TrimSpace(line[idx+len("Наименование = "):])
		if name != "" {
			names = append(names, name)
		}
	}
	return names
}

func itoa(n int) string {
	if n == 0 {
		return "0"
	}
	var digits []byte
	for n > 0 {
		digits = append([]byte{byte('0' + n%10)}, digits...)
		n /= 10
	}
	return string(digits)
}
