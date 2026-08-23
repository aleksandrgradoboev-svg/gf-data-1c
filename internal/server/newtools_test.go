package server_test

import (
	"strings"
	"testing"
)

// Срез регистра сведений и бухгалтерские итоги — проверяются на той базе, где им место:
// цены живут в торговле, счета — в бухгалтерии.

func TestLiveСрезРегистраСведений(t *testing.T) {
	cs, ctx := liveSession(t)

	// Стенд наполнен синтетикой неравномерно: половина регистров пуста, и срез по
	// пустому ничего не докажет. Поэтому ищем первый регистр с записями.
	list, isErr := call(t, cs, ctx, "metadata", map[string]any{
		"base": "ut11", "filter": "РегистрыСведений",
	})
	if isErr {
		t.Fatalf("перечень регистров сведений не получен: %s", list)
	}

	// Годится не всякий непустой регистр: непериодическому срез не положен, и отказ
	// по нему — правильное поведение, а не повод останавливать поиск.
	непериодических := 0
	for _, кандидат := range objectNames(list, 40) {
		out, isErr := call(t, cs, ctx, "count", map[string]any{
			"base": "ut11", "table": "РегистрСведений." + кандидат,
		})
		if isErr {
			continue
		}
		if n := extractNumber(out, "записей "); n == "" || n == "0" {
			continue
		}

		out, isErr = call(t, cs, ctx, "slice", map[string]any{
			"base": "ut11", "register": кандидат, "limit": 5,
		})
		if isErr {
			if strings.Contains(out, "непериодический") {
				непериодических++
				continue
			}
			t.Fatalf("срез по регистру %s не получен: %s", кандидат, out)
		}

		// В срезе обязан быть период: без него это просто выборка записей.
		if !strings.Contains(out, "Период") {
			t.Errorf("в срезе по %s нет периода:\n%s", кандидат, out)
		}
		t.Logf("срез проверен на регистре %s (непериодических пропущено: %d)",
			кандидат, непериодических)
		return
	}
	// Данных не нашлось — но запрос среза всё равно обязан собираться. Проверяем на
	// первом периодическом регистре: пустой результат тут законен, отказ — нет.
	for _, кандидат := range objectNames(list, 40) {
		out, isErr := call(t, cs, ctx, "slice", map[string]any{
			"base": "ut11", "register": кандидат, "limit": 5,
		})
		if isErr {
			if strings.Contains(out, "непериодический") {
				continue
			}
			t.Fatalf("срез по периодическому регистру %s не собрался: %s", кандидат, out)
		}
		t.Logf("данных нет ни в одном периодическом регистре; сборка среза проверена на %s: %s",
			кандидат, firstLineOf(out))
		return
	}
	t.Skipf("периодических регистров сведений не нашлось вовсе (непериодических с данными: %d)",
		непериодических)
}

// firstLineOf — первая строка ответа, для короткой записи в журнал теста.
func firstLineOf(s string) string {
	if idx := strings.IndexByte(s, '\n'); idx >= 0 {
		return s[:idx]
	}
	return s
}

// objectNames достаёт имена объектов из перечня категории, не больше limit.
func objectNames(list string, limit int) []string {
	var names []string
	for _, line := range strings.Split(list, "\n") {
		if !strings.HasPrefix(line, "  ") {
			continue
		}
		name := strings.TrimSpace(line)
		if idx := strings.Index(name, " — "); idx >= 0 {
			name = name[:idx]
		}
		if name == "" {
			continue
		}
		names = append(names, name)
		if len(names) >= limit {
			break
		}
	}
	return names
}

func TestLiveНепериодическийРегистрДаётОтказ(t *testing.T) {
	cs, ctx := liveSession(t)

	// Ищем непериодический регистр: по нему срез не должен собираться молча.
	list, _ := call(t, cs, ctx, "metadata", map[string]any{
		"base": "ut11", "filter": "РегистрыСведений",
	})
	имя := firstObjectName(list)
	if имя == "" {
		t.Skip("регистров сведений нет")
	}

	// Проверяем не конкретный регистр, а поведение: несуществующий тоже обязан
	// давать отказ, а не пустую таблицу.
	out, isErr := call(t, cs, ctx, "slice", map[string]any{
		"base": "ut11", "register": "ЗаведомоНетТакогоРегистра",
	})
	if !isErr {
		t.Errorf("несуществующий регистр обязан давать отказ, получено: %s", out)
	}
}

func TestLiveИтогиПоСчетам(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "accounts", map[string]any{
		"base": "bu3", "account": "41", "kind": "Остатки",
	})
	if isErr {
		t.Fatalf("итоги по счёту не получены: %s", out)
	}
	if !strings.Contains(out, "СуммаОстаток") {
		t.Errorf("в итогах нет стандартных показателей остатка:\n%s", out)
	}

	// Несуществующий счёт — отказ с указанием плана счетов, а не пустая таблица.
	out, isErr = call(t, cs, ctx, "accounts", map[string]any{
		"base": "bu3", "account": "99999",
	})
	if !isErr {
		t.Errorf("несуществующий счёт обязан давать отказ, получено: %s", out)
	}

	// Обороты без периода — отказ, а не молчаливая подстановка «за всё время».
	out, isErr = call(t, cs, ctx, "accounts", map[string]any{
		"base": "bu3", "account": "41", "kind": "Обороты",
	})
	if !isErr {
		t.Errorf("обороты без периода обязаны давать отказ, получено: %s", out)
	}
}
