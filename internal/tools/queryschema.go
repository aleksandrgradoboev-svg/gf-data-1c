package tools

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/google/jsonschema-go/jsonschema"
	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/refusal"
)

// ── Разбор запроса в структуру ────────────────────────────────────────────────

type QueryParseInput struct {
	Base  string `json:"base" jsonschema:"Имя базы 1С из реестра. Обязательно; перечень — bases с action=list"`
	Query string `json:"query" jsonschema:"Текст запроса на языке запросов 1С, который нужно разобрать"`
}

func QueryParseTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "query_parse",
		Description: "Разобрать запрос 1С в структуру: какие таблицы он читает, какими полями, с какими " +
			"соединениями, отборами, группировками и параметрами. Отвечает на вопрос «как устроен этот " +
			"запрос» — в отличие от query_check, который отвечает «правилен ли текст». Вызывай, когда " +
			"нужно понять чужой запрос из модуля конфигурации, не вычитывая его глазами: пакет " +
			"раскладывается по запросам, временные таблицы помечаются, вложенные запросы раскрываются, " +
			"параметры виртуальных таблиц (период, условие) вытаскиваются отдельно. Разбор выполняет " +
			"платформа 1С той же базы, поэтому несуществующие таблицы и поля отвергаются с указанием места.",
	}
}

// parseReply повторяет состав ответа расширения. Поле «запросы» остаётся сырым JSON:
// его структура рекурсивна (вложенные запросы), и пересобирать её в Go-типы — значит
// молча потерять то, чего в типах не предусмотрели.
type parseReply struct {
	ЗапросовВПакете  int             `json:"запросовВПакете"`
	Таблицы          []string        `json:"таблицы"`
	ВременныеТаблицы []string        `json:"временныеТаблицы"`
	Параметры        []string        `json:"параметры"`
	Запросы          json.RawMessage `json:"запросы"`
}

func (s *Set) QueryParse(ctx context.Context, _ *mcp.CallToolRequest, in QueryParseInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Query) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "текст запроса пуст", "поле query обязательно")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}
	var reply parseReply
	if err := client.Tell(ctx, "parse", map[string]any{"query": in.Query}, &reply); err != nil {
		// Сюда приходят с чужим текстом: подсказка о виртуальных таблицах и языке
		// нужна не меньше, чем при проверке своего.
		return nil, nil, EnrichQueryRefusal(err, in.Query, nil)
	}

	var sb strings.Builder
	fmt.Fprintf(&sb, "Запрос разобран: запросов в пакете %d.\n", reply.ЗапросовВПакете)
	if len(reply.Таблицы) > 0 {
		fmt.Fprintf(&sb, "Таблицы базы (%d): %s\n", len(reply.Таблицы), strings.Join(reply.Таблицы, ", "))
	}
	if len(reply.ВременныеТаблицы) > 0 {
		fmt.Fprintf(&sb, "Временные таблицы: %s\n", strings.Join(reply.ВременныеТаблицы, ", "))
	}
	if len(reply.Параметры) > 0 {
		fmt.Fprintf(&sb, "Параметры: %s\n", strings.Join(reply.Параметры, ", "))
	}
	sb.WriteString("\nУстройство по запросам пакета:\n")
	sb.WriteString(indentJSON(reply.Запросы))
	return text(sb.String()), nil, nil
}

// indentJSON перепечатывает сырой JSON с отступами. Расширение отдаёт его одной строкой,
// а читать структуру пакета предстоит модели — ей отступы и нужны.
func indentJSON(raw json.RawMessage) string {
	var out bytes.Buffer
	if err := json.Indent(&out, raw, "", "  "); err != nil {
		return string(raw)
	}
	return out.String()
}

// ── Сборка запроса из структуры ───────────────────────────────────────────────

type QueryBuildField struct {
	Поле      string `json:"поле,omitempty" jsonschema:"Имя поля источника, можно через точку: Номенклатура, Ссылка.Дата"`
	Функция   string `json:"функция,omitempty" jsonschema:"Агрегатная функция над полем: СУММА, КОЛИЧЕСТВО, МАКСИМУМ, МИНИМУМ, СРЕДНЕЕ"`
	Выражение string `json:"выражение,omitempty" jsonschema:"Готовое выражение вместо пары функция+поле: ВЫБОР КОГДА … КОНЕЦ, арифметика. Псевдоним таблицы указывать явно"`
	Как       string `json:"как,omitempty" jsonschema:"Псевдоним колонки результата"`
}

// UnmarshalJSON принимает колонку и строкой, и объектом. Строка — самый частый вид
// («Ссылка», «Номенклатура.Наименование»), и модели пишут её строкой, что бы ни
// говорило описание: прогон 27.08.2026 в Kilo дал отказ валидации «want object» на
// каждой попытке. Схема объявляет обе формы (см. QueryBuildTool), а здесь они сходятся.
func (f *QueryBuildField) UnmarshalJSON(b []byte) error {
	trimmed := bytes.TrimSpace(b)
	if len(trimmed) > 0 && trimmed[0] == '"' {
		var name string
		if err := json.Unmarshal(trimmed, &name); err != nil {
			return err
		}
		*f = QueryBuildField{Поле: name}
		return nil
	}
	// Псевдоним типа без методов — иначе Unmarshal позвал бы этот же метод по кругу.
	type plain QueryBuildField
	var obj plain
	if err := json.Unmarshal(trimmed, &obj); err != nil {
		return err
	}
	*f = QueryBuildField(obj)
	return nil
}

type QueryBuildInput struct {
	Base             string            `json:"base" jsonschema:"Имя базы 1С из реестра. Обязательно; перечень — bases с action=list"`
	Источник         string            `json:"источник" jsonschema:"Таблица-источник: Справочник.Номенклатура, Документ.РеализацияТоваровУслуг.Товары, РегистрНакопления.ТоварыНаСкладах.Остатки"`
	Псевдоним        string            `json:"псевдоним,omitempty" jsonschema:"Псевдоним источника в запросе. По умолчанию — последняя часть имени таблицы"`
	ПараметрыТаблицы []string          `json:"параметрыТаблицы,omitempty" jsonschema:"Параметры виртуальной таблицы по позициям, как в скобках: для Остатки — [\"&НаДату\", \"Склад = &Склад\"]. Пустая строка пропускает позицию"`
	Поля             []QueryBuildField `json:"поля" jsonschema:"Колонки результата. Каждая — либо строка с именем поля («Ссылка», «Номенклатура.Наименование»), либо объект {поле, функция, выражение, как} для агрегата или псевдонима"`
	Отбор            []string          `json:"отбор,omitempty" jsonschema:"Условия ГДЕ, по одному на элемент: Ссылка.Проведен, Дата МЕЖДУ &Начало И &Конец. Соединяются через И"`
	Группировка      []string          `json:"группировка,omitempty" jsonschema:"Поля СГРУППИРОВАТЬ ПО"`
	Порядок          []string          `json:"порядок,omitempty" jsonschema:"Поля УПОРЯДОЧИТЬ ПО — псевдонимы колонок результата, не выражения источника"`
	Различные        bool              `json:"различные,omitempty" jsonschema:"ВЫБРАТЬ РАЗЛИЧНЫЕ"`
	Первые           int               `json:"первые,omitempty" jsonschema:"ВЫБРАТЬ ПЕРВЫЕ N"`
}

func QueryBuildTool() *mcp.Tool {
	return &mcp.Tool{
		Name:        "query_build",
		InputSchema: queryBuildSchema(),
		Description: "Собрать текст запроса 1С из структуры — без единой запятой синтаксиса. Ты называешь " +
			"источник, поля, отбор и группировку; текст пишет платформа 1С, и она же проверяет каждое имя " +
			"в момент добавления: несуществующая таблица или поле отвергаются сразу, с указанием виновного. " +
			"Используй вместо ручного сочинения текста, когда запрос простой (один источник, агрегаты, " +
			"отбор): ошибиться синтаксисом здесь нельзя. Для виртуальной таблицы регистра параметры " +
			"(период, условие) передавай в параметрыТаблицы по позициям. Собранный текст выполняй " +
			"инструментом query; сложные запросы с соединениями пиши текстом сразу — этот инструмент " +
			"собирает один оператор по одному источнику.",
	}
}

// queryBuildSchema — схема ввода, выведенная из структуры и поправленная в одном месте:
// элемент «поля» допускает строку ИЛИ объект. Выведенная схема знает только объект, и SDK
// отвергал строку до вызова обработчика — модель получала «want object» и начинала
// сочинять текст запроса руками, ради чего построитель и заведён.
func queryBuildSchema() *jsonschema.Schema {
	s, err := jsonschema.For[QueryBuildInput](nil)
	if err != nil {
		panic(fmt.Errorf("query_build: схема ввода не выведена: %w", err))
	}
	fields, ok := s.Properties["поля"]
	if !ok || fields.Items == nil {
		panic("query_build: в схеме нет массива «поля»")
	}
	object := fields.Items
	object.Description = "Колонка с агрегатом, выражением или псевдонимом"
	fields.Items = &jsonschema.Schema{
		AnyOf: []*jsonschema.Schema{
			{Type: "string", Description: "Имя поля источника, можно через точку: Ссылка, Номенклатура.Наименование"},
			object,
		},
	}
	return s
}

type buildReply struct {
	Запрос    string   `json:"запрос"`
	Параметры []string `json:"параметры"`
}

func (s *Set) QueryBuild(ctx context.Context, _ *mcp.CallToolRequest, in QueryBuildInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Источник) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "источник не назван",
			"поле «источник» обязательно: имя таблицы вида Справочник.Номенклатура")
	}
	if len(in.Поля) == 0 {
		return nil, nil, refusal.New(refusal.BadRequest, "поля не заданы",
			"поле «поля» обязательно: хотя бы одна колонка результата")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	// Поля перекладываются в тот вид, который ждёт расширение: строка для простого
	// имени, объект для функции или псевдонима. Строкой короче, но схема одна.
	fields := make([]any, 0, len(in.Поля))
	for _, f := range in.Поля {
		if f.Функция == "" && f.Выражение == "" && f.Как == "" {
			fields = append(fields, f.Поле)
			continue
		}
		obj := map[string]any{}
		if f.Поле != "" {
			obj["поле"] = f.Поле
		}
		if f.Функция != "" {
			obj["функция"] = f.Функция
		}
		if f.Выражение != "" {
			obj["выражение"] = f.Выражение
		}
		if f.Как != "" {
			obj["как"] = f.Как
		}
		fields = append(fields, obj)
	}

	payload := map[string]any{
		"источник": in.Источник,
		"поля":     fields,
	}
	if in.Псевдоним != "" {
		payload["псевдоним"] = in.Псевдоним
	}
	if len(in.ПараметрыТаблицы) > 0 {
		payload["параметрыТаблицы"] = in.ПараметрыТаблицы
	}
	if len(in.Отбор) > 0 {
		payload["отбор"] = in.Отбор
	}
	if len(in.Группировка) > 0 {
		payload["группировка"] = in.Группировка
	}
	if len(in.Порядок) > 0 {
		payload["порядок"] = in.Порядок
	}
	if in.Различные {
		payload["различные"] = true
	}
	if in.Первые > 0 {
		payload["первые"] = in.Первые
	}

	var reply buildReply
	if err := client.Tell(ctx, "build", payload, &reply); err != nil {
		return nil, nil, EnrichQueryRefusal(err, in.Источник, nil)
	}

	var sb strings.Builder
	sb.WriteString("Запрос собран и проверен платформой:\n\n")
	sb.WriteString(reply.Запрос)
	if len(reply.Параметры) > 0 {
		fmt.Fprintf(&sb, "\n\nПараметры к заполнению при выполнении: %s",
			strings.Join(reply.Параметры, ", "))
	}
	sb.WriteString("\n\nВыполнить — инструмент query с этим текстом.")
	return text(sb.String()), nil, nil
}
