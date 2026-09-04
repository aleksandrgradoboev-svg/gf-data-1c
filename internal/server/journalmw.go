package server

import (
	"context"
	"encoding/json"
	"strings"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/aleksandrgradoboev-svg/gf-data-1c/internal/journal"
)

// Журнал вызовов инструментов.
//
// Ответы инструментов видит агент, но не мы: чтобы узнать, что у сервера спрашивали и что он
// отдал, до сих пор приходилось читать журнал того харнесса, из которого шли вызовы. Это делало
// любой замер зависимым от чужого формата и, хуже, показывало только ОДНУ установку сервера:
// дефект, при котором справка платформы была недоступна одному клиенту и доступна другому,
// не был виден ни в одном замере именно поэтому.
//
// Пишется вопрос и краткий итог ответа — этого хватает, чтобы пересчитать долю промахов
// по собственной фактуре. Секреты в журнал не попадают: значения полей с паролями и токенами
// заменяются до записи.

// journalLimit — сколько символов итога писать. Ответы бывают в десятки килобайт, а для разбора
// нужна опознавательная строка, а не весь текст.
const journalLimit = 200

// secretKeys — поля, значения которых в журнал не идут ни при каких условиях.
var secretKeys = []string{"password", "пароль", "token", "секрет", "secret", "auth"}

// journalMiddleware пишет в журнал каждый вызов инструмента: имя, аргументы, исход, время.
func journalMiddleware(next mcp.MethodHandler) mcp.MethodHandler {
	return func(ctx context.Context, method string, req mcp.Request) (mcp.Result, error) {
		if method != "tools/call" || !journal.Enabled() {
			return next(ctx, method, req)
		}
		params, ok := req.GetParams().(*mcp.CallToolParamsRaw)
		if !ok {
			return next(ctx, method, req)
		}

		started := time.Now()
		result, err := next(ctx, method, req)
		spent := time.Since(started).Round(time.Millisecond)

		// Время идёт до итога: итог обрезан по длине, и всё, что стоит после него, читатель
		// журнала не увидит.
		journal.Writef("вызов %s %s за %s → %s", params.Name, maskArguments(params.Arguments),
			spent, outcome(result, err))
		return result, err
	}
}

// outcome — чем кончился вызов, одной строкой. Отказ инструмента приходит результатом
// с признаком ошибки, а не ошибкой Go, и по журналу эти два случая надо различать:
// первый — ответ сервера, второй — поломка.
func outcome(result mcp.Result, err error) string {
	if err != nil {
		return "СБОЙ: " + shorten(err.Error())
	}
	call, ok := result.(*mcp.CallToolResult)
	if !ok {
		return "ок"
	}
	head := shorten(firstText(call))
	if call.IsError {
		return "ОТКАЗ: " + head
	}
	if head == "" {
		return "ок"
	}
	return "ок: " + head
}

// firstText достаёт первый текстовый кусок ответа — по нему вызов и опознаётся.
func firstText(call *mcp.CallToolResult) string {
	for _, c := range call.Content {
		if t, ok := c.(*mcp.TextContent); ok && strings.TrimSpace(t.Text) != "" {
			return t.Text
		}
	}
	return ""
}

// maskArguments — аргументы вызова в одну строку, с вычеркнутыми секретами. Разобрать не вышло —
// пишется отметка, а не сырьё: в сыром виде мог бы уехать пароль.
func maskArguments(raw json.RawMessage) string {
	if len(raw) == 0 {
		return "{}"
	}
	var args map[string]any
	if err := json.Unmarshal(raw, &args); err != nil {
		return "{нечитаемые аргументы}"
	}
	for k, v := range args {
		lower := strings.ToLower(k)
		for _, s := range secretKeys {
			if strings.Contains(lower, s) {
				args[k] = "…"
				break
			}
		}
		if s, ok := v.(string); ok && len(s) > journalLimit {
			args[k] = shorten(s)
		}
	}
	out, err := json.Marshal(args)
	if err != nil {
		return "{нечитаемые аргументы}"
	}
	return string(out)
}

// shorten сводит текст к одной строке ограниченной длины: журнал читают глазами.
func shorten(s string) string {
	s = strings.TrimSpace(strings.ReplaceAll(strings.ReplaceAll(s, "\r", " "), "\n", " "))
	s = strings.Join(strings.Fields(s), " ")
	if len([]rune(s)) > journalLimit {
		return string([]rune(s)[:journalLimit]) + "…"
	}
	return s
}
