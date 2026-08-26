package tools

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/refusal"
)

// ── Паспорт базы ──────────────────────────────────────────────────────────────

type BaseInfoInput struct {
	Base string `json:"base" jsonschema:"Имя базы 1С из реестра. Обязательно; перечень — bases с action=list"`
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
	Base   string `json:"base" jsonschema:"Имя базы 1С из реестра. Обязательно; перечень — bases с action=list"`
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
	Base       string `json:"base" jsonschema:"Имя базы 1С из реестра. Обязательно; перечень — bases с action=list"`
	ObjectType string `json:"object_type" jsonschema:"Тип объекта: Catalog, Document, Enum, InformationRegister, AccumulationRegister, AccountingRegister, CalculationRegister, ChartOfAccounts, ChartOfCharacteristicTypes, ChartOfCalculationTypes, ExchangePlan, BusinessProcess, Task, Constant, DataProcessor, Report, DefinedType, Subsystem. Соответствие категориям metadata: Справочники→Catalog, Документы→Document, Перечисления→Enum, РегистрыСведений→InformationRegister, РегистрыНакопления→AccumulationRegister, РегистрыБухгалтерии→AccountingRegister, Обработки→DataProcessor, Отчеты→Report, Подсистемы→Subsystem"`
	ObjectName string `json:"object_name" jsonschema:"Имя объекта метаданных, например Номенклатура"`
}

func ObjectTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "object",
		Description: "Получить поля объекта метаданных 1С — из чего он состоит и какими именами " +
			"им пользоваться в запросе: стандартные поля платформы (Период, Регистратор, Активность, " +
			"Номер, Дата, Проведен), реквизиты, измерения, ресурсы, табличные части, значения " +
			"перечисления. Типы называются явно (CatalogRef.Номенклатура, Number(15,2)). Вызывай " +
			"ПЕРЕД написанием запроса: имена ресурсов виртуальных таблиц регистра отличаются от имён " +
			"самого регистра, а у регистра бухгалтерии небалансовые измерения и ресурсы существуют " +
			"в таблице только как «имяДт» и «имяКт» — ответ показывает их в этом виде.",
	}
}

type field struct {
	Имя     string `json:"имя"`
	Синоним string `json:"синоним"`
	Тип     string `json:"тип"`
	// Только у регистра бухгалтерии. Небалансовое измерение или ресурс раздваивается
	// в таблице запроса на «имяДт» и «имяКт» — имя из метаданных туда не подставляется.
	// Указатель, а не bool: отсутствие признака и «небалансовое» — разные вещи.
	Балансовый *bool `json:"балансовый,omitempty"`
}

type objectReply struct {
	Тип       string `json:"тип"`
	Имя       string `json:"имя"`
	Синоним   string `json:"синоним"`
	ПолноеИмя string `json:"полноеИмя"`
	// Поля, которые заводит сама платформа: Период, Регистратор, Активность у регистров,
	// Номер, Дата, Проведен у документов. В коллекциях метаданных их нет, а запрос пишется
	// именно по ним.
	СтандартныеПоля []field `json:"стандартныеПоля"`
	Особенности     *struct {
		Корреспонденция bool     `json:"корреспонденция"`
		ПоляСчета       []string `json:"поляСчета"`
		ПланСчетов      string   `json:"планСчетов"`
		МаксСубконто    int      `json:"максСубконто"`
	} `json:"особенностиРегистраБухгалтерии"`
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
		return nil, nil, hintNotFound(err)
	}

	var b strings.Builder
	fmt.Fprintf(&b, "%s (%s)\n", reply.ПолноеИмя, reply.Синоним)

	// Раздвоение имён включается свойством регистра, а не типом поля: при корреспонденции
	// небалансовые измерения и ресурсы существуют в таблице только как «имяДт» и «имяКт».
	split := reply.Особенности != nil && reply.Особенности.Корреспонденция

	writeFields(&b, "Стандартные поля", reply.СтандартныеПоля, false)
	writeFields(&b, "Реквизиты", reply.Реквизиты, false)
	writeFields(&b, "Измерения", reply.Измерения, split)
	writeFields(&b, "Ресурсы", reply.Ресурсы, split)

	for _, part := range reply.ТабличныеЧасти {
		writeFields(&b, "Табличная часть "+part.Имя, part.Реквизиты, false)
	}
	if o := reply.Особенности; o != nil {
		b.WriteString("\nРегистр бухгалтерии:\n")
		fmt.Fprintf(&b, "  счёт в запросе — %s\n", strings.Join(o.ПоляСчета, ", "))
		if o.ПланСчетов != "" {
			fmt.Fprintf(&b, "  план счетов — ПланСчетов.%s", o.ПланСчетов)
			if o.МаксСубконто > 0 {
				fmt.Fprintf(&b, ", субконто до %d", o.МаксСубконто)
			}
			b.WriteString("\n")
		}
		if split {
			b.WriteString("  небалансовые измерения и ресурсы показаны выше в том виде, " +
				"в каком существуют в таблице: «имяДт» и «имяКт»\n")
		}
		b.WriteString("  отбор по счёту-группе через «=» вернёт ноль строк без ошибки — " +
			"пиши «В ИЕРАРХИИ (&Счет)»\n")
		b.WriteString("  Субконто1..N и ВидСубконто1..N платформа объявляет стандартными полями, " +
			"но в ОСНОВНОЙ таблице регистра их нет — запрос по ним не разберётся. Они лежат " +
			"в виртуальной ДвиженияССубконто(Начало, Конец, Условие, Порядок, " +
			"МаксимальноеКоличество) и называются там СубконтоДт1..N и СубконтоКт1..N; " +
			"условие идёт ТРЕТЬИМ параметром\n")
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

// split — раздваивать ли имена небалансовых полей на «имяДт» / «имяКт». Печатается именно то
// имя, которое примет запрос: имя из метаданных здесь ввело бы в заблуждение молча.
func writeFields(b *strings.Builder, title string, fields []field, split bool) {
	if len(fields) == 0 {
		return
	}
	fmt.Fprintf(b, "\n%s (%d):\n", title, len(fields))
	for _, f := range fields {
		name := f.Имя
		if split && f.Балансовый != nil && !*f.Балансовый {
			name = f.Имя + "Дт / " + f.Имя + "Кт"
		}
		fmt.Fprintf(b, "  %-40s %s\n", name, f.Тип)
	}
}

// hintNotFound добавляет отказу «объект не найден» ход, которым его надо закрывать.
//
// Заведено по случаю 26.08.2026: отказ прочитался как факт о 1С вообще, и модель принялась
// достраивать имя документа по образцу — «может, он называется иначе». Отказ теперь называет
// базу сам (refusal.Error.Base), а здесь дописывается, куда идти вместо подбора имён.
//
// Опрашивать соседние базы — «а нет ли объекта там» — сознательно НЕ делаем. Соседняя база
// это другая конфигурация и, как правило, другая организация: подсказка звала бы за чужими
// данными, а отчёт по ним вышел бы складным и неверным по смыслу. Плюс цена: по HTTP-запросу
// на базу с полным таймаутом, и всё это на пути отказа, который обязан быть быстрым.
func hintNotFound(err error) error {
	var ref *refusal.Error
	if !errors.As(err, &ref) || ref.Kind != refusal.BaseError ||
		!strings.Contains(ref.What, "не найден") {
		return err
	}
	ref.Hints = append(ref.Hints,
		"это отказ по НАЗВАННОЙ базе, а не про 1С вообще: в другой конфигурации объект "+
			"может называться иначе или отсутствовать",
		"перечень объектов категории — metadata с filter; подбирать имя по образцу нельзя",
		"если объект из другой конфигурации — назовите нужную базу параметром base "+
			"(перечень: bases с action=list)")
	return err
}
