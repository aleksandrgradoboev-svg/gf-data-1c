package tools

// Проверка обогащения отказов. Случаи взяты не из воображения, а из журнала живого прогона
// 25.08.2026, где вызывающий 25 раз подряд получал точный, но односложный отказ и в итоге
// бросил запросы и стал считать числа вручную.

import (
	"strings"
	"testing"

	"github.com/aleksandrgradoboev-svg/gf-data-1c/internal/refusal"
)

func baseErr(what, why string) error {
	return refusal.New(refusal.BaseError, what, why)
}

func TestПолеВиртуальнойТаблицы(t *testing.T) {
	err := baseErr("Запрос не выполнен",
		`{(3, 9)}: Поле не найдено "Регистратор"`)
	got := EnrichQueryRefusal(err,
		"ВЫБРАТЬ Регистратор ИЗ РегистрБухгалтерии.Хозрасчетный.ОстаткиИОбороты(&Н, &К, Месяц, , , , )", nil)
	s := got.Error()
	for _, want := range []string{"ОстаткиИОбороты", "основная таблица", "СуммаОборотДт", "object"} {
		if !strings.Contains(s, want) {
			t.Fatalf("в подсказке нет %q:\n%s", want, s)
		}
	}
	if !strings.HasPrefix(s, "ОТКАЗ:") {
		t.Fatalf("первая строка отказа изменилась: %s", s)
	}
}

func TestТаблицаСПрефиксомСправочник(t *testing.T) {
	err := baseErr("Запрос не разобран",
		`{(1, 41)}: Таблица не найдена "Справочник.ПланСчетов.Хозрасчетный"`)
	s := EnrichQueryRefusal(err, "ВЫБРАТЬ Код ИЗ Справочник.ПланСчетов.Хозрасчетный", nil).Error()
	if !strings.Contains(s, "ПланСчетов.<Имя>") {
		t.Fatalf("не подсказано про префикс:\n%s", s)
	}
	if !strings.Contains(s, "metadata") {
		t.Fatalf("не назван инструмент перечня объектов:\n%s", s)
	}
}

func TestНеверныеПараметрыВиртуальнойТаблицы(t *testing.T) {
	err := baseErr("Запрос не выполнен",
		`{(1, 414)}: Неверные параметры "РегистрБухгалтерии.Хозрасчетный.ОстаткиИОбороты, 3"`)
	s := EnrichQueryRefusal(err,
		"ВЫБРАТЬ Счет ИЗ РегистрБухгалтерии.Хозрасчетный.ОстаткиИОбороты(&Н, &К, Счет В ИЕРАРХИИ (&С))", nil).Error()
	if !strings.Contains(s, "Периодичность") {
		t.Fatalf("не показана сигнатура:\n%s", s)
	}
	if !strings.Contains(s, "№3") && !strings.Contains(s, "параметр №3") {
		t.Fatalf("не назван отвергнутый параметр:\n%s", s)
	}
}

func TestПараметрОбъявленНоНеПередан(t *testing.T) {
	err := baseErr("Запрос не выполнен", `{(9, 15)}: Не задано значение параметра "Нач"`)
	s := EnrichQueryRefusal(err, "ВЫБРАТЬ 1 ГДЕ Период >= &Нач", nil).Error()
	if !strings.Contains(s, "parameters") || !strings.Contains(s, "ГГГГ-ММ-ДД") {
		t.Fatalf("не сказано, как передать параметр:\n%s", s)
	}
}

func TestВыдуманныйОператор(t *testing.T) {
	err := baseErr("Запрос не разобран", `{(7, 16)}: Синтаксическая ошибка "НАЧАТОС"`)
	s := EnrichQueryRefusal(err, `ВЫБРАТЬ Код ИЗ ПланСчетов.Хозрасчетный ГДЕ Код НАЧАТОС &Код`, nil).Error()
	if !strings.Contains(s, "ПОДОБНО") {
		t.Fatalf("не предложена замена:\n%s", s)
	}
}

func TestСравнениеСсылкиСоСтрокой(t *testing.T) {
	err := baseErr("Запрос не выполнен",
		"{(8, 12)}: Неверные параметры в операции сравнения. Нельзя сравнивать поля неограниченной длины и поля несовместимых типов")
	s := EnrichQueryRefusal(err, "ВЫБРАТЬ 1 ИЗ РегистрБухгалтерии.Хозрасчетный ГДЕ СчетКт = &Сч", nil).Error()
	if !strings.Contains(s, "Счет.Код") {
		t.Fatalf("не предложен разбор через реквизит:\n%s", s)
	}
	if !strings.Contains(s, "ПУСТОЙ результат") {
		t.Fatalf("не предупреждено про молчаливый пустой ответ:\n%s", s)
	}
}

func TestЧужиеОтказыНеТрогаются(t *testing.T) {
	// Отказ канала обогащать нечем: он не про текст запроса, и лишние подсказки увели бы
	// вызывающего чинить запрос вместо веб-сервера.
	err := refusal.New(refusal.NoWebServer, "канал не отвечает", "соединение отвергнуто")
	if got := EnrichQueryRefusal(err, "ВЫБРАТЬ 1", nil); got.Error() != err.Error() {
		t.Fatalf("отказ канала изменён:\n%s", got.Error())
	}
}

func TestНезнакомаяОшибкаОстаётсяКакЕсть(t *testing.T) {
	err := baseErr("Запрос не выполнен", "{(1, 1)}: Внутренняя ошибка сервера баз данных")
	if got := EnrichQueryRefusal(err, "ВЫБРАТЬ 1", nil); got.Error() != err.Error() {
		t.Fatalf("к незнакомой ошибке дописано лишнее:\n%s", got.Error())
	}
}
