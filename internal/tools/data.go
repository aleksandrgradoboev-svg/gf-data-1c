package tools

import (
	"context"
	"encoding/json"
	"fmt"
	"net/url"
	"strconv"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/refusal"
)

// ── Проверка запроса ──────────────────────────────────────────────────────────

type QueryCheckInput struct {
	Base  string `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	Query string `json:"query" jsonschema:"Текст запроса на языке запросов 1С для проверки"`
}

func QueryCheckTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "query_check",
		Description: "Проверить синтаксис запроса 1С без выполнения — найдёт ошибки в ВЫБРАТЬ/SELECT " +
			"и покажет, какие колонки вернёт запрос. Вызывай перед query: разбор не обращается к " +
			"данным и стоит несоизмеримо дешевле самого запроса.",
	}
}

type checkReply struct {
	Колонки []string `json:"колонки"`
}

func (s *Set) QueryCheck(ctx context.Context, _ *mcp.CallToolRequest, in QueryCheckInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Query) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "текст запроса пуст", "поле query обязательно")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}
	var reply checkReply
	if err := client.Tell(ctx, "check", map[string]any{"query": in.Query}, &reply); err != nil {
		// Проверка запроса — то место, где подсказка нужнее всего: сюда приходят с черновиком.
		return nil, nil, EnrichQueryRefusal(err, in.Query, nil)
	}
	return text(fmt.Sprintf("Запрос разобран. Колонки (%d): %s",
		len(reply.Колонки), strings.Join(reply.Колонки, ", "))), nil, nil
}

// ── Запрос ────────────────────────────────────────────────────────────────────

type QueryInput struct {
	Base       string         `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	Query      string         `json:"query" jsonschema:"Текст запроса на языке запросов 1С. Только ВЫБРАТЬ/SELECT. Параметры через &ИмяПараметра"`
	Parameters map[string]any `json:"parameters,omitempty" jsonschema:"Параметры запроса: ключ без амперсанда. Даты строкой ГГГГ-ММ-ДД"`
	Limit      int            `json:"limit,omitempty" jsonschema:"Максимум строк результата (по умолчанию 100, максимум 1000)"`
	Offset     int            `json:"offset,omitempty" jsonschema:"Пропустить столько строк результата — следующая страница. Значение берите из строки «следующее смещение» предыдущего ответа. Для устойчивой разбивки запрос обязан содержать УПОРЯДОЧИТЬ"`
}

func QueryTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "query",
		Description: "Выполнить запрос на языке 1С (ВЫБРАТЬ/SELECT) и получить данные из базы: элементы " +
			"справочника, документы за период, остатки, обороты, сведения регистров. Имена таблиц в " +
			"единственном числе: Справочник.X, Документ.X, РегистрНакопления.X, РегистрСведений.X. " +
			"Перечисления таблицами не являются — используй ЗНАЧЕНИЕ(Перечисление.Имя.Значение). " +
			"Виртуальные таблицы: РегистрНакопления.X.Остатки(&Период), .Обороты(&Начало, &Конец), " +
			"РегистрСведений.X.СрезПоследних(&Период). Доступен ВЕСЬ язык запросов: пакет с " +
			"временными таблицами (ПОМЕСТИТЬ … ; … ; УНИЧТОЖИТЬ), соединения, вложенные запросы, " +
			"ИТОГИ ПО, объединения, регистры любого вида, включая расчёта и бухгалтерии с их " +
			"виртуальными таблицами. Параметр-ссылка задаётся объектом " +
			"{\"тип\": \"CatalogRef.Номенклатура\", \"идентификатор\": \"...\"}, значение перечисления — " +
			"{\"тип\": \"EnumRef.X\", \"значение\": \"ИмяЗначения\"}, список для «В (&П)» — массивом: " +
			"строка на месте ссылки даёт ноль строк или ошибку сравнения типов. Перечисляй нужные " +
			"поля вместо *: ячейки не сокращаются, и широкая выборка съедает ответ целиком. Имена " +
			"полей бери из object, синтаксис проверяй через query_check.",
	}
}

type queryReply struct {
	Колонки           []string         `json:"колонки"`
	Строк             int              `json:"строк"`
	ВсегоСтрок        int              `json:"всегоСтрок"`
	Смещение          int              `json:"смещение"`
	СледующееСмещение int              `json:"следующееСмещение"`
	ЕстьЕщё           bool             `json:"естьЕщё"`
	Обрезано          bool             `json:"обрезано"`
	Строки            []map[string]any `json:"строки"`
}

func (s *Set) Query(ctx context.Context, _ *mcp.CallToolRequest, in QueryInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Query) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "текст запроса пуст", "поле query обязательно")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	payload := map[string]any{"query": in.Query}
	if len(in.Parameters) > 0 {
		payload["parameters"] = in.Parameters
	}
	if in.Limit > 0 {
		payload["limit"] = in.Limit
	}
	if in.Offset > 0 {
		payload["offset"] = in.Offset
	}

	var reply queryReply
	if err := client.Tell(ctx, "query", payload, &reply); err != nil {
		// Отказ платформы точен, но односложен: к нему дописывается то, что известно про
		// виртуальные таблицы и язык запросов, — иначе вызывающий уходит угадывать.
		return nil, nil, EnrichQueryRefusal(err, in.Query, in.Parameters)
	}
	return text(renderTable(client.Base().Name, reply)), nil, nil
}

// ── Счёт записей ──────────────────────────────────────────────────────────────

type CountInput struct {
	Base       string         `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	Table      string         `json:"table" jsonschema:"Таблица 1С: Справочник.X, Документ.X, РегистрНакопления.X, РегистрСведений.X, РегистрБухгалтерии.X"`
	Where      string         `json:"where,omitempty" jsonschema:"Условие отбора без слова ГДЕ, например: Дата МЕЖДУ &Н И &К И НЕ ПометкаУдаления"`
	Parameters map[string]any `json:"parameters,omitempty" jsonschema:"Параметры условия: ключ без амперсанда"`
}

func CountTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "count",
		Description: "Посчитать записи таблицы 1С — есть ли вообще данные, сколько документов за период, " +
			"сколько элементов справочника. Отбор задаётся условием where с параметрами &Имя. Дешевле " +
			"и надёжнее полного запроса: текст собирается сервером, поэтому счёт нельзя сочинить.",
	}
}

type countReply struct {
	Таблица string `json:"таблица"`
	Отбор   string `json:"отбор"`
	Записей int    `json:"записей"`
}

func (s *Set) Count(ctx context.Context, _ *mcp.CallToolRequest, in CountInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Table) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "таблица не названа", "поле table обязательно")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	payload := map[string]any{"table": in.Table}
	if in.Where != "" {
		payload["where"] = in.Where
	}
	if len(in.Parameters) > 0 {
		payload["parameters"] = in.Parameters
	}

	var reply countReply
	if err := client.Tell(ctx, "count", payload, &reply); err != nil {
		return nil, nil, err
	}

	var b strings.Builder
	fmt.Fprintf(&b, "%s, база %s: записей %d", reply.Таблица, client.Base().Name, reply.Записей)
	if reply.Отбор != "" {
		fmt.Fprintf(&b, "\nОтбор: %s", reply.Отбор)
	}
	return text(b.String()), nil, nil
}

// ── Итоги регистра ────────────────────────────────────────────────────────────

type RegisterInput struct {
	Base       string   `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	Register   string   `json:"register" jsonschema:"Имя регистра накопления без префикса, например ТоварыНаСкладах"`
	Kind       string   `json:"kind,omitempty" jsonschema:"Какая виртуальная таблица нужна: Остатки (умолчание), Обороты, ОстаткиИОбороты"`
	Period     string   `json:"period,omitempty" jsonschema:"Дата остатков для kind=Остатки, например 2026-06-30. Пусто — на текущий момент"`
	Start      string   `json:"start,omitempty" jsonschema:"Начало периода для Обороты и ОстаткиИОбороты"`
	End        string   `json:"end,omitempty" jsonschema:"Конец периода для Обороты и ОстаткиИОбороты"`
	Dimensions []string `json:"dimensions,omitempty" jsonschema:"Измерения-разрезы для группировки. Пусто — итог одной строкой"`
	// Поле намеренно необязательное в схеме: пустые resources — частая ошибка, и отвечать
	// на неё должен наш отказ с подсказкой, где взять имена, а не сухое «missing properties».
	Resources  []string       `json:"resources,omitempty" jsonschema:"Ресурсы виртуальной таблицы: КоличествоОстаток для остатков, КоличествоОборот для оборотов. Имена берутся из object"`
	Where      string         `json:"where,omitempty" jsonschema:"Дополнительный отбор по полям виртуальной таблицы, без слова ГДЕ"`
	Parameters map[string]any `json:"parameters,omitempty" jsonschema:"Параметры отбора: ключ без амперсанда"`
	Limit      int            `json:"limit,omitempty" jsonschema:"Сколько строк вернуть (по умолчанию 100, максимум 1000)"`
}

func RegisterTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "register",
		Description: "Получить итоги регистра накопления через виртуальные таблицы: остатки на дату " +
			"(kind=Остатки), обороты за период (kind=Обороты) или и то и другое. Запрос собирается " +
			"сервером — имя виртуальной таблицы, порядок границ периода и группировка не сочиняются " +
			"заново. Разрезы перечисляются в dimensions, показатели в resources; имена ресурсов " +
			"виртуальной таблицы отличаются от имён регистра (ВНаличии → ВНаличииОстаток) и берутся " +
			"из object.",
	}
}

func (s *Set) Register(ctx context.Context, _ *mcp.CallToolRequest, in RegisterInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Register) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "регистр не назван", "поле register обязательно")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	payload := map[string]any{"register": in.Register}
	for key, value := range map[string]string{
		"kind": in.Kind, "period": in.Period, "start": in.Start, "end": in.End, "where": in.Where,
	} {
		if strings.TrimSpace(value) != "" {
			payload[key] = value
		}
	}
	if len(in.Dimensions) > 0 {
		payload["dimensions"] = in.Dimensions
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
	if err := client.Tell(ctx, "register", payload, &reply); err != nil {
		return nil, nil, err
	}
	return text(renderTable(client.Base().Name, reply)), nil, nil
}

// ── Журнал регистрации ────────────────────────────────────────────────────────

type EventLogInput struct {
	Base      string `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	StartDate string `json:"start_date,omitempty" jsonschema:"Начало периода в формате ISO 8601, например 2026-03-01T00:00:00"`
	EndDate   string `json:"end_date,omitempty" jsonschema:"Конец периода в формате ISO 8601"`
	Level     string `json:"level,omitempty" jsonschema:"Уровень важности: Ошибка, Предупреждение, Информация, Примечание"`
	User      string `json:"user,omitempty" jsonschema:"Имя пользователя 1С для фильтрации"`
	Limit     int    `json:"limit,omitempty" jsonschema:"Максимум записей (по умолчанию 50, максимум 500)"`
}

func EventLogTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "eventlog",
		Description: "Прочитать журнал регистрации 1С — ошибки, действия пользователей и системные " +
			"события. Фильтрация по периоду, уровню важности (Ошибка, Предупреждение, Информация, " +
			"Примечание) и пользователю.",
	}
}

type eventLogReply struct {
	Записей int `json:"записей"`
	Предел  int `json:"предел"`
	Записи  []struct {
		Дата         string `json:"дата"`
		Уровень      string `json:"уровень"`
		Пользователь string `json:"пользователь"`
		Событие      string `json:"событие"`
		Комментарий  string `json:"комментарий"`
	} `json:"записи"`
}

func (s *Set) EventLog(ctx context.Context, _ *mcp.CallToolRequest, in EventLogInput) (*mcp.CallToolResult, any, error) {
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	query := url.Values{}
	for key, value := range map[string]string{
		"start": in.StartDate, "end": in.EndDate, "level": in.Level, "user": in.User,
	} {
		if strings.TrimSpace(value) != "" {
			query.Set(key, value)
		}
	}
	if in.Limit > 0 {
		query.Set("limit", strconv.Itoa(in.Limit))
	}

	var reply eventLogReply
	if err := client.Ask(ctx, "eventlog", query, &reply); err != nil {
		return nil, nil, err
	}

	var b strings.Builder
	fmt.Fprintf(&b, "Журнал регистрации базы %s: записей %d (предел %d)\n\n",
		client.Base().Name, reply.Записей, reply.Предел)
	for _, rec := range reply.Записи {
		fmt.Fprintf(&b, "%s  %-14s %-16s %s\n", rec.Дата, rec.Уровень, rec.Пользователь, rec.Событие)
		if strings.TrimSpace(rec.Комментарий) != "" {
			fmt.Fprintf(&b, "    %s\n", firstLine(rec.Комментарий))
		}
	}
	if reply.Записей == reply.Предел {
		b.WriteString("\nВыдача упёрлась в предел: записей может быть больше — сузьте период или поднимите limit.")
	}
	return text(b.String()), nil, nil
}

// ── Печать таблицы ────────────────────────────────────────────────────────────

// renderTable печатает результат так, чтобы обрезание было видно.
//
// Молчаливое усечение — главная ложь табличного вывода: строк стало меньше,
// а выглядит как полный ответ.
func renderTable(base string, reply queryReply) string {
	var b strings.Builder
	fmt.Fprintf(&b, "База %s: строк %d", base, reply.Строк)
	if reply.Смещение > 0 {
		fmt.Fprintf(&b, ", начиная с %d", reply.Смещение)
	}
	if reply.ЕстьЕщё {
		fmt.Fprintf(&b, " из %d — показана часть", reply.ВсегоСтрок)
	}
	b.WriteString("\n\n")

	if len(reply.Строки) == 0 {
		b.WriteString("Результат пуст. Это ответ базы, а не отказ канала: запрос выполнен и вернул ноль строк.")
		return b.String()
	}

	for i, row := range reply.Строки {
		fmt.Fprintf(&b, "%d.", i+1)
		for _, col := range reply.Колонки {
			fmt.Fprintf(&b, "  %s = %s", col, renderValue(row[col]))
		}
		b.WriteString("\n")
	}
	if reply.ЕстьЕщё {
		fmt.Fprintf(&b, "\nПоказано %d из %d строк. Следующая страница: offset=%d "+
			"(для устойчивой разбивки запрос должен содержать УПОРЯДОЧИТЬ). "+
			"Весь результат целиком — инструмент export.",
			reply.Строк, reply.ВсегоСтрок, reply.СледующееСмещение)
	}
	return b.String()
}

// renderValue печатает значение ячейки. Ссылка разворачивается в «представление
// (тип, идентификатор)»: одного представления мало, по нему нельзя отобрать.
func renderValue(value any) string {
	switch v := value.(type) {
	case nil:
		return "—"
	case string:
		return v
	case float64:
		if v == float64(int64(v)) {
			return strconv.FormatInt(int64(v), 10)
		}
		return strconv.FormatFloat(v, 'f', -1, 64)
	case bool:
		if v {
			return "да"
		}
		return "нет"
	case map[string]any:
		представление, _ := v["представление"].(string)
		тип, _ := v["тип"].(string)
		идентификатор, _ := v["идентификатор"].(string)
		if тип == "" {
			return представление
		}
		return fmt.Sprintf("%s (%s, %s)", представление, тип, идентификатор)
	default:
		data, err := json.Marshal(v)
		if err != nil {
			return fmt.Sprint(v)
		}
		return string(data)
	}
}

func firstLine(s string) string {
	if idx := strings.IndexAny(s, "\r\n"); idx >= 0 {
		s = s[:idx]
	}
	if len(s) > 200 {
		return s[:200] + "…"
	}
	return s
}
