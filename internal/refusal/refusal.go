// Пакет refusal отвечает за главное правило продукта: отказ инструмента никогда
// не выглядит как пустой результат.
//
// Пустой ответ мёртвого канала неотличим от честного «в базе ничего нет», поэтому
// каждый неуспех оформляется отказом, первая строка которого прямо говорит, что вызов
// не выполнен, и по чьей вине. Отказ характеризует ВЫЗОВ, а не содержимое базы.
package refusal

import (
	"errors"
	"fmt"
	"strings"
)

// Kind — вид отказа. Различаются те виды, которые требуют разных действий человека.
type Kind int

const (
	NoWebServer   Kind = iota // веб-сервер не поднят: соединение отвергнуто
	NoPublication             // базы нет на веб-сервере: 404 страницей самого Apache
	NoExtension               // расширение не установлено в базе: 404 от самой 1С
	Unauthorized              // отказ прав: 401/403
	UnknownBase               // базы нет в реестре
	BadRequest                // вызывающий передал негодные аргументы
	BaseError                 // база ответила ошибкой на осмысленный запрос
	Internal                  // наша собственная поломка
)

// Error — отказ с причиной и подсказкой, что делать.
//
// Base — имя базы, к которой относится отказ. Без него «в этой конфигурации такого нет»
// читается как факт о 1С вообще, и вызывающий начинает перебирать имена объекта вместо
// того, чтобы усомниться в базе. Ровно этот сценарий и наблюдался 26.08.2026.
type Error struct {
	Kind  Kind
	Base  string   // база, к которой относится отказ (пусто — отказ не про конкретную базу)
	What  string   // что не удалось сделать
	Why   string   // чем именно ответила та сторона
	Hints []string // что предпринять вызывающему
}

func (e *Error) Error() string {
	var b strings.Builder
	b.WriteString("ОТКАЗ")
	if e.Base != "" {
		// База называется в самой первой строке, а не сноской внизу: отказ, прочитанный
		// до конца, — не то же самое, что отказ, прочитанный до первой точки.
		b.WriteString(" (база ")
		b.WriteString(e.Base)
		b.WriteString(")")
	}
	b.WriteString(": ")
	b.WriteString(e.What)
	if e.Why != "" {
		b.WriteString(" — ")
		b.WriteString(e.Why)
	}
	b.WriteString(".\n")
	b.WriteString("Это отказ вызова, а не ответ базы: считать его отсутствием данных нельзя.")
	for _, h := range e.Hints {
		b.WriteString("\n• ")
		b.WriteString(h)
	}
	return b.String()
}

func New(kind Kind, what, why string, hints ...string) *Error {
	return &Error{Kind: kind, What: what, Why: why, Hints: hints}
}

// UnknownBaseError — отдельный конструктор: незнакомое имя базы обязано отвечать
// перечнем известных, иначе вызывающий решит, что база пуста.
func UnknownBaseError(name string, known []string) *Error {
	why := fmt.Sprintf("в реестре её нет; известны: %s", strings.Join(known, ", "))
	if len(known) == 0 {
		why = "реестр баз пуст"
	}
	return New(UnknownBase, fmt.Sprintf("база %q не найдена", name), why,
		"перечень баз — инструмент bases с action=list",
		"добавить базу — bases с action=add, url и учётными данными")
}

// Stamp проставляет базу отказу, если она ещё не названа, и возвращает его же.
// Чужие ошибки и отказы, уже знающие свою базу, проходят насквозь.
func Stamp(err error, base string) error {
	var e *Error
	if errors.As(err, &e) && e.Base == "" {
		e.Base = base
	}
	return err
}
