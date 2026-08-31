package tools

import (
	"os"
	"strings"
	"testing"
)

// Двуязычие языка запросов: английская форма ключевого слова обязана приводить туда же,
// куда русская. Пары берутся у вендора (страница «Двуязычное представление ключевых слов»),
// а не выписываются нами — выписанный руками словарь расходится со справкой молча.
//
// Проверяются формы, которые до появления словаря отказывали или уводили в объектную модель:
// ORDER BY, TOTALS и SELECT — самые частые конструкции языка запросов.
func TestДвуязычиеКлючевыхСлов(t *testing.T) {
	if os.Getenv("GTDATA_KB") == "" {
		t.Skip("база справки не подана (GTDATA_KB)")
	}
	db, err := openKB()
	if err != nil {
		t.Skip(err)
	}
	defer db.Close()

	pairs := []struct{ ru, en string }{
		{"УПОРЯДОЧИТЬ ПО", "ORDER BY"},
		{"ИТОГИ", "TOTALS"},
		{"ВЫБРАТЬ", "SELECT"},
		{"ПОДОБНО", "LIKE"},
		{"ГДЕ", "WHERE"},
		{"СГРУППИРОВАТЬ ПО", "GROUP BY"},
		{"ВЫРАЗИТЬ", "CAST"},
	}
	for _, p := range pairs {
		ru, _ := searchHelp(db, p.ru)
		if ru == nil {
			t.Errorf("русская форма %q не нашлась — проверять нечем", p.ru)
			continue
		}
		en, _ := searchHelp(db, p.en)
		if en == nil {
			t.Errorf("%q: русская даёт %s, английская — отказ", p.en, ru.Object)
			continue
		}
		if en.Object != ru.Object {
			t.Errorf("%q ведёт на %s (%s), а %q — на %s (%s)",
				p.en, en.Object, en.Title, p.ru, ru.Object, ru.Title)
		}
	}
}

// Словарь строится из справки и непуст: пустой словарь молча вернул бы поведение
// «английские формы не находятся», и заметить это можно было бы только замером.
func TestСловарьДвуязычияСобран(t *testing.T) {
	if os.Getenv("GTDATA_KB") == "" {
		t.Skip("база справки не подана (GTDATA_KB)")
	}
	db, err := openKB()
	if err != nil {
		t.Skip(err)
	}
	defer db.Close()
	ix := getHelpIndex(db)
	if len(ix.alias) < 100 {
		t.Fatalf("в словаре %d связей — таблица вендора разобрана не полностью", len(ix.alias))
	}
	for _, c := range []struct{ key, want string }{
		{"ORDERBY", "УПОРЯДОЧИТЬ ПО"},
		{"TOTALS", "ИТОГИ"}, // головное слово составной конструкции «ИТОГИ … ПО»
		{"ПОДОБНО", "LIKE"},
	} {
		if got := ix.alias[normKey(c.key)]; !strings.EqualFold(got, c.want) {
			t.Errorf("alias[%q] = %q, ожидалось %q", c.key, got, c.want)
		}
	}
}
