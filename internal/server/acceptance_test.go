package server_test

import (
	"strings"
	"testing"
)

// Приёмка: новый канал против прежнего и против самого себя.
//
// Эталонные числа сняты действующим каналом 23.08.2026 на стенде УТ. Приёмка обязана
// падать, если сервер начнёт отвечать иначе: «сервер ответил» — не критерий, критерий —
// «ответил то же самое».
const (
	эталонНоменклатуры = 20
	эталонОстатка      = "3823"
	эталонКонфигурация = "УправлениеТорговлей"
	эталонВерсия       = "11.5.12.256"
)

// Типы объектов, которые понимал прежний канал: замена не считается полной,
// пока хоть один из них не распознаётся.
var типыПрежнегоКанала = map[string]string{
	"Catalog":                    "Справочники",
	"Document":                   "Документы",
	"Enum":                       "Перечисления",
	"InformationRegister":        "РегистрыСведений",
	"AccumulationRegister":       "РегистрыНакопления",
	"AccountingRegister":         "РегистрыБухгалтерии",
	"CalculationRegister":        "РегистрыРасчета",
	"ChartOfAccounts":            "ПланыСчетов",
	"ChartOfCharacteristicTypes": "ПланыВидовХарактеристик",
	"ChartOfCalculationTypes":    "ПланыВидовРасчета",
	"ExchangePlan":               "ПланыОбмена",
	"BusinessProcess":            "БизнесПроцессы",
	"Task":                       "Задачи",
	"DataProcessor":              "Обработки",
	"Report":                     "Отчеты",
	"DefinedType":                "ОпределяемыеТипы",
	"Subsystem":                  "Подсистемы",
}

func TestПриёмкаЧислаСовпадаютСЭталоном(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "base_info", nil)
	if isErr {
		t.Fatalf("паспорт базы не получен: %s", out)
	}
	for _, want := range []string{эталонКонфигурация, эталонВерсия} {
		if !strings.Contains(out, want) {
			t.Errorf("паспорт базы разошёлся с эталоном: нет %q\n%s", want, out)
		}
	}

	out, isErr = call(t, cs, ctx, "count", map[string]any{"table": "Справочник.Номенклатура"})
	if isErr {
		t.Fatalf("счёт не выполнен: %s", out)
	}
	if got := extractNumber(out, "записей "); got != "20" {
		t.Errorf("число элементов номенклатуры разошлось с эталоном: %s против %d",
			got, эталонНоменклатуры)
	}

	out, isErr = call(t, cs, ctx, "register", map[string]any{
		"register": "ТоварыНаСкладах", "kind": "Остатки",
		"dimensions": []string{"Склад"}, "resources": []string{"ВНаличииОстаток"},
	})
	if isErr {
		t.Fatalf("итоги регистра не получены: %s", out)
	}
	if !strings.Contains(out, эталонОстатка) {
		t.Errorf("остаток по складу разошёлся с эталоном %s:\n%s", эталонОстатка, out)
	}
}

func TestПриёмкаВсеТипыПрежнегоКаналаРаботают(t *testing.T) {
	cs, ctx := liveSession(t)

	// Тип засчитывается проверенным, если сработал хотя бы на одной базе: планов счетов
	// в торговле нет, а в бухгалтерии есть — по отдельности ни одна база полноты не даёт.
	проверено := map[string]bool{}

	for _, база := range []string{"ut11", "bu3"} {
		summary, isErr := call(t, cs, ctx, "metadata", map[string]any{"base": база})
		if isErr {
			t.Fatalf("сводка метаданных базы %s не получена: %s", база, summary)
		}

		for тип, категория := range типыПрежнегоКанала {
			if !strings.Contains(summary, категория) {
				t.Errorf("база %s: категории %q нет в сводке — тип %s недостижим", база, категория, тип)
				continue
			}

			list, isErr := call(t, cs, ctx, "metadata", map[string]any{
				"base": база, "filter": категория,
			})
			if isErr {
				t.Errorf("база %s: перечень категории %s не получен: %s", база, категория, list)
				continue
			}
			имя := firstObjectName(list)
			if имя == "" {
				continue // категория пуста в этой конфигурации — смотрим на другой базе
			}

			out, isErr := call(t, cs, ctx, "object", map[string]any{
				"base": база, "object_type": тип, "object_name": имя,
			})
			if isErr {
				t.Errorf("база %s: тип %s не работает на объекте %q: %s", база, тип, имя, out)
				continue
			}
			проверено[тип] = true
		}
	}

	// Чего нет ни в торговле, ни в бухгалтерии (регистры расчёта живут в ЗУП), проверяем
	// иначе: тип обязан быть РАСПОЗНАН. Отказ «объект не найден» — законный ответ,
	// отказ «тип не распознан» — дыра в переносе.
	for тип := range типыПрежнегоКанала {
		if проверено[тип] {
			continue
		}
		out, isErr := call(t, cs, ctx, "object", map[string]any{
			"object_type": тип, "object_name": "ЗаведомоНесуществующийОбъект",
		})
		if !isErr {
			t.Errorf("тип %s: несуществующий объект обязан давать отказ, получено: %s", тип, out)
			continue
		}
		if strings.Contains(out, "Тип объекта не распознан") {
			t.Errorf("тип %s не распознаётся сервером — перенос неполный", тип)
			continue
		}
		t.Logf("тип %s: объектов этого вида нет на стендах, проверено распознавание типа", тип)
	}
}

func TestПриёмкаОсобыеПоляОбъектов(t *testing.T) {
	cs, ctx := liveSession(t)

	// Подсистема без состава бесполезна: ради состава её и спрашивают.
	list, _ := call(t, cs, ctx, "metadata", map[string]any{"filter": "Подсистемы"})
	if имя := firstObjectName(list); имя != "" {
		out, isErr := call(t, cs, ctx, "object", map[string]any{
			"object_type": "Subsystem", "object_name": имя,
		})
		if isErr || !strings.Contains(out, "Состав подсистемы") && !strings.Contains(out, "Вложенные подсистемы") {
			t.Errorf("подсистема %q не отдала ни состава, ни вложенных:\n%s", имя, out)
		}
	}

	// Определяемый тип без состава типов — тупик: видно имя, не видно, что за ним.
	list, _ = call(t, cs, ctx, "metadata", map[string]any{"filter": "ОпределяемыеТипы"})
	if имя := firstObjectName(list); имя != "" {
		out, isErr := call(t, cs, ctx, "object", map[string]any{
			"object_type": "DefinedType", "object_name": имя,
		})
		if isErr || !strings.Contains(out, "Состав определяемого типа") {
			t.Errorf("определяемый тип %q не отдал состав типов:\n%s", имя, out)
		}
	}
}

func TestПриёмкаЗапросРазбираетсяБезИсполнения(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "query_check", map[string]any{
		"query": "ВЫБРАТЬ ПЕРВЫЕ 1 Ссылка КАК Ссылка ИЗ Справочник.Номенклатура",
	})
	if isErr || !strings.Contains(out, "Ссылка") {
		t.Errorf("разбор запроса не удался: %s", out)
	}

	out, isErr = call(t, cs, ctx, "query_check", map[string]any{
		"query": "ВЫБРАТЬ ИЗ ИЗ Справочник.Номенклатура",
	})
	if !isErr {
		t.Errorf("битый запрос обязан быть отклонён: %s", out)
	}
}

func TestПриёмкаЖурналРегистрации(t *testing.T) {
	cs, ctx := liveSession(t)

	out, isErr := call(t, cs, ctx, "eventlog", map[string]any{"limit": 5})
	if isErr {
		t.Fatalf("журнал регистрации не прочитан: %s", out)
	}
	if !strings.Contains(out, "Журнал регистрации базы") {
		t.Errorf("ответ журнала не опознан:\n%s", out)
	}
}

// firstObjectName достаёт имя первого объекта из перечня категории.
//
// Перечень печатается строками «  Имя — синоним», поэтому берём первую строку
// с отступом и режем по тире.
func firstObjectName(list string) string {
	for _, line := range strings.Split(list, "\n") {
		if !strings.HasPrefix(line, "  ") {
			continue
		}
		name := strings.TrimSpace(line)
		if idx := strings.Index(name, " — "); idx >= 0 {
			name = name[:idx]
		}
		if name != "" {
			return name
		}
	}
	return ""
}
