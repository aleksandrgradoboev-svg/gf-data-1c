package server_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// dataTools — инструменты, работающие с данными названной базы. Список полный:
// добавили инструмент — добавьте сюда, иначе он тихо разрешит вызов без базы.
var dataTools = []string{
	"base_info", "metadata", "object", "count", "query", "query_check",
	"register", "slice", "accounts", "eventlog", "export",
	"query_parse", "query_build",
}

// TestBaseОбъявленОбязательным проверяет САМУ СХЕМУ, а не поведение обработчика:
// клиент видит обязательность до вызова, и слабая модель не отправляет запрос,
// который заведомо откажут.
func TestBaseОбъявленОбязательным(t *testing.T) {
	cs, ctx := connect(t)
	res, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("список инструментов не получен: %v", err)
	}
	seen := map[string]bool{}
	for _, tool := range res.Tools {
		seen[tool.Name] = true
		required := requiredFields(t, tool.Name, tool.InputSchema)
		switch tool.Name {
		case "probe":
			// Исключение по устройству: пустой base у пробы значит «все базы»,
			// а не «выбери за меня».
			if required["base"] {
				t.Errorf("probe: base не должен быть обязательным — там он означает «проверить все»")
			}
		case "bases", "syntax":
			// Реестр и справка вендора к базе не обращаются.
		default:
			if !required["base"] {
				t.Errorf("%s: base обязан быть required в схеме", tool.Name)
			}
		}
	}
	for _, name := range dataTools {
		if !seen[name] {
			t.Errorf("инструмент %q не объявлен — список dataTools разошёлся с сервером", name)
		}
	}
}

// TestВызовБезБазыОтклоняется — вторая половина контракта. Схему клиент может и
// проигнорировать, поэтому важно, что вызов без base не проходит НИ на каком уровне:
// SDK отбивает его валидацией схемы, а если бы дошло до обработчика — отказал бы реестр
// (см. TestResolveБезИмени в пакете registry).
func TestВызовБезБазыОтклоняется(t *testing.T) {
	cs, ctx := connect(t)
	call(t, cs, ctx, "bases", map[string]any{
		"action": "add", "name": "ut11", "url": "http://127.0.0.1:1/data",
	})
	for _, tool := range dataTools {
		_, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: tool, Arguments: map[string]any{}})
		if err == nil {
			t.Errorf("%s: вызов без base прошёл — база выбрана за вызывающего", tool)
			continue
		}
		if !strings.Contains(err.Error(), "base") {
			t.Errorf("%s: отклонение должно называть параметр base, получено: %v", tool, err)
		}
	}
}

// TestЕдинственнаяБазаНеСтановитсяУмолчанием — поблажка «если база одна, бери её» вернула
// бы умолчание сама собой в тот день, когда баз останется одна.
func TestЕдинственнаяБазаНеСтановитсяУмолчанием(t *testing.T) {
	cs, ctx := connect(t)
	call(t, cs, ctx, "bases", map[string]any{
		"action": "add", "name": "единственная", "url": "http://127.0.0.1:1/data",
	})
	if _, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name: "metadata", Arguments: map[string]any{},
	}); err == nil {
		t.Error("единственная база не отменяет требования назвать её")
	}
}

// TestSetDefaultУбранВнятно — старый вызов обязан объяснить, что механизма больше нет,
// а не выглядеть опечаткой в имени действия.
func TestSetDefaultУбранВнятно(t *testing.T) {
	cs, ctx := connect(t)
	call(t, cs, ctx, "bases", map[string]any{
		"action": "add", "name": "ut11", "url": "http://127.0.0.1:1/data",
	})
	out, isErr := call(t, cs, ctx, "bases", map[string]any{"action": "set_default", "name": "ut11"})
	if !isErr {
		t.Fatalf("set_default обязан отказывать, получено: %s", out)
	}
	if !strings.Contains(out, "базы по умолчанию больше нет") {
		t.Errorf("отказ должен объяснить, что механизм убран, получено: %s", out)
	}
}

// TestСписокБазНеОбещаетУмолчания — стрелка «база по умолчанию» из вывода убрана.
func TestСписокБазНеОбещаетУмолчания(t *testing.T) {
	cs, ctx := connect(t)
	call(t, cs, ctx, "bases", map[string]any{
		"action": "add", "name": "ut11", "url": "http://127.0.0.1:1/data",
	})
	out, _ := call(t, cs, ctx, "bases", map[string]any{"action": "list"})
	if strings.Contains(out, "→") {
		t.Errorf("в списке баз не должно быть стрелки умолчания, получено: %s", out)
	}
	if !strings.Contains(out, "Базы по умолчанию нет") {
		t.Errorf("список должен прямо говорить, что умолчания нет, получено: %s", out)
	}
}

// TestОтказНазываетБазу — отказ, не назвавший базу, читается как факт о конфигурации.
func TestОтказНазываетБазу(t *testing.T) {
	cs, ctx := connect(t)
	call(t, cs, ctx, "bases", map[string]any{
		"action": "add", "name": "ut11", "url": "http://127.0.0.1:1/data",
	})
	// Адрес заведомо мёртвый: важен не вид отказа, а то, что база в нём названа.
	out, isErr := call(t, cs, ctx, "base_info", map[string]any{"base": "ut11"})
	if !isErr {
		t.Fatalf("мёртвый канал обязан быть отказом, получено: %s", out)
	}
	if !strings.Contains(out, "ОТКАЗ (база ut11)") {
		t.Errorf("отказ обязан называть базу в первой строке, получено: %s", out)
	}
}

// requiredFields достаёт список обязательных полей из схемы инструмента. Схема приходит
// как any (её форму задаёт SDK), поэтому разбираем через JSON — так тест не зависит от
// того, каким типом SDK её положил.
func requiredFields(t *testing.T, tool string, schema any) map[string]bool {
	t.Helper()
	out := map[string]bool{}
	if schema == nil {
		return out
	}
	raw, err := json.Marshal(schema)
	if err != nil {
		t.Fatalf("%s: схема не сериализована: %v", tool, err)
	}
	var parsed struct {
		Required []string `json:"required"`
	}
	if err := json.Unmarshal(raw, &parsed); err != nil {
		t.Fatalf("%s: схема не разобрана: %v", tool, err)
	}
	for _, name := range parsed.Required {
		out[name] = true
	}
	return out
}
