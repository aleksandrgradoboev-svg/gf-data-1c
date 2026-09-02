package tools

// Лояльный приём списков. Слабые модели заворачивают JSON-массив в строку —
// «resources»: "[\"СуммаОборотДт\"]" — и строгая схема отвечает сырым отказом
// валидатора, который модель прочитать не умеет: 18 отказов одной природы за
// одну живую сессию 02.09.2026. Сервер строится для слабых моделей, поэтому
// строку с массивом внутри принимает и разбирает сам.

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/google/jsonschema-go/jsonschema"
)

// СписокСтрок — []string, принимающий три формы: массив строк, строку с
// JSON-массивом внутри и одиночную строку (список из одного элемента).
type СписокСтрок []string

func (л *СписокСтрок) UnmarshalJSON(b []byte) error {
	var массив []string
	if json.Unmarshal(b, &массив) == nil {
		*л = массив
		return nil
	}
	var строка string
	if err := json.Unmarshal(b, &строка); err != nil {
		return fmt.Errorf("ожидается массив строк или строка: %w", err)
	}
	обрезано := strings.TrimSpace(строка)
	if strings.HasPrefix(обрезано, "[") && json.Unmarshal([]byte(обрезано), &массив) == nil {
		*л = массив
		return nil
	}
	*л = []string{строка}
	return nil
}

// схемаСоСтрочнымиСписками — схема типа T, где названные ключи-списки
// принимают и массив строк, и строку. Без правки схемы лояльный разбор не
// работает вовсе: SDK отвергает вызов валидатором ДО нашего UnmarshalJSON.
func схемаСоСтрочнымиСписками[T any](ключи ...string) *jsonschema.Schema {
	s, err := jsonschema.For[T](nil)
	if err != nil {
		panic(fmt.Errorf("схема ввода не выведена: %w", err))
	}
	for _, ключ := range ключи {
		свойство, есть := s.Properties[ключ]
		if !есть {
			panic(fmt.Errorf("в схеме нет ключа %s", ключ))
		}
		свойство.AnyOf = []*jsonschema.Schema{
			{Type: "string", Description: "Одно имя строкой либо JSON-массив, случайно завёрнутый в строку"},
			{Type: "array", Items: &jsonschema.Schema{Type: "string"}, Description: свойство.Description},
		}
		свойство.Type = ""
		свойство.Types = nil
		свойство.Items = nil
	}
	return s
}
