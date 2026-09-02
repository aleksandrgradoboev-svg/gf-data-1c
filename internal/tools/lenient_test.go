package tools

import (
	"encoding/json"
	"testing"

	"github.com/google/jsonschema-go/jsonschema"
)

func TestСписокСтрокПринимаетТриФормы(t *testing.T) {
	случаи := []struct {
		имя  string
		вход string
		ждём []string
	}{
		{"массив", `["А", "Б"]`, []string{"А", "Б"}},
		{"массив в строке", `"[\"СуммаОборотДт\", \"СуммаОборотКт\"]"`, []string{"СуммаОборотДт", "СуммаОборотКт"}},
		{"одиночная строка", `"СуммаОборотДт"`, []string{"СуммаОборотДт"}},
		{"строка с пробелами вокруг массива", `"  [\"А\"]  "`, []string{"А"}},
	}
	for _, с := range случаи {
		var л СписокСтрок
		if err := json.Unmarshal([]byte(с.вход), &л); err != nil {
			t.Fatalf("%s: %v", с.имя, err)
		}
		if len(л) != len(с.ждём) {
			t.Fatalf("%s: получено %v, ждали %v", с.имя, л, с.ждём)
		}
		for i := range л {
			if л[i] != с.ждём[i] {
				t.Fatalf("%s: получено %v, ждали %v", с.имя, л, с.ждём)
			}
		}
	}
}

func TestСписокСтрокЧислоДаётОшибку(t *testing.T) {
	var л СписокСтрок
	if err := json.Unmarshal([]byte(`42`), &л); err == nil {
		t.Fatal("число не должно приниматься")
	}
}

// Стрингифицированный select — 17 отказов подряд в живой сессии 02.09.2026.
// Вход обязан разбираться в те же поля, что и честный массив.
func TestQueryBuildInputРазворачиваетСтрокуСМассивом(t *testing.T) {
	вход := `{"base":"bu3","from":"Документ.Тест",` +
		`"select":"[\"Ссылка\", {\"field\": \"СуммаДокумента\", \"func\": \"СУММА\", \"as\": \"Сумма\"}]",` +
		`"where":"[\"Дата МЕЖДУ &Н И &К\", \"Проведен\"]"}`
	var in QueryBuildInput
	if err := json.Unmarshal([]byte(вход), &in); err != nil {
		t.Fatal(err)
	}
	if len(in.Поля) != 2 {
		t.Fatalf("select: получено %d полей, ждали 2", len(in.Поля))
	}
	if len(in.Отбор) != 2 {
		t.Fatalf("where: получено %v, ждали 2 условия", in.Отбор)
	}
	if in.Отбор[0] != "Дата МЕЖДУ &Н И &К" {
		t.Fatalf("where[0]: %q", in.Отбор[0])
	}
}

// Одно условие строкой (НЕ массив) — прежнее поведение не сломано.
func TestQueryBuildInputОдноУсловиеСтрокой(t *testing.T) {
	вход := `{"base":"bu3","from":"Документ.Тест","select":["Ссылка"],"where":"Дата МЕЖДУ &Н И &К"}`
	var in QueryBuildInput
	if err := json.Unmarshal([]byte(вход), &in); err != nil {
		t.Fatal(err)
	}
	if len(in.Отбор) != 1 || in.Отбор[0] != "Дата МЕЖДУ &Н И &К" {
		t.Fatalf("where: %v", in.Отбор)
	}
}

// Схемы обязаны пропускать строку там, где разбор её понимает — иначе
// валидатор SDK отвергает вызов ДО нашего кода и лояльность мертва.
func TestСхемыПропускаютСтрокуВместоСписка(t *testing.T) {
	для := func(имя string, схема *jsonschema.Schema, ключи ...string) {
		for _, ключ := range ключи {
			свойство, есть := схема.Properties[ключ]
			if !есть {
				t.Fatalf("%s: нет ключа %s", имя, ключ)
			}
			нашлась := false
			for _, вариант := range свойство.AnyOf {
				if вариант.Type == "string" {
					нашлась = true
				}
			}
			if !нашлась {
				t.Fatalf("%s.%s: схема не принимает строку", имя, ключ)
			}
		}
	}
	для("query_build", queryBuildSchema(), "select", "where", "group_by", "order_by", "table_params", "params", "joins", "totals")
	для("accounts", схемаСоСтрочнымиСписками[AccountsInput]("resources"), "resources")
	для("register", схемаСоСтрочнымиСписками[RegisterInput]("dimensions", "resources"), "dimensions", "resources")
}
