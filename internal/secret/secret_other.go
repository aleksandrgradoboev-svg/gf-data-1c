//go:build !windows

// Заглушка для не-Windows: DPAPI там нет.
//
// Молча «шифровать» ничем нельзя — это создало бы видимость защиты. Поэтому значение
// возвращается как есть, а Available() честно отвечает, что защиты нет: вызывающий
// решает сам, предупреждать пользователя или отказываться работать.
package secret

import "strings"

const Prefix = "dpapi:"

func Protect(value string) (string, error) { return value, nil }

func Reveal(value string) (string, error) {
	if IsProtected(value) {
		return "", errUnsupported{}
	}
	return value, nil
}

func IsProtected(value string) bool { return strings.HasPrefix(value, Prefix) }

func Available() bool { return false }

type errUnsupported struct{}

func (errUnsupported) Error() string {
	return "защищённый пароль прочитать нечем: DPAPI есть только в Windows"
}
