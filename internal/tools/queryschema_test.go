package tools

import (
	"encoding/json"
	"testing"
)

// Колонки «поля» приходят от модели и строкой, и объектом. Обе формы обязаны проходить
// схему ДО обработчика и раскладываться в одну структуру — иначе отказ валидации
// выглядит для модели как поломка инструмента, и она уходит писать текст руками.

func TestQueryBuildПоляСтрокойИОбъектом(t *testing.T) {
	raw := `{"base":"ut11","источник":"Справочник.Номенклатура",
		"поля":["Ссылка", {"поле":"Наименование","как":"Имя"}, {"функция":"КОЛИЧЕСТВО","поле":"Ссылка","как":"Всего"}]}`
	var in QueryBuildInput
	if err := json.Unmarshal([]byte(raw), &in); err != nil {
		t.Fatalf("разбор ввода: %v", err)
	}
	if len(in.Поля) != 3 {
		t.Fatalf("колонок %d, ждали 3", len(in.Поля))
	}
	if in.Поля[0].Поле != "Ссылка" || in.Поля[0].Как != "" {
		t.Errorf("строка не легла в «поле»: %+v", in.Поля[0])
	}
	if in.Поля[1].Поле != "Наименование" || in.Поля[1].Как != "Имя" {
		t.Errorf("объект разобран неверно: %+v", in.Поля[1])
	}
	if in.Поля[2].Функция != "КОЛИЧЕСТВО" {
		t.Errorf("агрегат потерян: %+v", in.Поля[2])
	}
}

func TestQueryBuildСхемаДопускаетСтроку(t *testing.T) {
	s := queryBuildSchema()
	items := s.Properties["поля"].Items
	if items == nil || len(items.AnyOf) != 2 {
		t.Fatalf("элемент «поля» должен быть anyOf из двух форм, получено %+v", items)
	}
	if items.AnyOf[0].Type != "string" || items.AnyOf[1].Type != "object" {
		t.Errorf("ждали [string, object], получено [%s, %s]", items.AnyOf[0].Type, items.AnyOf[1].Type)
	}
	// Схема должна резолвиться и пропускать строку — это то, что SDK делает до вызова.
	resolved, err := s.Resolve(nil)
	if err != nil {
		t.Fatalf("схема не резолвится: %v", err)
	}
	var doc any
	_ = json.Unmarshal([]byte(`{"base":"ut11","источник":"Справочник.Номенклатура","поля":["Ссылка",{"поле":"Код"}]}`), &doc)
	if err := resolved.Validate(doc); err != nil {
		t.Errorf("строка в «поля» отвергнута схемой: %v", err)
	}
	_ = json.Unmarshal([]byte(`{"base":"ut11","источник":"Справочник.Номенклатура","поля":[42]}`), &doc)
	if err := resolved.Validate(doc); err == nil {
		t.Errorf("число в «поля» должно отвергаться")
	}
}
