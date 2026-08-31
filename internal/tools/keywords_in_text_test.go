package tools

import (
	"os"
	"strings"
	"testing"
)

// Слова, у которых своей страницы в справке нет: они описаны ВНУТРИ тем. Верный ответ на них —
// страница темы, а не отказ. До 26.08.2026 таких слов было 10 из 90 ключевых, и все они
// отказывали — при том что справка их описывает.
func TestСловаВнутриТемНаходятся(t *testing.T) {
	if os.Getenv("GTDATA_KB") == "" {
		t.Skip("база справки не подана (GTDATA_KB)")
	}
	db, err := openKB()
	if err != nil {
		t.Skip(err)
	}
	defer db.Close()

	cases := []struct{ word, expectTitle string }{
		{"ТОГДА", "выбор"},
		{"КОГДА", "выбор"},
		{"ИНАЧЕ", "выбор"},
		{"ИЛИ", "услови"},
		{"СПЕЦСИМВОЛ", "подоби"},
		{"НАБОРАМ", "сгруппировать"},
		{"УНИКАЛЬНО", "индексировать"},
	}
	for _, c := range cases {
		page, _ := searchHelp(db, c.word)
		if page == nil {
			t.Errorf("%q — отказ, хотя слово описано в справке", c.word)
			continue
		}
		if !strings.Contains(strings.ToLower(page.Title), c.expectTitle) {
			t.Errorf("%q ведёт на %q, ожидалась тема про %q", c.word, page.Title, c.expectTitle)
		}
	}
}

// Индекс тела не должен спорить с оглавлением: слово, у которого СВОЯ тема есть, берётся
// из оглавления. Иначе выводы по частоте перебивают вендорское соответствие — проверено
// замером, двуязычие падало с 99% до 94% (ДАТА уходила на РАЗНОСТЬДАТ).
func TestИндексТелаНеСпоритСОглавлением(t *testing.T) {
	if os.Getenv("GTDATA_KB") == "" {
		t.Skip("база справки не подана (GTDATA_KB)")
	}
	db, err := openKB()
	if err != nil {
		t.Skip(err)
	}
	defer db.Close()
	ix := getHelpIndex(db)

	for _, word := range []string{"ДАТА", "ПОДОБНО", "ИТОГИ", "ВЫБРАТЬ"} {
		k := normKey(word)
		if _, ok := ix.inText[k]; ok {
			t.Errorf("%q попало в индекс тела, хотя своя тема у него есть", word)
		}
	}
	// А слова без своей темы там быть обязаны — иначе индекс пуст и правило не работает.
	found := 0
	for _, word := range []string{"ТОГДА", "КОГДА", "ИНАЧЕ", "СПЕЦСИМВОЛ"} {
		if _, ok := ix.inText[normKey(word)]; ok {
			found++
		}
	}
	if found == 0 {
		t.Error("индекс тела пуст: ни одно слово без своей темы в него не попало")
	}
}
