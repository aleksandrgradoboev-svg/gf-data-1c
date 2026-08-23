package tools

import (
	"context"
	"errors"
	"fmt"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/channel"
	"github.com/greentech/gt-data-1c/internal/refusal"
)

// ProbeInput — параметры диагностики канала.
type ProbeInput struct {
	Base string `json:"base,omitempty" jsonschema:"Проверить только эту базу. Опущено — проверяются все базы реестра"`
}

// ProbeTool — описание инструмента для агента.
func ProbeTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "probe",
		Description: "Проверить, отвечают ли базы 1С из реестра: по каждой базе отдельная строка — " +
			"жив канал, не установлено расширение (404), не поднят веб-сервер (соединение отвергнуто) " +
			"или отказ прав (401). Вызывай перед работой с данными и всякий раз, когда база отвечает " +
			"пусто: пустой ответ мёртвого канала выглядит точно так же, как честное отсутствие данных. " +
			"Без параметра base проверяет все базы разом.",
	}
}

// Probe опрашивает базы и печатает по строке на каждую.
//
// Инструмент намеренно НЕ возвращает отказ, когда база не ответила: неответ базы —
// это его результат, ради которого его и звали. Отказом заканчивается только
// невозможность выполнить саму проверку (нечего проверять, реестр не читается).
func (s *Set) Probe(ctx context.Context, _ *mcp.CallToolRequest, in ProbeInput) (*mcp.CallToolResult, any, error) {
	reg, err := s.registry()
	if err != nil {
		return nil, nil, err
	}
	if len(reg.Bases) == 0 {
		return nil, nil, refusal.New(refusal.BadRequest, "проверять нечего", "реестр баз пуст",
			"добавьте базу: bases с action=add")
	}

	targets := reg.Bases
	if strings.TrimSpace(in.Base) != "" {
		base, err := reg.Resolve(in.Base)
		if err != nil {
			return nil, nil, err
		}
		targets = targets[:0]
		targets = append(targets, base)
	}

	var b strings.Builder
	fmt.Fprintf(&b, "Проверка канала, баз: %d\n\n", len(targets))
	alive, stale := 0, 0
	for _, base := range targets {
		client := channel.New(base, s.Timeout)
		line, version, ok := probeOne(ctx, client)
		if ok {
			alive++
			// Расширение старше сервера отвечает на новые методы пустотой, а не ошибкой:
			// без этой сверки рассинхрон читается как «в базе ничего нет».
			if version != "" && s.Version != "" && version != s.Version {
				stale++
				line += fmt.Sprintf("\n   ⚠ расширение %s, сервер %s — переустановите расширение "+
					"в этой базе: разошедшиеся версии дают пустые ответы вместо ошибок",
					version, s.Version)
			}
		}
		fmt.Fprintf(&b, "%s\n", line)
	}
	fmt.Fprintf(&b, "\nЖивых каналов: %d из %d.", alive, len(targets))
	if stale > 0 {
		fmt.Fprintf(&b, "\nРазошлись версии расширения и сервера: баз %d. "+
			"Пока версии не сведены, ответам этих баз доверять нельзя.", stale)
	}
	if alive < len(targets) {
		b.WriteString("\nПока канал не жив, работа по данным этой базы не начинается: " +
			"пустой ответ мёртвого канала неотличим от отсутствия данных.")
	}
	return text(b.String()), nil, nil
}

// versionReply — ответ пробы. Разбирается, а не печатается сырьём: сырой JSON
// в строке отчёта читается как поломка, даже когда всё в порядке.
type versionReply struct {
	Расширение string `json:"расширение"`
	Платформа  string `json:"платформа"`
}

// probeOne опрашивает одну базу: строка отчёта, версия расширения, признак живости.
func probeOne(ctx context.Context, client *channel.Client) (string, string, bool) {
	base := client.Base()

	var version versionReply
	err := client.Ask(ctx, "version", nil, &version)
	if err == nil {
		return fmt.Sprintf("✅ %-10s жив — расширение %s, платформа %s",
			base.Name, version.Расширение, version.Платформа), version.Расширение, true
	}

	var ref *refusal.Error
	if errors.As(err, &ref) {
		switch ref.Kind {
		case refusal.NoWebServer:
			return fmt.Sprintf("❌ %-10s веб-сервер не отвечает — %s", base.Name, ref.Why), "", false
		case refusal.NoExtension:
			return fmt.Sprintf("❌ %-10s расширение не установлено — %s", base.Name, ref.Why), "", false
		case refusal.Unauthorized:
			return fmt.Sprintf("❌ %-10s отказ прав — %s", base.Name, ref.Why), "", false
		default:
			return fmt.Sprintf("❌ %-10s %s — %s", base.Name, ref.What, ref.Why), "", false
		}
	}
	return fmt.Sprintf("❌ %-10s не проверена — %v", base.Name, err), "", false
}
