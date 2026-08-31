package tools

import (
	"strings"
	"testing"
)

// Три дефекта syntax из прогона локальной модели 27.08.2026 (сессия 15:34): текст запроса
// в поле query отвечал страницей «Первые» сырыми скобками; «SGROUP BY функция выражение»
// отвечал ВыражениеXPath; тело страницы бралось по имени объекта без порядка.

const sessionQueryText = "ВЫБРАТЬ ПЕРВЫЕ 5 Месяц, КОЛИЧЕСТВО(РАЗЛИЧНЫЕ Ссылка) ИЗ Документ.РеализацияТоваровУслуг КАК р " +
	"ГДЕ р.Дата МЕЖДУ &Н И &К СГРУППИРОВАТЬ ПО НАЧАЛОПЕРИОДА(р.Дата, МЕСЯЦ) КАК Месяц"

func TestLooksLikeQueryText(t *testing.T) {
	yes := []string{sessionQueryText, "ВЫБРАТЬ 1 ИЗ Справочник.Номенклатура", "SELECT Ref FROM Catalog.Goods WHERE Code = &Code"}
	no := []string{"ПОДОБНО", "УПОРЯДОЧИТЬ ПО", "ОстаткиИОбороты", "РегистрБухгалтерии.ОстаткиИОбороты", "агрегатные функции", "НАЧАЛОПЕРИОДА"}
	for _, q := range yes {
		if !looksLikeQueryText(q) {
			t.Errorf("%q: это текст запроса, не распознан", q)
		}
	}
	for _, q := range no {
		if looksLikeQueryText(q) {
			t.Errorf("%q: это имя конструкции, принято за текст запроса", q)
		}
	}
}

func TestConstructionsInQueryText(t *testing.T) {
	db := openTestKB(t)
	defer db.Close()
	found := getHelpIndex(db).constructionsIn(sessionQueryText)
	joined := strings.Join(found, " ")
	for _, want := range []string{"ПЕРВЫЕ", "СГРУППИРОВАТЬ", "НАЧАЛОПЕРИОДА"} {
		if !strings.Contains(joined, want) {
			t.Errorf("в тексте не узнана конструкция %s: %v", want, found)
		}
	}
	for _, bad := range []string{"Месяц", "Ссылка", "Документ"} {
		if strings.Contains(joined, bad) {
			t.Errorf("имя поля/таблицы %s принято за конструкцию: %v", bad, found)
		}
	}
}

func TestSearchRefusesWeakHit(t *testing.T) {
	db := openTestKB(t)
	defer db.Close()
	if best, _ := searchHelp(db, "SGROUP BY функция выражение"); best != nil {
		t.Errorf("слабое попадание по одному слову обязано давать отказ, выдана %s (%s)", best.Object, best.Title)
	}
	// Многословные вопросы с настоящим ответом отказом не становятся.
	for _, q := range []string{"агрегатные функции", "РегистрБухгалтерии.ОстаткиИОбороты", "УПОРЯДОЧИТЬ ПО"} {
		if best, _ := searchHelp(db, q); best == nil {
			t.Errorf("%q: ложный отказ", q)
		}
	}
}

func TestPageBodyНеСкобки(t *testing.T) {
	db := openTestKB(t)
	defer db.Close()
	for _, q := range []string{"ПЕРВЫЕ", "Первые", "ПОДОБНО", "ОстаткиИОбороты"} {
		best, _ := searchHelp(db, q)
		if best == nil {
			continue
		}
		body, _ := pageBody(db, *best)
		if strings.HasPrefix(strings.TrimSpace(body), "{") || strings.TrimSpace(body) == "" {
			t.Errorf("%q → %s: тело страницы — скобки или пусто:\n%s", q, best.Path, body[:min(len(body), 120)])
		}
	}
}
