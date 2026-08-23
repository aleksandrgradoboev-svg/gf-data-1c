package tools

import (
	"context"
	"fmt"
	"net/url"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/refusal"
)

// ── Паспорт базы ──────────────────────────────────────────────────────────────

type BaseInfoInput struct {
	Base string `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра (см. инструмент bases). Опущено — база по умолчанию"`
}

func BaseInfoTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "base_info",
		Description: "Получить общую информацию о базе 1С: название конфигурации, версия, поставщик, " +
			"платформа, режим совместимости. Используй первым делом, чтобы понять, с какой " +
			"конфигурацией работаешь.",
	}
}

type baseInfoReply struct {
	Конфигурация       string `json:"конфигурация"`
	Синоним            string `json:"синоним"`
	Версия             string `json:"версия"`
	Поставщик          string `json:"поставщик"`
	Платформа          string `json:"платформа"`
	РежимСовместимости string `json:"режимСовместимости"`
}

func (s *Set) BaseInfo(ctx context.Context, _ *mcp.CallToolRequest, in BaseInfoInput) (*mcp.CallToolResult, any, error) {
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}
	var reply baseInfoReply
	if err := client.Ask(ctx, "base", nil, &reply); err != nil {
		return nil, nil, err
	}

	var b strings.Builder
	fmt.Fprintf(&b, "База: %s\n", client.Base().Name)
	fmt.Fprintf(&b, "Конфигурация: %s (%s)\n", reply.Конфигурация, reply.Синоним)
	fmt.Fprintf(&b, "Версия: %s\n", reply.Версия)
	fmt.Fprintf(&b, "Поставщик: %s\n", reply.Поставщик)
	fmt.Fprintf(&b, "Платформа: %s\n", reply.Платформа)
	fmt.Fprintf(&b, "Режим совместимости: %s", reply.РежимСовместимости)
	return text(b.String()), nil, nil
}

// ── Состав конфигурации ───────────────────────────────────────────────────────

type MetadataInput struct {
	Base   string `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	Filter string `json:"filter,omitempty" jsonschema:"Категория метаданных: Справочники, Документы, Перечисления, РегистрыСведений, РегистрыНакопления и др. Без фильтра приходит сводка по категориям"`
}

func MetadataTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "metadata",
		Description: "Список объектов конфигурации 1С по категориям: справочники, документы, регистры, " +
			"перечисления и т.д. Без фильтра — сводка (категория и количество), с filter — полный " +
			"перечень объектов категории. Вызывай первым при работе с незнакомой конфигурацией: " +
			"имена объектов из результата используются в object и в запросах.",
	}
}

type metadataReply struct {
	Категории []struct {
		Категория  string `json:"категория"`
		Количество int    `json:"количество"`
	} `json:"категории"`
	Категория  string `json:"категория"`
	Количество int    `json:"количество"`
	Объекты    []struct {
		Имя     string `json:"имя"`
		Синоним string `json:"синоним"`
	} `json:"объекты"`
}

func (s *Set) Metadata(ctx context.Context, _ *mcp.CallToolRequest, in MetadataInput) (*mcp.CallToolResult, any, error) {
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}
	query := url.Values{}
	if strings.TrimSpace(in.Filter) != "" {
		query.Set("filter", in.Filter)
	}

	var reply metadataReply
	if err := client.Ask(ctx, "metadata", query, &reply); err != nil {
		return nil, nil, err
	}

	var b strings.Builder
	if in.Filter == "" {
		итого := 0
		fmt.Fprintf(&b, "Состав конфигурации базы %s:\n\n", client.Base().Name)
		for _, c := range reply.Категории {
			fmt.Fprintf(&b, "  %-30s %d\n", c.Категория, c.Количество)
			итого += c.Количество
		}
		fmt.Fprintf(&b, "\nВсего объектов по категориям: %d.\n", итого)
		b.WriteString(`Перечень объектов категории: тот же инструмент с filter="<категория>".`)
		return text(b.String()), nil, nil
	}

	fmt.Fprintf(&b, "%s базы %s: %d\n\n", reply.Категория, client.Base().Name, reply.Количество)
	for _, o := range reply.Объекты {
		if o.Синоним != "" && o.Синоним != o.Имя {
			fmt.Fprintf(&b, "  %s — %s\n", o.Имя, o.Синоним)
		} else {
			fmt.Fprintf(&b, "  %s\n", o.Имя)
		}
	}
	return text(b.String()), nil, nil
}

// ── Структура объекта ─────────────────────────────────────────────────────────

type ObjectInput struct {
	Base       string `json:"base,omitempty" jsonschema:"Имя базы 1С из реестра. Опущено — база по умолчанию"`
	ObjectType string `json:"object_type" jsonschema:"Тип объекта: Catalog, Document, Enum, InformationRegister, AccumulationRegister, AccountingRegister, CalculationRegister, ChartOfAccounts, ChartOfCharacteristicTypes, ChartOfCalculationTypes, ExchangePlan, BusinessProcess, Task, Constant, DataProcessor, Report, DefinedType, Subsystem. Соответствие категориям metadata: Справочники→Catalog, Документы→Document, Перечисления→Enum, РегистрыСведений→InformationRegister, РегистрыНакопления→AccumulationRegister, РегистрыБухгалтерии→AccountingRegister, Обработки→DataProcessor, Отчеты→Report, Подсистемы→Subsystem"`
	ObjectName string `json:"object_name" jsonschema:"Имя объекта метаданных, например Номенклатура"`
}

func ObjectTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "object",
		Description: "Получить реквизиты, табличные части, измерения, ресурсы и значения перечисления " +
			"объекта метаданных 1С — из чего он состоит и какими полями им пользоваться. Типы полей " +
			"называются явно (CatalogRef.Номенклатура, Number(15,2)), чтобы по ним можно было писать " +
			"запросы. Вызывай ПЕРЕД написанием запроса: имена ресурсов виртуальных таблиц регистра " +
			"отличаются от имён самого регистра и берутся отсюда.",
	}
}

type field struct {
	Имя     string `json:"имя"`
	Синоним string `json:"синоним"`
	Тип     string `json:"тип"`
}

type objectReply struct {
	Тип            string  `json:"тип"`
	Имя            string  `json:"имя"`
	Синоним        string  `json:"синоним"`
	ПолноеИмя      string  `json:"полноеИмя"`
	Реквизиты      []field `json:"реквизиты"`
	Измерения      []field `json:"измерения"`
	Ресурсы        []field `json:"ресурсы"`
	ТабличныеЧасти []struct {
		Имя       string  `json:"имя"`
		Синоним   string  `json:"синоним"`
		Реквизиты []field `json:"реквизиты"`
	} `json:"табличныеЧасти"`
	Значения []struct {
		Имя     string `json:"имя"`
		Синоним string `json:"синоним"`
	} `json:"значения"`
	// Только для DefinedType — состав типов; и для Subsystem — что в неё входит.
	Типы       []string `json:"типы"`
	Состав     []string `json:"состав"`
	Подсистемы []struct {
		Имя     string `json:"имя"`
		Синоним string `json:"синоним"`
	} `json:"подсистемы"`
}

func (s *Set) Object(ctx context.Context, _ *mcp.CallToolRequest, in ObjectInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.ObjectType) == "" || strings.TrimSpace(in.ObjectName) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "объект не назван",
			"нужны object_type и object_name",
			"перечень объектов категории — инструмент metadata с filter")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	query := url.Values{}
	query.Set("type", in.ObjectType)
	query.Set("name", in.ObjectName)

	var reply objectReply
	if err := client.Ask(ctx, "object", query, &reply); err != nil {
		return nil, nil, err
	}

	var b strings.Builder
	fmt.Fprintf(&b, "%s (%s)\n", reply.ПолноеИмя, reply.Синоним)
	writeFields(&b, "Реквизиты", reply.Реквизиты)
	writeFields(&b, "Измерения", reply.Измерения)
	writeFields(&b, "Ресурсы", reply.Ресурсы)

	for _, part := range reply.ТабличныеЧасти {
		writeFields(&b, "Табличная часть "+part.Имя, part.Реквизиты)
	}
	if len(reply.Значения) > 0 {
		fmt.Fprintf(&b, "\nЗначения перечисления (%d):\n", len(reply.Значения))
		for _, v := range reply.Значения {
			fmt.Fprintf(&b, "  %s — %s\n", v.Имя, v.Синоним)
		}
	}
	if len(reply.Типы) > 0 {
		fmt.Fprintf(&b, "\nСостав определяемого типа (%d):\n", len(reply.Типы))
		for _, t := range reply.Типы {
			fmt.Fprintf(&b, "  %s\n", t)
		}
	}
	if len(reply.Состав) > 0 {
		fmt.Fprintf(&b, "\nСостав подсистемы (%d):\n", len(reply.Состав))
		for _, o := range reply.Состав {
			fmt.Fprintf(&b, "  %s\n", o)
		}
	}
	if len(reply.Подсистемы) > 0 {
		fmt.Fprintf(&b, "\nВложенные подсистемы (%d):\n", len(reply.Подсистемы))
		for _, sub := range reply.Подсистемы {
			fmt.Fprintf(&b, "  %s — %s\n", sub.Имя, sub.Синоним)
		}
	}
	if len(reply.Ресурсы) > 0 {
		b.WriteString("\nВ виртуальных таблицах регистра имена ресурсов другие: " +
			"к остаткам добавляется «Остаток», к оборотам — «Оборот».")
	}
	return text(b.String()), nil, nil
}

func writeFields(b *strings.Builder, title string, fields []field) {
	if len(fields) == 0 {
		return
	}
	fmt.Fprintf(b, "\n%s (%d):\n", title, len(fields))
	for _, f := range fields {
		fmt.Fprintf(b, "  %-40s %s\n", f.Имя, f.Тип)
	}
}
