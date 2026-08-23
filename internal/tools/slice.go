package tools

import (
	"context"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/refusal"
)

// ── Срез регистра сведений ────────────────────────────────────────────────────

type SliceInput struct {
	Base       string         `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	Register   string         `json:"register" jsonschema:"Имя регистра сведений без префикса, например ЦеныНоменклатуры"`
	Kind       string         `json:"kind,omitempty" jsonschema:"СрезПоследних (умолчание) или СрезПервых"`
	Period     string         `json:"period,omitempty" jsonschema:"Дата среза, например 2026-06-30. Пусто — на текущий момент"`
	Where      string         `json:"where,omitempty" jsonschema:"Отбор по полям среза, без слова ГДЕ. Пример: Номенклатура = &Товар"`
	Parameters map[string]any `json:"parameters,omitempty" jsonschema:"Параметры отбора: ключ без амперсанда"`
	Limit      int            `json:"limit,omitempty" jsonschema:"Сколько строк вернуть (по умолчанию 100, максимум 1000)"`
}

func SliceTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "slice",
		Description: "Получить срез регистра сведений — значения, действующие на дату: цены " +
			"номенклатуры, курсы валют, ставки, настройки. Запрос собирается сервером: измерения " +
			"и ресурсы регистра подставляются сами, дата уходит параметром среза. Используй это " +
			"вместо запроса к самому регистру: выборка записей вернёт всю историю, и первая " +
			"попавшаяся строка легко сойдёт за действующее значение. Непериодический регистр " +
			"даёт отказ — у него среза не бывает.",
	}
}

func (s *Set) Slice(ctx context.Context, _ *mcp.CallToolRequest, in SliceInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Register) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "регистр не назван", "поле register обязательно")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	payload := map[string]any{"register": in.Register}
	for key, value := range map[string]string{
		"kind": in.Kind, "period": in.Period, "where": in.Where,
	} {
		if strings.TrimSpace(value) != "" {
			payload[key] = value
		}
	}
	if len(in.Parameters) > 0 {
		payload["parameters"] = in.Parameters
	}
	if in.Limit > 0 {
		payload["limit"] = in.Limit
	}

	var reply queryReply
	if err := client.Tell(ctx, "slice", payload, &reply); err != nil {
		return nil, nil, err
	}
	return text(renderTable(client.Base().Name, reply)), nil, nil
}

// ── Итоги по счетам ───────────────────────────────────────────────────────────

type AccountsInput struct {
	Base       string         `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	Account    string         `json:"account" jsonschema:"Код счёта, например 41 или 62.01"`
	Kind       string         `json:"kind,omitempty" jsonschema:"Остатки (умолчание), Обороты или ОстаткиИОбороты"`
	Period     string         `json:"period,omitempty" jsonschema:"Дата остатков для kind=Остатки. Пусто — на текущий момент"`
	Start      string         `json:"start,omitempty" jsonschema:"Начало периода для Обороты и ОстаткиИОбороты"`
	End        string         `json:"end,omitempty" jsonschema:"Конец периода для Обороты и ОстаткиИОбороты"`
	Register   string         `json:"register,omitempty" jsonschema:"Имя регистра бухгалтерии (по умолчанию Хозрасчетный)"`
	Resources  []string       `json:"resources,omitempty" jsonschema:"Показатели: СуммаОстатокДт, СуммаОборотДт и т.п. Пусто — стандартный набор для выбранного вида"`
	Parameters map[string]any `json:"parameters,omitempty" jsonschema:"Параметры отбора: ключ без амперсанда"`
	Limit      int            `json:"limit,omitempty" jsonschema:"Сколько строк вернуть (по умолчанию 100, максимум 1000)"`
}

func AccountsTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "accounts",
		Description: "Получить бухгалтерские итоги по счёту: остатки на дату (kind=Остатки), обороты " +
			"за период (kind=Обороты) или полную картину с начальным и конечным остатком " +
			"(kind=ОстаткиИОбороты). Счёт задаётся кодом — 41, 62.01, 51. Суммы приходят раздельно " +
			"по дебету и кредиту. Это для конфигураций с бухгалтерским учётом; складские и товарные " +
			"итоги живут в регистрах накопления, их берёт инструмент register.",
	}
}

func (s *Set) Accounts(ctx context.Context, _ *mcp.CallToolRequest, in AccountsInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Account) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "счёт не назван",
			"поле account обязательно: код счёта, например 41 или 62.01",
			"перечень счетов — metadata с filter=ПланыСчетов, затем object по нужному плану")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	payload := map[string]any{"account": in.Account}
	for key, value := range map[string]string{
		"kind": in.Kind, "period": in.Period, "start": in.Start,
		"end": in.End, "register": in.Register,
	} {
		if strings.TrimSpace(value) != "" {
			payload[key] = value
		}
	}
	if len(in.Resources) > 0 {
		payload["resources"] = in.Resources
	}
	if len(in.Parameters) > 0 {
		payload["parameters"] = in.Parameters
	}
	if in.Limit > 0 {
		payload["limit"] = in.Limit
	}

	var reply queryReply
	if err := client.Tell(ctx, "accounts", payload, &reply); err != nil {
		return nil, nil, err
	}
	return text(renderTable(client.Base().Name, reply)), nil, nil
}
