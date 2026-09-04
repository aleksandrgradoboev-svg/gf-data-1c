package tools

import (
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/aleksandrgradoboev-svg/gf-data-1c/internal/channel"
	"github.com/aleksandrgradoboev-svg/gf-data-1c/internal/refusal"
	"github.com/aleksandrgradoboev-svg/gf-data-1c/internal/registry"
)

// Set — набор инструментов, разделяющих общее состояние: путь реестра и таймаут канала.
// Реестр читается на каждый вызов, а не кэшируется: его правят снаружи (в том числе
// инструментом bases), и агент не должен видеть устаревший список.
type Set struct {
	RegistryPath string
	Timeout      time.Duration
	// Version — версия сервера. Нужна пробе: расширение старше сервера отвечает не
	// ошибкой, а пустотой в новых методах, и это неотличимо от отсутствия данных.
	Version string
	// AllowRawQuery отключает гейт построителя (gate.go): query выполняет любой текст, а
	// query_check не запирается после отказа. Только для тестов сервера, которые проверяют
	// сам язык и канал; в поставке гейт включён всегда — его нельзя выключить вызовом.
	AllowRawQuery bool
	// gate — состояние сессии для query_check / query / query_build: см. gate.go.
	gate queryGate
}

func (s *Set) registry() (*registry.Registry, error) {
	return registry.Load(s.RegistryPath)
}

// channelFor открывает канал к названной базе, разрешая имя по реестру.
func (s *Set) channelFor(name string) (*channel.Client, error) {
	reg, err := s.registry()
	if err != nil {
		return nil, err
	}
	base, err := reg.Resolve(name)
	if err != nil {
		return nil, err
	}
	return channel.New(base, s.Timeout), nil
}

// text — обычный текстовый ответ инструмента.
func text(s string) *mcp.CallToolResult {
	return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: s}}}
}

var _ = refusal.Internal
