package server

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Пароль в журнал не попадает. Проверяется механизмом, а не внимательностью: журнал пишется
// на каждый вызов, а bases с action=add несёт пароль базы открытым текстом.
func TestMaskArgumentsПрячетСекреты(t *testing.T) {
	raw := json.RawMessage(`{"action":"add","name":"проба","user":"agent","password":"СУПЕРСЕКРЕТ","auth":"Basic 123"}`)
	got := maskArguments(raw)
	for _, secret := range []string{"СУПЕРСЕКРЕТ", "Basic 123"} {
		if strings.Contains(got, secret) {
			t.Errorf("секрет утёк в журнал: %s", got)
		}
	}
	for _, keep := range []string{"add", "проба", "agent"} {
		if !strings.Contains(got, keep) {
			t.Errorf("несекретное поле пропало: %q нет в %s", keep, got)
		}
	}
}

// Нечитаемые аргументы не пишутся сырьём: в сыром виде мог бы уехать тот же пароль.
func TestMaskArgumentsНеПишетСырьёПриРазборе(t *testing.T) {
	got := maskArguments(json.RawMessage(`{сломано"password":"СЕКРЕТ"`))
	if strings.Contains(got, "СЕКРЕТ") {
		t.Errorf("секрет утёк через нечитаемые аргументы: %s", got)
	}
}

// Отказ инструмента и поломка сервера — разные события, и по журналу их надо различать.
func TestOutcomeРазличаетОтказИСбой(t *testing.T) {
	refusal := &mcp.CallToolResult{IsError: true,
		Content: []mcp.Content{&mcp.TextContent{Text: "ОТКАЗ: база не названа"}}}
	if got := outcome(refusal, nil); !strings.HasPrefix(got, "ОТКАЗ:") {
		t.Errorf("отказ распознан как %q", got)
	}
	ok := &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: "Документы базы ut11: 268"}}}
	if got := outcome(ok, nil); !strings.HasPrefix(got, "ок:") {
		t.Errorf("успех распознан как %q", got)
	}
}

// Итог обрезается: ответы бывают в десятки килобайт, а журнал читают глазами.
func TestShortenОбрезаетИСклеиваетСтроки(t *testing.T) {
	got := shorten("первая\nвторая   строка\r\n" + strings.Repeat("я", 500))
	if strings.ContainsAny(got, "\n\r") {
		t.Error("перевод строки остался в записи журнала")
	}
	if len([]rune(got)) > journalLimit+1 {
		t.Errorf("длина %d больше предела %d", len([]rune(got)), journalLimit)
	}
}
