package server_test

import (
	"context"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/server"
)

// Сквозной прогон против живого стенда.
//
// Пропускается, если стенд не поднят: тест обязан молчать на машине без базы,
// но обязан падать там, где база есть и ответила не тем.
const (
	liveURL  = "http://localhost:8081/ut11/hs/gt-data"
	liveUser = "agent"
	livePass = "111"
)

func liveSession(t *testing.T) (*mcp.ClientSession, context.Context) {
	t.Helper()
	if os.Getenv("GT_SKIP_LIVE") != "" {
		t.Skip("живой прогон отключён переменной GT_SKIP_LIVE")
	}
	client := &http.Client{Timeout: 5 * time.Second}
	req, _ := http.NewRequest(http.MethodGet, liveURL+"/version", nil)
	req.SetBasicAuth(liveUser, livePass)
	resp, err := client.Do(req)
	if err != nil {
		t.Skipf("стенд не отвечает (%v) — живой прогон пропущен", err)
	}
	resp.Body.Close()

	ctx := context.Background()
	regPath := filepath.Join(t.TempDir(), "bases.json")
	srv := server.New(server.Options{RegistryPath: regPath, Timeout: 60 * time.Second})

	clientTransport, serverTransport := mcp.NewInMemoryTransports()
	if _, err := srv.Connect(ctx, serverTransport, nil); err != nil {
		t.Fatalf("сервер не поднялся: %v", err)
	}
	mcpClient := mcp.NewClient(&mcp.Implementation{Name: "live-test"}, nil)
	cs, err := mcpClient.Connect(ctx, clientTransport, nil)
	if err != nil {
		t.Fatalf("клиент не подключился: %v", err)
	}
	t.Cleanup(func() { cs.Close() })

	// Регистрируются обе базы стенда: часть типов метаданных есть только в бухгалтерии
	// (планы счетов, регистры бухгалтерии), и на одной УТ полнота не проверяется.
	for _, base := range []struct{ name, url, title string }{
		{"ut11", liveURL, "УТ 11 — стенд"},
		{"bu3", strings.Replace(liveURL, "/ut11/", "/bu3/", 1), "БП 3.0 — стенд"},
	} {
		out, isErr := call(t, cs, ctx, "bases", map[string]any{
			"action": "add", "name": base.name, "url": base.url,
			"user": liveUser, "password": livePass, "title": base.title,
		})
		if isErr {
			t.Fatalf("база %s не зарегистрирована: %s", base.name, out)
		}
	}
	return cs, ctx
}

func TestLiveКаналЖивИНазываетКонфигурацию(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "probe", nil)
	if isErr || !strings.Contains(out, "жив") {
		t.Fatalf("проба не подтвердила канал: %s", out)
	}

	out, isErr = call(t, cs, ctx, "base_info", map[string]any{"base": "ut11"})
	if isErr {
		t.Fatalf("паспорт базы не получен: %s", out)
	}
	for _, want := range []string{"УправлениеТорговлей", "11.5.12", "8.3.27"} {
		if !strings.Contains(out, want) {
			t.Errorf("в паспорте базы нет %q:\n%s", want, out)
		}
	}
}

func TestLiveСоставКонфигурацииИСтруктураОбъекта(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "metadata", map[string]any{"base": "ut11"})
	if isErr || !strings.Contains(out, "Справочники") {
		t.Fatalf("сводка метаданных не получена: %s", out)
	}

	out, isErr = call(t, cs, ctx, "object", map[string]any{"base": "ut11",
		"object_type": "Catalog", "object_name": "Номенклатура",
	})
	if isErr {
		t.Fatalf("структура объекта не получена: %s", out)
	}
	// Тип обязан быть пригоден для запроса, а не описан прозой.
	if !strings.Contains(out, "CatalogRef.") {
		t.Errorf("типы реквизитов не в запросной нотации:\n%s", out)
	}
}

func TestLiveСчётСходитсяСЗапросом(t *testing.T) {
	cs, ctx := liveSession(t)

	viaCount, isErr := call(t, cs, ctx, "count", map[string]any{"base": "ut11", "table": "Справочник.Номенклатура"})
	if isErr {
		t.Fatalf("счёт не выполнен: %s", viaCount)
	}
	viaQuery, isErr := call(t, cs, ctx, "query", map[string]any{"base": "ut11",
		"query": "ВЫБРАТЬ КОЛИЧЕСТВО(*) КАК Всего ИЗ Справочник.Номенклатура",
	})
	if isErr {
		t.Fatalf("запрос не выполнен: %s", viaQuery)
	}

	number := extractNumber(viaCount, "записей ")
	if number == "" || !strings.Contains(viaQuery, "Всего = "+number) {
		t.Errorf("счёт и запрос разошлись:\ncount: %s\nquery: %s", viaCount, viaQuery)
	}
}

func TestLiveЗаписьОтклоняется(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "query", map[string]any{"base": "ut11",
		"query": "УДАЛИТЬ ИЗ Справочник.Номенклатура",
	})
	if !isErr {
		t.Fatalf("запрос на изменение обязан быть отклонён, получено: %s", out)
	}
	if !strings.Contains(out, "только читает") {
		t.Errorf("отказ не называет причину:\n%s", out)
	}
}

func TestLiveПустойРезультатНеПутаетсяСОтказом(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "query", map[string]any{"base": "ut11",
		"query":      "ВЫБРАТЬ Ссылка КАК Ссылка ИЗ Справочник.Номенклатура ГДЕ Наименование = &Имя",
		"parameters": map[string]any{"Имя": "такой номенклатуры заведомо нет"},
	})
	if isErr {
		t.Fatalf("пустая выборка не должна быть отказом: %s", out)
	}
	if !strings.Contains(out, "ответ базы, а не отказ канала") {
		t.Errorf("пустой результат не отличён от отказа явно:\n%s", out)
	}
}

func TestLiveИтогиРегистра(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "register", map[string]any{"base": "ut11",
		"register": "ТоварыНаСкладах", "kind": "Остатки",
		"dimensions": []string{"Склад"}, "resources": []string{"ВНаличииОстаток"},
	})
	if isErr {
		t.Fatalf("итоги регистра не получены: %s", out)
	}
	if !strings.Contains(out, "ВНаличииОстаток") {
		t.Errorf("в итогах нет запрошенного показателя:\n%s", out)
	}

	// Без ресурсов сервер обязан не гадать, а сказать, где взять имена.
	out, isErr = call(t, cs, ctx, "register", map[string]any{"base": "ut11", "register": "ТоварыНаСкладах"})
	if !isErr {
		t.Fatalf("вызов без resources обязан быть отказом: %s", out)
	}
}

// extractNumber достаёт число, идущее сразу после метки.
func extractNumber(text, label string) string {
	idx := strings.Index(text, label)
	if idx < 0 {
		return ""
	}
	rest := text[idx+len(label):]
	end := strings.IndexFunc(rest, func(r rune) bool { return r < '0' || r > '9' })
	if end < 0 {
		end = len(rest)
	}
	return rest[:end]
}

var _ = json.Marshal
