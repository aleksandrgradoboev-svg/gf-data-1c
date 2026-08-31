// Пакет tools — инструменты, которые сервер предъявляет агенту.
//
// Общий принцип оформления ответов: печатается сводка, а не сырой дамп, и каждая
// цифра сопровождается тем, откуда она взята. Неуспех возвращается отказом (пакет
// refusal), а не пустым результатом.
package tools

import (
	"context"
	"fmt"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/refusal"
	"github.com/greentech/gt-data-1c/internal/registry"
	"github.com/greentech/gt-data-1c/internal/secret"
)

// BasesInput — параметры инструмента управления реестром баз.
type BasesInput struct {
	Action   string `json:"action,omitempty" jsonschema:"Что сделать с реестром: list (умолчание), add, remove"`
	Name     string `json:"name,omitempty" jsonschema:"Короткий ключ базы, которым она называется в параметре base (например buh, trade)"`
	URL      string `json:"url,omitempty" jsonschema:"Адрес HTTP-сервиса базы (нужен для add)"`
	User     string `json:"user,omitempty" jsonschema:"Пользователь 1С для HTTP-сервиса (add)"`
	Password string `json:"password,omitempty" jsonschema:"Пароль пользователя 1С (add). Хранится защищённым средствами Windows, в открытом виде в файл не попадает"`
	Title    string `json:"title,omitempty" jsonschema:"Человекочитаемое название базы для списка (add, необязательно)"`
	Auth     string `json:"auth,omitempty" jsonschema:"Способ аутентификации: basic (умолчание) или ntlm для доменной учётки. Логин вида ДОМЕН\\пользователь опознаётся как доменный сам"`
}

// BasesTool — описание инструмента для агента.
func BasesTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "bases",
		Description: "Список зарегистрированных баз 1С и управление реестром: посмотреть, какие базы " +
			"доступны (action=list), зарегистрировать новую по адресу её HTTP-сервиса (action=add), " +
			"убрать (action=remove). Имя базы из этого списка передаётся остальным инструментам " +
			"параметром base — он обязателен у всех, базы по умолчанию у сервера нет. Начинай " +
			"с action=list, когда не знаешь, какие базы есть.",
	}
}

// Bases обслуживает реестр баз.
func (s *Set) Bases(ctx context.Context, _ *mcp.CallToolRequest, in BasesInput) (*mcp.CallToolResult, any, error) {
	reg, err := s.registry()
	if err != nil {
		return nil, nil, err
	}

	action := strings.ToLower(strings.TrimSpace(in.Action))
	if action == "" {
		action = "list"
	}

	switch action {
	case "list":
		return text(listBases(reg)), nil, nil

	case "add":
		base := registry.Base{
			Name: in.Name, Title: in.Title, URL: in.URL,
			User: in.User, Password: in.Password, Auth: in.Auth,
		}
		if err := reg.Add(base); err != nil {
			return nil, nil, err
		}
		return text(fmt.Sprintf("База %q добавлена в реестр (%s).\n\n%s",
			in.Name, reg.Path(), listBases(reg))), nil, nil

	case "remove":
		if err := reg.Remove(in.Name); err != nil {
			return nil, nil, err
		}
		return text(fmt.Sprintf("База %q убрана из реестра.\n\n%s", in.Name, listBases(reg))), nil, nil

	default:
		hints := []string{"допустимо: list, add, remove"}
		if action == "set_default" {
			// Старый вызов обязан объяснить, что механизм убран, а не выглядеть опечаткой.
			hints = append(hints,
				"базы по умолчанию больше нет: параметр base обязателен у всех инструментов данных")
		}
		return nil, nil, refusal.New(refusal.BadRequest, "действие не распознано",
			fmt.Sprintf("action=%q", in.Action), hints...)
	}
}

// listBases печатает реестр. Пароли не показываются никогда — ни маской, ни длиной.
func listBases(reg *registry.Registry) string {
	if len(reg.Bases) == 0 {
		return "Реестр баз пуст.\nДобавьте базу: action=add, name=<ключ>, url=<адрес HTTP-сервиса>, user, password.\n" +
			"Файл реестра: " + reg.Path()
	}
	var b strings.Builder
	fmt.Fprintf(&b, "Баз в реестре: %d. Файл: %s\n\n", len(reg.Bases), reg.Path())
	for _, base := range reg.Bases {
		fmt.Fprintf(&b, "  %s", base.Name)
		if base.Title != "" {
			fmt.Fprintf(&b, " — %s", base.Title)
		}
		fmt.Fprintf(&b, "\n    адрес: %s\n", base.URL)
		if base.User != "" {
			fmt.Fprintf(&b, "    пользователь: %s", base.User)
			if secret.IsProtected(base.Password) {
				b.WriteString(", пароль защищён")
			} else if base.Password != "" {
				b.WriteString(", пароль ОТКРЫТЫМ ТЕКСТОМ (будет защищён при первом сохранении)")
			}
			b.WriteString("\n")
		}
		if base.Auth != "" {
			fmt.Fprintf(&b, "    аутентификация: %s\n", base.Auth)
		}
	}
	b.WriteString("\nБазы по умолчанию нет: любой инструмент данных требует base явно.")
	return b.String()
}
