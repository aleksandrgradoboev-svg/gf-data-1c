package tools

import (
	"os"
	"strings"
	"testing"
)

func memberIndexForTest(t *testing.T) *memberIndex {
	t.Helper()
	if os.Getenv("GTDATA_KB") == "" {
		t.Skip("база справки не подана (GTDATA_KB)")
	}
	db, err := openKB()
	if err != nil {
		t.Skip(err)
	}
	t.Cleanup(func() { db.Close() })
	return getMemberIndex(db)
}

// Перечень членов типа собирается из страниц справки, разложенных вендором по /methods/
// и /properties/. Числа сверены с mcp-bsl-context 0.3.2 на восьми типах 26.08.2026:
// 103 члена против 102 у него, перечни совпадают член в член.
func TestЧленыТипаСобираются(t *testing.T) {
	ix := memberIndexForTest(t)

	e, ok := ix.membersOf("ТаблицаЗначений")
	if !ok {
		t.Fatal("ТаблицаЗначений — членов не найдено")
	}
	if e.TypeEN != "ValueTable" {
		t.Errorf("английское имя типа %q, ожидалось ValueTable", e.TypeEN)
	}
	var methods, props int
	for _, m := range e.Members {
		switch m.Kind {
		case kindMethod:
			methods++
		case kindProperty:
			props++
		}
	}
	// У ТаблицаЗначений вендор описывает 19 методов и 2 свойства. Проверяется порог, а не
	// точное число: справка новой платформы может добавить член, и тест не должен падать
	// на пополнении вендора — он ловит развал разбора, а не редакцию справки.
	if methods < 19 {
		t.Errorf("методов %d, ожидалось не меньше 19", methods)
	}
	if props < 2 {
		t.Errorf("свойств %d, ожидалось не меньше 2", props)
	}
}

// Тип спрашивают и по-русски, и по-английски — оба имени обязаны вести к одному перечню.
func TestЧленыТипаПоОбоимИменам(t *testing.T) {
	ix := memberIndexForTest(t)

	ru, okRU := ix.membersOf("ТаблицаЗначений")
	en, okEN := ix.membersOf("ValueTable")
	if !okRU || !okEN {
		t.Fatal("тип найден не по обоим именам")
	}
	if len(ru.Members) != len(en.Members) {
		t.Errorf("по РУ %d членов, по EN %d — должно совпадать", len(ru.Members), len(en.Members))
	}
}

// Свойство «<Имя ключа>» у Структуры записано вендором отдельной страницей, и чужой сервер
// его теряет. Это не придирка: именно оно объясняет, что к структуре обращаются как
// Стр.ИмяКлюча, и без него перечень выглядит так, будто у типа нет полей вовсе.
func TestЧленыСтруктурыВключаютИмяКлюча(t *testing.T) {
	ix := memberIndexForTest(t)

	e, ok := ix.membersOf("Структура")
	if !ok {
		t.Fatal("Структура — членов не найдено")
	}
	var found bool
	for _, m := range e.Members {
		if strings.Contains(m.Name, "Имя ключа") {
			found = true
		}
	}
	if !found {
		t.Error("свойство «<Имя ключа>» потеряно")
	}
}

// Тип возврата очищается от служебной обёртки справки, но перечисление типов сохраняется:
// «Число, Неопределено» — это два допустимых типа, а не мусор.
func TestТипВозвратаОчищен(t *testing.T) {
	cases := map[string]string{
		"Тип: СтрокаТаблицыЗначений.":         "СтрокаТаблицыЗначений",
		"Тип: Число, Неопределено.":           "Число, Неопределено",
		"  Тип:  Массив.  ":                   "Массив",
		"СтрокаТаблицыЗначений, Неопределено": "СтрокаТаблицыЗначений, Неопределено",
	}
	for in, want := range cases {
		if got := cleanReturns(in); got != want {
			t.Errorf("cleanReturns(%q) = %q, ожидалось %q", in, got, want)
		}
	}
}

// Счёт типов не должен считать один тип дважды: русское и английское имя ведут на одну запись.
func TestСчётТиповНеДвоится(t *testing.T) {
	ix := memberIndexForTest(t)

	n := ix.typeCount()
	if n == 0 {
		t.Fatal("типов с членами не найдено вовсе")
	}
	if n >= len(ix.byType) {
		t.Errorf("типов %d при %d ключах — счёт идёт по ключам, а не по типам",
			n, len(ix.byType))
	}
}
